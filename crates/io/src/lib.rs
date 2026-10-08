mod color_metadata;
mod exr_io;
mod raw_options;
pub use raw_options::{
    BelowBlack, RawCrop, RawDevelopOptions, RawWhiteBalance, SensorLevels, temperature_white_xy,
};
#[cfg(feature = "raw")]
pub mod raw;
use anyhow::{Context, Result, bail, ensure};
use image::{DynamicImage, ImageBuffer, ImageDecoder, ImageEncoder, ImageReader, Rgb, Rgba};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};
use vibecolor_color::{ColorSpace, Primaries, Transfer, convert_linear, decode, encode};
use vibecolor_core::Frame;

#[derive(Debug, Serialize)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub color_type: String,
    pub format: String,
    pub has_icc: bool,
    pub orientation: String,
    pub hash: String,
    pub raw_metadata: Option<RawMetadata>,
    pub color_metadata: FileColorMetadata,
}
#[derive(Debug, Default, Serialize)]
pub struct FileColorMetadata {
    pub color_interop_id: Option<String>,
    pub chromaticities: Option<[[f32; 2]; 4]>,
    pub cicp: Option<[u8; 4]>,
    pub icc_hash: Option<String>,
    pub alpha_mode: String,
    pub white_luminance_nits: Option<f32>,
}
#[derive(Debug, Serialize)]
pub struct RawMetadata {
    pub make: String,
    pub model: String,
    pub sensor_width: usize,
    pub sensor_height: usize,
    pub bits: usize,
    pub components: usize,
    pub wb_coefficients: Vec<Option<f32>>,
    pub decoder: &'static str,
    pub demosaic: &'static str,
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
pub fn inspect(path: &Path) -> Result<ImageInfo> {
    #[cfg(feature = "raw")]
    if raw::is_raw(path) {
        return raw::inspect(path).map(|(mut info, metadata)| {
            info.raw_metadata = Some(metadata);
            info
        });
    }
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = format!("{:?}", reader.format());
    let mut d = reader.into_decoder()?;
    let (width, height) = d.dimensions();
    let color_type = format!("{:?}", d.color_type());
    let icc = if format.contains("Tiff") {
        color_metadata::tiff_profile(path)?
    } else {
        d.icc_profile()?
    };
    let has_icc = icc.is_some();
    let mut color_metadata = FileColorMetadata {
        icc_hash: icc.as_ref().map(|v| blake3::hash(v).to_hex().to_string()),
        alpha_mode: "straight".into(),
        ..Default::default()
    };
    if let Some(info) = color_metadata::png_info(path)? {
        color_metadata.cicp = info.coding_independent_code_points.map(|c| {
            [
                c.color_primaries,
                c.transfer_function,
                c.matrix_coefficients,
                u8::from(c.is_video_full_range_image),
            ]
        });
    }
    if reader_format_is_exr(&format) {
        let meta = exr_io::metadata(path)?;
        if let Some(h) = meta.headers.iter().find(|h| {
            ["R", "G", "B"]
                .iter()
                .all(|name| h.channels.list.iter().any(|c| c.name.to_string() == *name))
        }) {
            color_metadata.color_interop_id = exr_io::id(&h.shared_attributes, &h.own_attributes);
            color_metadata.chromaticities = h
                .shared_attributes
                .chromaticities
                .map(|c| [c.red, c.green, c.blue, c.white].map(|v| [v.x(), v.y()]));
            color_metadata.alpha_mode = if exr_io::is_straight(&h.own_attributes) {
                "straight (explicit VibeColor attribute)"
            } else {
                "premultiplied (EXR convention)"
            }
            .into();
            color_metadata.white_luminance_nits = h.own_attributes.white_luminance;
        }
    }
    let orientation = format!("{:?}", d.orientation()?);
    Ok(ImageInfo {
        width,
        height,
        color_type,
        format,
        has_icc,
        orientation,
        hash: hash_file(path)?,
        raw_metadata: None,
        color_metadata,
    })
}
fn reader_format_is_exr(format: &str) -> bool {
    format.contains("OpenExr")
}
fn linear_profile() -> moxcms::ColorProfile {
    color_metadata::rgb_profile(ColorSpace {
        primaries: Primaries::Srgb,
        transfer: Transfer::Linear,
    })
    .expect("linear sRGB is a supported ICC definition")
}
fn png_colorimetry(path: &Path, explicit: bool, icc_managed: bool) -> Result<Option<ColorSpace>> {
    let Some(info) = color_metadata::png_info(path)? else {
        return Ok(None);
    };
    if explicit {
        return Ok(None);
    }
    if let Some(cicp) = info.coding_independent_code_points {
        return Ok(Some(color_metadata::cicp_space(cicp)?));
    }
    ensure!(
        (explicit || icc_managed)
            || info.srgb.is_some()
            || (info.source_chromaticities.is_none() && info.source_gamma.is_none()),
        "PNG gamma/chromaticities require explicit input-space or an ICC profile"
    );
    Ok(None)
}
/// ICC conversion uses a real Rust CMS. Explicit colorimetry overrides embedded metadata.
pub fn load(path: &Path, space: Option<ColorSpace>) -> Result<Frame> {
    #[cfg(feature = "raw")]
    if raw::is_raw(path) {
        ensure!(
            space.is_none(),
            "input-space overrides apply to rendered images; RAW uses camera calibration"
        );
        return raw::load(path);
    }
    let reader = ImageReader::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .with_guessed_format()?;
    if reader.format() == Some(image::ImageFormat::OpenExr) {
        return exr_io::load(path, space);
    }
    let explicit = space.is_some();
    let format = reader.format();
    let mut d = reader
        .into_decoder()
        .context("unsupported image format; camera RAW is not implemented yet")?;
    let (w, h) = d.dimensions();
    ensure!(
        w as u64 * h as u64 <= 100_000_000,
        "image exceeds 100M pixel limit"
    );
    let icc = if format == Some(image::ImageFormat::Tiff) {
        color_metadata::tiff_profile(path)?
    } else {
        d.icc_profile()?
    };
    let color = d.color_type();
    // The image crate converts CMYK during decoding, so it cannot apply original CMYK ICC here.
    let tagged_space = png_colorimetry(path, explicit, icc.is_some())?;
    if let Some(profile) = icc.as_ref().filter(|_| !explicit && tagged_space.is_none()) {
        ensure!(
            moxcms::ColorProfile::new_from_slice(profile)?.color_space
                == moxcms::DataColorSpace::Rgb,
            "only RGB ICC input profiles are supported; use an explicit input-space for decoded gray/RGB"
        );
    }
    let orientation = d.orientation()?;
    let mut img = DynamicImage::from_decoder(d)?;
    img.apply_orientation(orientation);
    let space = space.or(tagged_space).unwrap_or(ColorSpace {
        primaries: Primaries::Srgb,
        transfer: if matches!(color, image::ColorType::Rgb32F | image::ColorType::Rgba32F) {
            Transfer::Linear
        } else {
            Transfer::Srgb
        },
    });
    let rgba = img.to_rgba32f();
    let (w, h) = rgba.dimensions();
    if let Some(profile) = icc.filter(|_| !explicit && tagged_space.is_none()) {
        let source = moxcms::ColorProfile::new_from_slice(&profile)?;
        let transform = source.create_transform_f32(
            moxcms::Layout::Rgba,
            &linear_profile(),
            moxcms::Layout::Rgba,
            moxcms::TransformOptions {
                allow_extended_range_rgb_xyz: true,
                prefer_fixed_point: false,
                allow_use_cicp_transfer: false,
                ..Default::default()
            },
        )?;
        let raw = rgba.as_raw();
        let mut out = vec![0.0; raw.len()];
        transform.transform(raw, &mut out)?;
        return Frame::new(
            w,
            h,
            out.chunks_exact(4)
                .map(|p| [p[0], p[1], p[2], p[3]])
                .collect(),
        );
    }
    let pixels = rgba
        .pixels()
        .map(|p| {
            let rgb = convert_linear(
                [
                    decode(p[0], space.transfer),
                    decode(p[1], space.transfer),
                    decode(p[2], space.transfer),
                ],
                space.primaries,
                Primaries::Srgb,
            );
            [rgb[0], rgb[1], rgb[2], p[3]]
        })
        .collect();
    Frame::new(w, h, pixels)
}

pub fn load_matte(path: &Path, w: u32, h: u32) -> Result<Vec<f32>> {
    let image = ImageReader::open(path)?.with_guessed_format()?.decode()?;
    ensure!(
        image.width() == w && image.height() == h,
        "bitmap matte dimensions {}x{} must match node input {w}x{h}",
        image.width(),
        image.height()
    );
    // Gray intensity, not alpha: matte files must be grayscale or grayscale RGB.
    Ok(image
        .to_luma32f()
        .pixels()
        .map(|p| p[0].clamp(0.0, 1.0))
        .collect())
}
/// Standalone matte data: grayscale PNG, no ICC/gamma conversion, exact alpha samples.
pub fn export_matte(frame: &Frame, path: &Path, bits: u8, overwrite: bool) -> Result<()> {
    ensure!(
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
        "matte output must be a grayscale PNG"
    );
    ensure!([8, 16].contains(&bits), "matte bit depth must be 8 or 16");
    let mut bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    if bits == 8 {
        let raw: Vec<u8> = frame
            .pixels
            .iter()
            .map(|p| (p[3] * 255.0).round() as u8)
            .collect();
        encoder.write_image(
            &raw,
            frame.width,
            frame.height,
            image::ExtendedColorType::L8,
        )?;
    } else {
        let raw: Vec<u8> = frame
            .pixels
            .iter()
            .flat_map(|p| ((p[3] * 65535.0).round() as u16).to_ne_bytes())
            .collect();
        encoder.write_image(
            &raw,
            frame.width,
            frame.height,
            image::ExtendedColorType::L16,
        )?;
    }
    atomic_bytes(path, &bytes, overwrite)
}
pub fn atomic_bytes(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    if overwrite {
        tmp.persist(path).map_err(|e| e.error)?;
    } else {
        tmp.persist_noclobber(path).map_err(|e| e.error)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct ExportOptions {
    pub bit_depth: u8,
    pub space: ColorSpace,
    pub overwrite: bool,
    pub quality: u8,
    /// Manual PQ encoding: luminance of linear RGB 1.0. OCIO owns this mapping itself.
    pub linear_unit_nits: Option<f32>,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            bit_depth: 16,
            space: ColorSpace::default(),
            overwrite: false,
            quality: 95,
            linear_unit_nits: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_space: ColorSpace,
    pub clipped_channels: u64,
    pub warnings: Vec<String>,
    pub linear_unit_nits: Option<f32>,
    pub color_metadata: FileColorMetadata,
}

/// RGB signal values whose encoding is owned by an external color processor.
/// Unlike Frame, this type makes no scene-linear working-space promise.
#[derive(Debug, Clone)]
pub struct SignalImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

impl SignalImage {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.width > 0
                && self.height > 0
                && self.width as u64 * self.height as u64 == self.pixels.len() as u64,
            "signal image dimensions do not match pixels"
        );
        ensure!(
            self.pixels.iter().flatten().all(|v| v.is_finite()),
            "signal pixels must be finite"
        );
        ensure!(
            self.pixels.iter().all(|p| (0.0..=1.0).contains(&p[3])),
            "signal alpha must be 0..1"
        );
        Ok(())
    }
}

/// Decode stored RGB samples without ICC, transfer, or primary conversion.
/// The caller MUST explicitly supply their encoding to a color processor.
pub fn load_signal(path: &Path) -> Result<SignalImage> {
    #[cfg(feature = "raw")]
    ensure!(
        !raw::is_raw(path),
        "RAW sensor samples require camera development; use managed input for RAW"
    );
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    if reader.format() == Some(image::ImageFormat::OpenExr) {
        return exr_io::signal(path);
    }
    let decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    ensure!(
        w as u64 * h as u64 <= 100_000_000,
        "image exceeds 100M pixel limit"
    );
    let mut decoder = decoder;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let rgba = image.to_rgba32f();
    let result = SignalImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.pixels().map(|p| p.0).collect(),
    };
    result.validate()?;
    Ok(result)
}

