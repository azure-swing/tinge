//! File colorimetry and deterministic output ICC definitions.
use anyhow::{Result, bail, ensure};
use std::{borrow::Cow, fs::File, io::BufReader, path::Path};
use vibecolor_color::{ColorSpace, Primaries, Transfer};

pub fn rgb_profile(space: ColorSpace) -> Result<moxcms::ColorProfile> {
    let mut profile = match space.primaries {
        Primaries::Srgb => moxcms::ColorProfile::new_srgb(),
        Primaries::DisplayP3 => moxcms::ColorProfile::new_display_p3(),
        Primaries::Rec2020 => moxcms::ColorProfile::new_bt2020(),
        Primaries::AcesCg => moxcms::ColorProfile::new_aces_cg_linear(),
    };
    let curve = match space.transfer {
        Transfer::Srgb => moxcms::ColorProfile::new_srgb().red_trc.unwrap(),
        Transfer::Linear => moxcms::curve_from_gamma(1.0),
        Transfer::Gamma22 => moxcms::curve_from_gamma(2.2),
        Transfer::Gamma24 => moxcms::curve_from_gamma(2.4),
        _ => bail!("HDR files require PNG cICP; an SDR ICC fallback would be misleading"),
    };
    profile.red_trc = Some(curve.clone());
    profile.green_trc = Some(curve.clone());
    profile.blue_trc = Some(curve);
    profile.cicp = None;
    // Built-in CMS profiles use a temperature approximation for D65. Use
    // standard xy whites and a real source-to-PCS Bradford adaptation instead.
    let c = crate::exr_io::chromaticities(space.primaries);
    let white = moxcms::XyY {
        x: c.white.x() as f64,
        y: c.white.y() as f64,
        yb: 1.0,
    };
    let xyz = |v: exr::math::Vec2<f32>| moxcms::Chromaticity { x: v.x(), y: v.y() }.to_xyzd();
    let [r, g, b] = [c.red, c.green, c.blue].map(xyz);
    let native = moxcms::ColorProfile::rgb_to_xyz_d(
        moxcms::Matrix3d {
            v: [[r.x, g.x, b.x], [r.y, g.y, b.y], [r.z, g.z, b.z]],
        },
        white.to_xyzd(),
    );
    let pcs = moxcms::Xyz {
        x: 0.9642,
        y: 1.0,
        z: 0.8249,
    };
    let adaptation = moxcms::adaption_matrix_d(white.to_xyz(), pcs);
    let matrix = adaptation * native;
    let column = |i| moxcms::Xyzd {
        x: matrix.v[0][i],
        y: matrix.v[1][i],
        z: matrix.v[2][i],
    };
    profile.red_colorant = column(0);
    profile.green_colorant = column(1);
    profile.blue_colorant = column(2);
    profile.white_point = pcs.to_xyzd();
    profile.media_white_point = Some(pcs.to_xyzd());
    profile.chromatic_adaptation = Some(adaptation);
    Ok(profile)
}

pub fn output_profile(space: ColorSpace) -> Result<Vec<u8>> {
    let profile = rgb_profile(space)?;
    let mut bytes = profile.encode()?;
    ensure!(bytes.len() >= 128, "generated ICC header is truncated");
    for (i, value) in [2026u16, 10, 7, 0, 0, 0].iter().enumerate() {
        bytes[24 + i * 2..26 + i * 2].copy_from_slice(&value.to_be_bytes());
    }
    Ok(bytes)
}

pub fn png_info(path: &Path) -> Result<Option<png::Info<'static>>> {
    let mut signature = [0; 8];
    use std::io::Read;
    let mut file = File::open(path)?;
    if file.read_exact(&mut signature).is_err() || signature != *b"\x89PNG\r\n\x1a\n" {
        return Ok(None);
    }
    let reader = png::Decoder::new(BufReader::new(File::open(path)?)).read_info()?;
    Ok(Some(reader.info().clone()))
}
pub fn tiff_profile(path: &Path) -> Result<Option<Vec<u8>>> {
    // image's TIFF decoder ties its decoding budget to the pixel byte count;
    // reading a profile larger than a tiny image then fails and is discarded.
    // Read tags independently with a bounded metadata budget, preserving errors.
    let mut limits = tiff::decoder::Limits::default();
    limits.ifd_value_size = 32 * 1024 * 1024;
    let mut decoder =
        tiff::decoder::Decoder::new(BufReader::new(File::open(path)?))?.with_limits(limits);
    Ok(decoder.find_tag_unsigned_vec::<u8>(tiff::tags::Tag::IccProfile)?)
}

pub fn cicp_space(c: png::CodingIndependentCodePoints) -> Result<ColorSpace> {
    ensure!(
        c.matrix_coefficients == 0 && c.is_video_full_range_image,
        "PNG cICP currently requires full-range RGB (matrix 0); declare input_space to override unsupported signaling"
    );
    let primaries = match c.color_primaries {
        1 => Primaries::Srgb,
        9 => Primaries::Rec2020,
        12 => Primaries::DisplayP3,
        code => bail!("unsupported PNG cICP primaries {code}; declare input_space"),
    };
    let transfer = match c.transfer_function {
        8 => Transfer::Linear,
        13 => Transfer::Srgb,
        16 => Transfer::Pq,
        code => bail!(
            "unsupported PNG cICP transfer {code}; HLG display OOTF requires explicit configuration"
        ),
    };
    Ok(ColorSpace {
        primaries,
        transfer,
    })
}

pub fn write_png(
    writer: impl std::io::Write,
    w: u32,
    h: u32,
    bits: u8,
    space: ColorSpace,
    raw: &[f32],
) -> Result<()> {
    let mut info = png::Info::with_size(w, h);
    let alpha_max = if bits == 8 { 255.0 } else { 65535.0 };
    // Blending can leave 1-epsilon alpha. Drop the channel only when every
    // actual encoded sample is fully opaque, preserving decoded pixels exactly.
    let opaque = raw
        .chunks_exact(4)
        .all(|p| (p[3].clamp(0.0, 1.0) * alpha_max).round() == alpha_max);
    info.color_type = if opaque {
        png::ColorType::Rgb
    } else {
        png::ColorType::Rgba
    };
    info.bit_depth = if bits == 8 {
        png::BitDepth::Eight
    } else {
        png::BitDepth::Sixteen
    };
    if space.transfer == Transfer::Pq {
        ensure!(
            space.primaries == Primaries::Rec2020,
            "PQ PNG currently requires Rec.2020 primaries"
        );
        info.coding_independent_code_points = Some(png::CodingIndependentCodePoints {
            color_primaries: 9,
            transfer_function: 16,
            matrix_coefficients: 0,
            is_video_full_range_image: true,
        });
    } else {
        info.icc_profile = Some(Cow::Owned(output_profile(space)?));
    }
    let mut writer = png::Encoder::with_info(writer, info)?.write_header()?;
    // png 0.18 parses cICP but does not emit Info.coding_independent_code_points.
    // Write the standard ancillary chunk before IDAT using the encoder's CRC path.
    if space.transfer == Transfer::Pq {
        writer.write_chunk(png::chunk::cICP, &[9, 16, 0, 1])?;
    }
    let data: Vec<u8> = if bits == 8 {
        raw.chunks_exact(4)
            .flat_map(|p| &p[..if opaque { 3 } else { 4 }])
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    } else {
        raw.chunks_exact(4)
            .flat_map(|p| &p[..if opaque { 3 } else { 4 }])
            .flat_map(|v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes())
            .collect()
    };
    writer.write_image_data(&data)?;
    writer.finish()?;
    Ok(())
}
