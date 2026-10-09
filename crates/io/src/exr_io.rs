//! Native EXR metadata, primary conversion, and alpha association.
use crate::SignalImage;
use anyhow::{Context, Result, bail, ensure};
use exr::{
    meta::attribute::{AttributeValue, Chromaticities, Text},
    prelude::*,
};
use std::path::Path;
use tinge_color::{ColorSpace, Primaries, Transfer, convert_linear, decode};
use tinge_core::Frame;

pub fn chromaticities(p: Primaries) -> Chromaticities {
    let (r, g, b, w) = match p {
        Primaries::Srgb => ((0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)),
        Primaries::DisplayP3 => ((0.68, 0.32), (0.265, 0.69), (0.15, 0.06), (0.3127, 0.3290)),
        Primaries::Rec2020 => (
            (0.708, 0.292),
            (0.170, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
        ),
        Primaries::AcesCg => (
            (0.713, 0.293),
            (0.165, 0.830),
            (0.128, 0.044),
            (0.32168, 0.33767),
        ),
    };
    Chromaticities {
        red: r.into(),
        green: g.into(),
        blue: b.into(),
        white: w.into(),
    }
}
fn matches(a: &Chromaticities, b: &Chromaticities) -> bool {
    [a.red, a.green, a.blue, a.white]
        .into_iter()
        .zip([b.red, b.green, b.blue, b.white])
        .all(|(a, b)| (a.x() - b.x()).abs() < 1e-5 && (a.y() - b.y()).abs() < 1e-5)
}
pub fn interop_id(p: Primaries) -> &'static str {
    match p {
        Primaries::Srgb => "lin_rec709_scene",
        Primaries::DisplayP3 => "lin_p3d65_scene",
        Primaries::Rec2020 => "lin_rec2020_scene",
        Primaries::AcesCg => "lin_ap1_scene",
    }
}
pub fn metadata(path: &Path) -> Result<MetaData> {
    let meta = MetaData::read_from_file(path, true)?;
    for header in &meta.headers {
        ensure!(
            header.layer_size.area() <= 100_000_000,
            "EXR exceeds 100M pixel limit"
        );
    }
    Ok(meta)
}
pub fn id(attributes: &ImageAttributes, layer: &LayerAttributes) -> Option<String> {
    attributes
        .other
        .get(b"colorInteropID".as_slice())
        .or_else(|| layer.other.get(b"colorInteropID".as_slice()))
        .and_then(|a| match a {
            AttributeValue::Text(t) => Some(t.to_string()),
            _ => None,
        })
}
fn tagged_primary(interop: &str) -> Option<Primaries> {
    let base = interop.split_once(':').map_or(interop, |(_, tail)| tail);
    [
        Primaries::Srgb,
        Primaries::DisplayP3,
        Primaries::Rec2020,
        Primaries::AcesCg,
    ]
    .into_iter()
    .find(|p| interop_id(*p) == base)
}
pub fn is_straight(layer: &LayerAttributes) -> bool {
    layer
        .other
        .get(b"tingeAlphaMode".as_slice())
        .is_some_and(|v| matches!(v, AttributeValue::Text(t) if t=="straight"))
}
fn read_signal(path: &Path) -> Result<(SignalImage, ImageAttributes, LayerAttributes)> {
    metadata(path)?;
    let image = read_first_rgba_layer_from_file(
        path,
        |size, _| (size.width(), vec![[0.0; 4]; size.area()]),
        |(width, pixels), pos, (r, g, b, a): (f32, f32, f32, f32)| {
            pixels[pos.y() * *width + pos.x()] = [r, g, b, a];
        },
    )?;
    let size = image.layer_data.size;
    let mut pixels = image.layer_data.channel_data.pixels.1;
    let explicit_straight = is_straight(&image.layer_data.attributes);
    if !explicit_straight {
        for p in &mut pixels {
            if p[3] > 0.0 {
                for c in 0..3 {
                    p[c] /= p[3];
                }
            } else {
                ensure!(
                    p[..3].iter().all(|v| *v == 0.0),
                    "EXR additive color at zero alpha needs an additive compositing model; current Frame uses straight alpha"
                );
            }
        }
    }
    let image_pixels = SignalImage {
        width: size.width() as u32,
        height: size.height() as u32,
        pixels,
    };
    image_pixels.validate()?;
    Ok((image_pixels, image.attributes, image.layer_data.attributes))
}
pub fn signal(path: &Path) -> Result<SignalImage> {
    Ok(read_signal(path)?.0)
}
pub fn load(path: &Path, explicit: Option<ColorSpace>) -> Result<Frame> {
    let (mut image, attributes, layer) = read_signal(path)?;
    let tagged = id(&attributes, &layer);
    let standard = if explicit.is_none() {
        if let Some(id) = tagged.as_deref().filter(|id| *id != "unknown") {
            ensure!(
                id != "data",
                "EXR data channels are not a color image; declare input_space to override"
            );
            let p=tagged_primary(id).with_context(||format!("unrecognized EXR colorInteropID {id}; declare input_space or use an explicit OCIO input"))?;
            if let Some(c) = &attributes.chromaticities {
                ensure!(
                    matches(c, &chromaticities(p)),
                    "EXR colorInteropID and chromaticities disagree; declare input_space to resolve"
                );
            }
            Some(p)
        } else if let Some(c) = &attributes.chromaticities {
            [
                Primaries::Srgb,
                Primaries::DisplayP3,
                Primaries::Rec2020,
                Primaries::AcesCg,
            ]
            .into_iter()
            .find(|p| matches(c, &chromaticities(*p)))
        } else {
            Some(Primaries::Srgb)
        } // explicit documented legacy fallback
    } else {
        None
    };
    if let Some(space) = explicit.or_else(|| {
        standard.map(|primaries| ColorSpace {
            primaries,
            transfer: Transfer::Linear,
        })
    }) {
        for p in &mut image.pixels {
            let rgb = convert_linear(
                [p[0], p[1], p[2]].map(|v| decode(v, space.transfer)),
                space.primaries,
                Primaries::Srgb,
            );
            p[..3].copy_from_slice(&rgb);
        }
    } else if let Some(c) = attributes.chromaticities {
        let matrix = custom_matrix(c)?;
        for p in &mut image.pixels {
            let rgb = matrix.v.map(|row| {
                (row[0] * p[0] as f64 + row[1] * p[1] as f64 + row[2] * p[2] as f64) as f32
            });
            p[..3].copy_from_slice(&rgb);
        }
    } else {
        bail!("EXR color metadata cannot be resolved");
    }
    Frame::new(image.width, image.height, image.pixels)
}
fn custom_matrix(c: Chromaticities) -> Result<moxcms::Matrix3d> {
    ensure!(
        [c.red, c.green, c.blue, c.white]
            .iter()
            .all(|v| v.x().is_finite() && v.y().is_finite() && v.y().abs() > 1e-8),
        "invalid EXR chromaticities"
    );
    let chroma = |v: Vec2<f32>| moxcms::Chromaticity { x: v.x(), y: v.y() };
    let matrix = |c: Chromaticities| {
        moxcms::ColorProfile::colorants_matrix(
            moxcms::XyY {
                x: c.white.x() as f64,
                y: c.white.y() as f64,
                yb: 1.0,
            },
            moxcms::ColorPrimaries {
                red: chroma(c.red),
                green: chroma(c.green),
                blue: chroma(c.blue),
            },
        )
    };
    let result = matrix(chromaticities(Primaries::Srgb)).inverse() * matrix(c);
    ensure!(
        result.v.iter().flatten().all(|v| v.is_finite()),
        "singular EXR chromaticities"
    );
    Ok(result)
}
pub fn write(
    writer: impl std::io::Write + std::io::Seek,
    w: u32,
    h: u32,
    raw: &[f32],
    space: ColorSpace,
) -> Result<()> {
    let mut image = Image::from_channels(
        (w as usize, h as usize),
        SpecificChannels::rgba(|p: Vec2<usize>| {
            let i = (p.y() * w as usize + p.x()) * 4;
            let a = raw[i + 3];
            (raw[i] * a, raw[i + 1] * a, raw[i + 2] * a, a)
        }),
    );
    image.attributes.chromaticities = Some(chromaticities(space.primaries));
    image.attributes.other.insert(
        Text::from("colorInteropID"),
        AttributeValue::Text(Text::from(interop_id(space.primaries))),
    );
    image.layer_data.attributes.software_name = Some(Text::from("Tinge"));
    image.layer_data.attributes.other.insert(
        Text::from("tingeAlphaMode"),
        AttributeValue::Text(Text::from("premultiplied")),
    );
    image.write().to_unbuffered(writer)?;
    Ok(())
}