pub fn export(frame: &Frame, path: &Path, opt: ExportOptions) -> Result<ExportReport> {
    write_image(
        frame.width,
        frame.height,
        &frame.pixels,
        path,
        opt,
        true,
        None,
    )
}

/// Write already encoded output with the declared output profile. No gamma is applied.
/// Output colorimetry must match the processor's encoded values.
pub fn export_signal(frame: &SignalImage, path: &Path, opt: ExportOptions) -> Result<ExportReport> {
    frame.validate()?;
    write_image(
        frame.width,
        frame.height,
        &frame.pixels,
        path,
        opt,
        false,
        None,
    )
}

/// Encode a tagged preview directly in memory through the same pixel/metadata path.
pub fn preview_png_bytes(
    w: u32,
    h: u32,
    pixels: &[[f32; 4]],
    scene_linear: bool,
) -> Result<(Vec<u8>, ExportReport)> {
    let mut bytes = Vec::new();
    let report = write_image(
        w,
        h,
        pixels,
        Path::new("preview.png"),
        ExportOptions {
            bit_depth: 8,
            ..Default::default()
        },
        scene_linear,
        Some(&mut bytes),
    )?;
    Ok((bytes, report))
}

fn write_image(
    w: u32,
    h: u32,
    pixels: &[[f32; 4]],
    path: &Path,
    opt: ExportOptions,
    scene_linear: bool,
    memory: Option<&mut Vec<u8>>,
) -> Result<ExportReport> {
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        [8, 16, 32].contains(&opt.bit_depth),
        "bit depth must be 8, 16, or 32"
    );
    ensure!(
        opt.quality >= 1 && opt.quality <= 100,
        "JPEG quality must be 1..100"
    );
    match ext.as_str() {
        "png" => ensure!(opt.bit_depth != 32, "PNG supports 8/16 bits"),
        "jpg" | "jpeg" => ensure!(opt.bit_depth == 8, "JPEG requires 8 bits"),
        "exr" => ensure!(
            opt.bit_depth == 32 && opt.space.transfer == Transfer::Linear,
            "EXR requires linear 32-bit output"
        ),
        "tif" | "tiff" => {}
        _ => bail!("export format must be PNG, JPEG, TIFF, or EXR"),
    };
    if opt.space.transfer == Transfer::Pq {
        ensure!(
            ext == "png" && opt.bit_depth == 16 && opt.space.primaries == Primaries::Rec2020,
            "PQ output requires 16-bit Rec.2020 PNG with cICP"
        );
    }
    ensure!(
        opt.space.transfer != Transfer::Hlg,
        "HLG output requires a display OOTF, not just the scene OETF"
    );
    if opt.space.transfer == Transfer::Pq && scene_linear {
        ensure!(
            opt.linear_unit_nits
                .is_some_and(|v| v.is_finite() && v > 0.0),
            "manual PQ encoding requires positive linear_unit_nits; use an OCIO HDR display chain for scene rendering"
        );
    } else {
        ensure!(
            opt.linear_unit_nits.is_none(),
            "linear_unit_nits applies only to manual scene RGB to PQ encoding"
        );
    }
    if opt.bit_depth == 32 {
        ensure!(
            opt.space.transfer == Transfer::Linear,
            "float output requires linear transfer"
        );
    }
    let mut clipped = 0u64;
    let mut raw = Vec::with_capacity(pixels.len() * 4);
    for p in pixels {
        let rgb = if scene_linear {
            convert_linear([p[0], p[1], p[2]], Primaries::Srgb, opt.space.primaries).map(|v| {
                encode(
                    if opt.space.transfer == Transfer::Pq {
                        v * opt.linear_unit_nits.unwrap() / 10000.0
                    } else {
                        v
                    },
                    opt.space.transfer,
                )
            })
        } else {
            [p[0], p[1], p[2]]
        };
        for v in rgb {
            if opt.bit_depth != 32 && !(0.0..=1.0).contains(&v) {
                clipped += 1;
            }
            raw.push(v);
        }
        raw.push(p[3]);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = if memory.is_none() {
        fs::create_dir_all(parent)?;
        Some(tempfile::NamedTempFile::new_in(parent)?)
    } else {
        None
    };
    let format = match ext.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "exr" => image::ImageFormat::OpenExr,
        _ => image::ImageFormat::Tiff,
    };
    let mut warnings = vec![
        "Original EXIF/XMP/IPTC are not copied; output uses newly generated color metadata.".into(),
    ];
    let color_metadata = if format == image::ImageFormat::OpenExr {
        let c = exr_io::chromaticities(opt.space.primaries);
        FileColorMetadata {
            color_interop_id: Some(exr_io::interop_id(opt.space.primaries).into()),
            chromaticities: Some([c.red, c.green, c.blue, c.white].map(|v| [v.x(), v.y()])),
            alpha_mode: "premultiplied".into(),
            ..Default::default()
        }
    } else if opt.space.transfer == Transfer::Pq {
        FileColorMetadata {
            cicp: Some([9, 16, 0, 1]),
            alpha_mode: "straight".into(),
            ..Default::default()
        }
    } else {
        FileColorMetadata {
            icc_hash: Some(
                blake3::hash(&color_metadata::output_profile(opt.space)?)
                    .to_hex()
                    .to_string(),
            ),
            alpha_mode: "straight".into(),
            ..Default::default()
        }
    };
    if format == image::ImageFormat::OpenExr {
        exr_io::write(tmp.as_mut().unwrap().as_file_mut(), w, h, &raw, opt.space)?;
        let hidden = pixels
            .iter()
            .filter(|p| p[3] == 0.0 && p[..3].iter().any(|v| *v != 0.0))
            .count();
        if hidden > 0 {
            warnings.push(format!(
                "{hidden} transparent pixels had hidden RGB discarded during EXR premultiplication."
            ));
        }
    } else if format == image::ImageFormat::Png {
        if let Some(memory) = memory {
            color_metadata::write_png(memory, w, h, opt.bit_depth, opt.space, &raw)?;
        } else {
            color_metadata::write_png(tmp.as_mut().unwrap(), w, h, opt.bit_depth, opt.space, &raw)?;
        }
    } else if format == image::ImageFormat::Jpeg {
        ensure!(
            pixels.iter().all(|p| p[3] >= 1.0 - 1e-6),
            "JPEG cannot represent transparency; choose PNG or TIFF"
        );
        let rgb: Vec<u8> = raw
            .chunks_exact(4)
            .flat_map(|p| {
                p[..3]
                    .iter()
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            })
            .collect();
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(tmp.as_mut().unwrap(), opt.quality);
        encoder.set_icc_profile(color_metadata::output_profile(opt.space)?)?;
        encoder.write_image(&rgb, w, h, image::ExtendedColorType::Rgb8)?;
    } else {
        let img = match opt.bit_depth {
            8 => DynamicImage::ImageRgba8(
                ImageBuffer::<Rgba<u8>, _>::from_raw(
                    w,
                    h,
                    raw.iter()
                        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                        .collect(),
                )
                .context("buffer size")?,
            ),
            16 => DynamicImage::ImageRgba16(
                ImageBuffer::<Rgba<u16>, _>::from_raw(
                    w,
                    h,
                    raw.iter()
                        .map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16)
                        .collect(),
                )
                .context("buffer size")?,
            ),
            _ => {
                if format == image::ImageFormat::Tiff {
                    ensure!(
                        pixels.iter().all(|p| p[3] >= 1.0 - 1e-6),
                        "32-bit TIFF currently supports RGB only; use EXR for float alpha"
                    );
                    let rgb = raw
                        .chunks_exact(4)
                        .flat_map(|p| p[..3].iter().copied())
                        .collect();
                    DynamicImage::ImageRgb32F(
                        ImageBuffer::<Rgb<f32>, _>::from_raw(w, h, rgb).context("buffer size")?,
                    )
                } else {
                    DynamicImage::ImageRgba32F(
                        ImageBuffer::<Rgba<f32>, _>::from_raw(w, h, raw).context("buffer size")?,
                    )
                }
            }
        };
        match format {
            image::ImageFormat::Tiff => {
                let mut enc =
                    image::codecs::tiff::TiffEncoder::new(tmp.as_mut().unwrap().as_file_mut());
                enc.set_icc_profile(color_metadata::output_profile(opt.space)?)?;
                img.write_with_encoder(enc)?;
            }
            _ => img.write_to(tmp.as_mut().unwrap().as_file_mut(), format)?,
        }
    }
    if let Some(tmp) = tmp {
        tmp.as_file().sync_all()?;
        if opt.overwrite {
            tmp.persist(path).map_err(|e| e.error)?;
        } else {
            tmp.persist_noclobber(path).map_err(|e| e.error)?;
        }
    }
    if clipped > 0 {
        warnings.push(format!(
            "{clipped} RGB channel values clipped at integer output; inspect highlights/gamut or use scene-linear float output."
        ));
    }
    Ok(ExportReport {
        path: path.display().to_string(),
        width: w,
        height: h,
        bit_depth: opt.bit_depth,
        color_space: opt.space,
        clipped_channels: clipped,
        warnings,
        linear_unit_nits: opt.linear_unit_nits,
        color_metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_png_preserves_opaque_and_transparent_samples_and_memory_encoding() {
        let dir = tempfile::tempdir().unwrap();
        for alpha in [1.0, 1.0 - f32::EPSILON, 0.99999, 0.25] {
            let frame = Frame::new(7, 5, vec![[0.18, 0.4, 0.8, alpha]; 35]).unwrap();
            for bits in [8, 16] {
                let path = dir.path().join(format!("{alpha}-{bits}.png"));
                export(
                    &frame,
                    &path,
                    ExportOptions {
                        bit_depth: bits,
                        ..Default::default()
                    },
                )
                .unwrap();
                let image = image::open(&path).unwrap();
                let max = if bits == 8 { 255.0 } else { 65535.0 };
                assert_eq!(image.color().has_alpha(), (alpha * max).round() != max);
                let decoded = image.to_rgba16();
                assert_eq!(
                    decoded.get_pixel(0, 0).0[3],
                    if bits == 8 {
                        ((alpha * 255.0).round() as u16) * 257
                    } else {
                        (alpha * 65535.0).round() as u16
                    }
                );
                if bits == 8 {
                    let (memory, _) =
                        preview_png_bytes(frame.width, frame.height, &frame.pixels, true).unwrap();
                    assert_eq!(memory, fs::read(&path).unwrap());
                }
            }
        }
    }
    #[test]
    fn cicp_takes_precedence_over_icc_and_explicit_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hdr-labelled.png");
        let f = Frame::new(1, 1, vec![[0.18, 0.18, 0.18, 1.0]]).unwrap();
        export(&f, &p, ExportOptions::default()).unwrap();
        let original = fs::read(&p).unwrap();
        let mut chunk = vec![0, 0, 0, 4];
        let body = [b'c', b'I', b'C', b'P', 9, 16, 0, 1];
        chunk.extend(body);
        chunk.extend(crc32fast::hash(&body).to_be_bytes());
        let data = [original[..33].to_vec(), chunk, original[33..].to_vec()].concat();
        fs::write(&p, data).unwrap();
        let signal = load_signal(&p).unwrap();
        let expected = convert_linear(
            signal.pixels[0][..3].try_into().unwrap(),
            Primaries::Rec2020,
            Primaries::Srgb,
        );
        // The neutral test pixel has equal channels; PQ must override its sRGB ICC.
        let automatic = load(&p, None).unwrap();
        assert!((automatic.pixels[0][0] - decode(expected[0], Transfer::Pq)).abs() < 1e-5);
        assert!((automatic.pixels[0][0] - 0.18).abs() > 0.1);
        let override_rgb = load(&p, Some(ColorSpace::default())).unwrap();
        assert!((override_rgb.pixels[0][0] - 0.18).abs() < 3e-5);
    }
    #[test]
    fn png16_roundtrip_no_overwrite_and_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        let f = Frame::new(1, 1, vec![[0.12345, 0.4, 0.8, 0.25]]).unwrap();
        export(&f, &p, ExportOptions::default()).unwrap();
        assert!(export(&f, &p, ExportOptions::default()).is_err());
        let declared = load(&p, Some(ColorSpace::default())).unwrap();
        assert!((0..4).all(|c| (declared.pixels[0][c] - f.pixels[0][c]).abs() < 3e-5));
        // ICC colorants use s15Fixed16; the matrix encoding adds a small quantization error.
        let out = load(&p, None).unwrap();
        assert!(
            (0..4).all(|c| (out.pixels[0][c] - f.pixels[0][c]).abs() < 6e-5),
            "{:?} versus {:?}",
            out.pixels,
            f.pixels
        );
        assert!(inspect(&p).unwrap().has_icc);
    }
    #[test]
    fn display_p3_icc_matches_reference_matrix() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p3.png");
        let enc = [0.5f32, 0.25, 0.75];
        let data = enc.map(|v| (v * 65535.0).round() as u16);
        let buf = ImageBuffer::<Rgb<u16>, _>::from_raw(1, 1, data.to_vec()).unwrap();
        let mut encoder = image::codecs::png::PngEncoder::new(File::create(&p).unwrap());
        encoder
            .set_icc_profile(moxcms::ColorProfile::new_display_p3().encode().unwrap())
            .unwrap();
        DynamicImage::ImageRgb16(buf)
            .write_with_encoder(encoder)
            .unwrap();
        let actual = load(&p, None).unwrap();
        let expected = convert_linear(
            enc.map(|v| decode(v, Transfer::Srgb)),
            Primaries::DisplayP3,
            Primaries::Srgb,
        );
        assert!(
            (0..3).all(|c| (actual.pixels[0][c] - expected[c]).abs() < 0.0005),
            "{:?} vs {expected:?}",
            actual.pixels
        );
    }
    #[test]
    fn float_tiff_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("float.tiff");
        let f = Frame::new(1, 1, vec![[0.18, 2.0, 0.6, 1.0]]).unwrap();
        export(
            &f,
            &p,
            ExportOptions {
                bit_depth: 32,
                space: ColorSpace {
                    primaries: Primaries::Srgb,
                    transfer: Transfer::Linear,
                },
                ..Default::default()
            },
        )
        .unwrap();
        let actual = load(&p, None).unwrap();
        assert!(
            (0..3).all(|c| (actual.pixels[0][c] - f.pixels[0][c]).abs() < 0.0002),
            "{:?}",
            actual.pixels
        );
    }
    #[test]
    fn exr_retains_hdr_and_negatives() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.exr");
        let f = Frame::new(1, 1, vec![[-0.2, 4.0, 0.5, 0.3]]).unwrap();
        export(
            &f,
            &p,
            ExportOptions {
                bit_depth: 32,
                space: ColorSpace {
                    primaries: Primaries::Srgb,
                    transfer: Transfer::Linear,
                },
                ..Default::default()
            },
        )
        .unwrap();
        let out = load(&p, None).unwrap();
        assert!((0..4).all(|c| (out.pixels[0][c] - f.pixels[0][c]).abs() < 0.002));
    }
}
