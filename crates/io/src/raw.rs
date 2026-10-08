//! RAW decoding/demosaicing adapter. Camera matrices are developed into unclipped linear sRGB.
use super::{BelowBlack, RawCrop, RawDevelopOptions, RawWhiteBalance, SensorLevels};
use anyhow::{Context, Result, ensure};
use rawler::rawimage::{RawImageData, RawPhotometricInterpretation};
use rawler::{
    RawImage,
    imgop::{
        chromatic_adaption::adapt_bradford,
        develop::{Intermediate, ProcessingStep, RawDevelop},
        matrix::{multiply, normalize, pseudo_inverse, transform_1d},
        xyz::{Illuminant, SRGB_TO_XYZ_D65},
    },
};
use std::path::Path;
use vibecolor_core::Frame;

pub use super::RawMetadata as RawInfo;
pub fn is_raw(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|e| {
        [
            "dng", "arw", "cr2", "cr3", "nef", "nrw", "raf", "orf", "rw2", "pef", "srw", "erf",
            "3fr", "iiq", "kdc", "dcr", "mef", "mos", "mrw", "srf", "sr2", "rwl", "x3f", "raw",
        ]
        .contains(&e.to_ascii_lowercase().as_str())
    })
}
fn decode(path: &Path) -> Result<RawImage> {
    let raw = rawler::decode_file(path)
        .context("RAW decode failed (camera/format must be supported by rawler)")?;
    ensure!(
        raw.width as u64 * raw.height as u64 <= 100_000_000,
        "RAW exceeds 100M pixel limit"
    );
    ensure!(
        [1, 3, 4].contains(&raw.cpp),
        "unsupported RAW channel count"
    );
    Ok(raw)
}
pub fn inspect(path: &Path) -> Result<(super::ImageInfo, RawInfo)> {
    let raw = decode(path)?;
    let info = super::ImageInfo {
        width: raw.width as u32,
        height: raw.height as u32,
        color_type: format!("RAW{}", raw.bps),
        format: "CameraRAW / rawler".into(),
        has_icc: false,
        orientation: format!("{:?}", raw.orientation),
        hash: super::hash_file(path)?,
        raw_metadata: None,
        color_metadata: super::FileColorMetadata {
            alpha_mode: "sensor samples".into(),
            ..Default::default()
        },
    };
    let metadata = RawInfo {
        make: raw.clean_make,
        model: raw.clean_model,
        sensor_width: raw.width,
        sensor_height: raw.height,
        bits: raw.bps,
        components: raw.cpp,
        wb_coefficients: raw
            .wb_coeffs
            .iter()
            .map(|v| if v.is_finite() { Some(*v) } else { None })
            .collect(),
        decoder: "rawler 0.8",
        demosaic: "PPG Bayer / bilinear X-Trans and 4-color",
    };
    Ok((info, metadata))
}
pub fn load(path: &Path) -> Result<Frame> {
    let raw = decode(path)?;
    if matches!(raw.photometric, RawPhotometricInterpretation::BlackIsZero) {
        // rawler's Rescale step has no BlackIsZero implementation.
        return load_with_options(path, &RawDevelopOptions::default());
    }
    if let rawler::rawimage::RawPhotometricInterpretation::Cfa(config) = &raw.photometric {
        ensure!(
            (config.colors.plane_count() == 3
                && config.cfa.is_rgb()
                && matches!(
                    config.sensor,
                    rawler::imgop::sensor::SensorType::Bayer
                        | rawler::imgop::sensor::SensorType::Xtrans
                ))
                || (config.colors.plane_count() == 4
                    && config.cfa.unique_colors() == 4
                    && config.sensor == rawler::imgop::sensor::SensorType::Bayer),
            "unsupported RAW sensor pattern"
        );
        ensure!(
            raw.width >= 16 && raw.height >= 16,
            "RAW demosaic requires dimensions >= 16"
        );
        ensure!(
            raw.fuji_rotation_width.is_none() || raw.active_area.is_none(),
            "Fuji rotated sensor with active crop requires separate development support"
        );
    }
    let dev = RawDevelop::new_with(&[
        ProcessingStep::Rescale,
        ProcessingStep::Demosaic,
        ProcessingStep::FujiRotate,
        ProcessingStep::CropActiveArea,
        ProcessingStep::CropDefault,
    ]);
    let developed = dev.develop_intermediate(&raw)?;
    let dim = developed.dim();
    let cam: Vec<[f32; 4]> = match developed {
        Intermediate::Monochrome(p) => {
            let pixels = p.data.into_iter().map(|v| [v, v, v, 1.0]).collect();
            return orient(
                Frame::new(dim.w as u32, dim.h as u32, pixels)?,
                raw.orientation.to_u16(),
            );
        }
        Intermediate::ThreeColor(p) => p
            .data
            .into_iter()
            .map(|v| [v[0], v[1], v[2], 0.0])
            .collect(),
        Intermediate::FourColor(p) => p.data,
    };
    let (illuminant, matrix) = raw
        .color_matrix_find_first([
            Illuminant::D65,
            Illuminant::A,
            Illuminant::D50,
            Illuminant::D55,
            Illuminant::D75,
            Illuminant::Daylight,
            Illuminant::Flash,
        ])
        .context("RAW has no supported camera calibration matrix")?;
    let matrix = if illuminant == Illuminant::D65 {
        matrix
    } else {
        ensure!(
            matrix.len() == 9,
            "non-D65 four-color camera matrix is not supported"
        );
        adapt_bradford(
            &illuminant,
            &Illuminant::D65,
            &transform_1d::<3, 3>(&matrix).context("invalid matrix")?,
        )
        .as_flattened()
        .to_vec()
    };
    ensure!(
        [9, 12].contains(&matrix.len()),
        "invalid camera color matrix"
    );
    let mut xyz2cam = [[0.0; 3]; 4];
    for (i, row) in matrix.chunks_exact(3).enumerate() {
        xyz2cam[i].copy_from_slice(row);
    }
    let cam2rgb = pseudo_inverse(normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65)));
    ensure!(
        cam2rgb.iter().flatten().all(|v| v.is_finite()),
        "singular RAW camera calibration"
    );
    let mut wb = raw.wb_coeffs;
    if !wb[..matrix.len() / 3]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
    {
        wb = white_gains(&xyz2cam, matrix.len() / 3, [0.3127, 0.3290])?;
    }
    ensure!(
        wb[..matrix.len() / 3]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0),
        "invalid camera white balance"
    );
    let green = wb[1];
    for v in &mut wb {
        *v = if v.is_finite() && *v > 0.0 {
            *v / green
        } else {
            1.0
        };
    }
    let pixels = cam
        .into_iter()
        .map(|p| {
            let rgb: [f32; 3] =
                std::array::from_fn(|c| (0..4).map(|i| cam2rgb[c][i] * p[i] * wb[i]).sum());
            [rgb[0], rgb[1], rgb[2], 1.0]
        })
        .collect();
    orient(
        Frame::new(dim.w as u32, dim.h as u32, pixels)?,
        raw.orientation.to_u16(),
    )
}
fn orient(frame: Frame, orientation: u16) -> Result<Frame> {
    // Retain f32 values and apply EXIF orientation without integer conversion.
    let (w, h) = if matches!(orientation, 5..=8) {
        (frame.height, frame.width)
    } else {
        (frame.width, frame.height)
    };
    let pixels = (0..w * h)
        .map(|i| {
            let x = i % w;
            let y = i / w;
            let (sx, sy) = match orientation {
                2 => (frame.width - 1 - x, y),
                3 => (frame.width - 1 - x, frame.height - 1 - y),
                4 => (x, frame.height - 1 - y),
                5 => (y, x),
                6 => (y, frame.height - 1 - x),
                7 => (frame.width - 1 - y, frame.height - 1 - x),
                8 => (frame.width - 1 - y, x),
                _ => (x, y),
            };
            frame.pixels[(sy * frame.width + sx) as usize]
        })
        .collect();
    Frame::new(w, h, pixels)
}

#[derive(Debug, serde::Serialize)]
pub struct RawPlan {
    pub options: RawDevelopOptions,
    pub sensor_dimensions: [usize; 2],
    pub components_per_pixel: usize,
    pub camera_planes: usize,
    pub photometric: String,
    pub plane_order: Vec<String>,
    pub cfa_pattern: Option<String>,
    pub black_levels: SensorLevels,
    pub white_levels: SensorLevels,
    pub white_balance_gains: Vec<f32>,
    pub white_balance_source: String,
    pub white_balance_white_xy: Option<[f32; 2]>,
    pub calibration_illuminant: Option<u16>,
    pub available_calibration_illuminants: Vec<u16>,
    pub xyz_to_camera_d65: Vec<[f32; 3]>,
    pub active_area: Option<[usize; 4]>,
    pub default_crop: Option<[usize; 4]>,
    pub orientation: u16,
    pub warnings: Vec<String>,
    pub sensor_probes: Vec<SensorProbe>,
}
#[derive(Debug, serde::Serialize)]
pub struct SensorProbe {
    pub position: [usize; 2],
    pub native_values: Vec<f32>,
    pub normalized_before_wb: Vec<f32>,
}
fn rect_array(r: rawler::imgop::Rect) -> [usize; 4] {
    [r.p.x, r.p.y, r.d.w, r.d.h]
}
fn white_gains(matrix: &[[f32; 3]; 4], planes: usize, xy: [f32; 2]) -> Result<[f32; 4]> {
    let xyz = [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]];
    let mut wb = [1.0; 4];
    for i in 0..planes {
        let response: f32 = (0..3).map(|c| matrix[i][c] * xyz[c]).sum();
        ensure!(
            response.is_finite() && response > 0.0,
            "camera matrix cannot represent requested white point"
        );
        wb[i] = 1.0 / response;
    }
    let reference = wb[1];
    for v in &mut wb {
        *v /= reference;
    }
    ensure!(
        wb[..planes].iter().all(|v| v.is_finite() && *v > 0.0),
        "invalid computed camera white balance"
    );
    Ok(wb)
}
fn prepare(raw: &RawImage, options: &RawDevelopOptions) -> Result<RawPlan> {
    options.validate()?;
    ensure!(
        raw.fuji_rotation_width.is_none(),
        "version 1 RAW controls do not support rotated Fuji sensors; use legacy decode or a normalized RAW"
    );
    let (planes, order, cfa) = match &raw.photometric {
        RawPhotometricInterpretation::BlackIsZero => {
            ensure!(raw.cpp == 1, "monochrome RAW must have one component");
            (1, vec!["monochrome".into()], None)
        }
        RawPhotometricInterpretation::LinearRaw => {
            ensure!(
                [1, 3, 4].contains(&raw.cpp),
                "unsupported linear RAW components"
            );
            (
                raw.cpp,
                (0..raw.cpp).map(|i| format!("camera_{i}")).collect(),
                None,
            )
        }
        RawPhotometricInterpretation::Cfa(config) => {
            ensure!(
                raw.cpp == 1 && raw.width >= 16 && raw.height >= 16,
                "CFA RAW requires one component and dimensions >=16"
            );
            let n = config.colors.plane_count();
            ensure!(
                (n == 3
                    && config.cfa.is_rgb()
                    && matches!(
                        config.sensor,
                        rawler::imgop::sensor::SensorType::Bayer
                            | rawler::imgop::sensor::SensorType::Xtrans
                    ))
                    || (n == 4
                        && config.cfa.unique_colors() == 4
                        && config.sensor == rawler::imgop::sensor::SensorType::Bayer),
                "unsupported RAW sensor pattern or plane count"
            );
            let lookup = config.colors.plane_lookup_table();
            for y in 0..config.cfa.height {
                for x in 0..config.cfa.width {
                    ensure!(
                        lookup[config.cfa.color_at(y, x)] < n,
                        "CFA refers to an absent camera plane"
                    );
                }
            }
            (
                n,
                config
                    .colors
                    .colors
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect(),
                Some(config.cfa.name.clone()),
            )
        }
    };
    for crop in [raw.active_area, raw.crop_area].into_iter().flatten() {
        ensure!(
            crop.d.w > 0
                && crop.d.h > 0
                && crop
                    .p
                    .x
                    .checked_add(crop.d.w)
                    .is_some_and(|x| x <= raw.width)
                && crop
                    .p
                    .y
                    .checked_add(crop.d.h)
                    .is_some_and(|y| y <= raw.height),
            "invalid RAW crop bounds"
        );
    }
    if options.crop != RawCrop::Sensor {
        if planes > 1
            && matches!(raw.photometric, RawPhotometricInterpretation::Cfa(_))
            && let Some(active) = raw.active_area
        {
            ensure!(
                active.d.w >= 16 && active.d.h >= 16,
                "RAW active area is too small for demosaic"
            );
        }
        if let (Some(active), Some(crop)) = (raw.active_area, raw.crop_area) {
            ensure!(
                crop.p.x >= active.p.x
                    && crop.p.y >= active.p.y
                    && crop.p.x + crop.d.w <= active.p.x + active.d.w
                    && crop.p.y + crop.d.h <= active.p.y + active.d.h,
                "default RAW crop lies outside active area"
            );
        }
    }
    let black = options.black_levels.clone().unwrap_or(SensorLevels {
        repeat: [raw.blacklevel.width, raw.blacklevel.height],
        values: raw.blacklevel.as_vec(),
    });
    let white = options.white_levels.clone().unwrap_or_else(|| {
        let values = raw.whitelevel.as_vec();
        let repeat = if values.len() == raw.cpp {
            [1, 1]
        } else if raw.cpp == 1 && values.len() == 4 {
            [2, 2]
        } else {
            [1, 1]
        };
        SensorLevels { repeat, values }
    });
    black.validate_components(raw.cpp)?;
    white.validate_components(raw.cpp)?;
    // Both patterns repeat; checking their product spans every paired residue.
    for y in 0..black.repeat[1] * white.repeat[1] {
        for x in 0..black.repeat[0] * white.repeat[0] {
            for c in 0..raw.cpp {
                ensure!(
                    white.at(x, y, c, raw.cpp) > black.at(x, y, c, raw.cpp),
                    "RAW white level must exceed black level at every sensor position"
                );
            }
        }
    }
    let mut warnings = vec!["Matrix-only camera calibration; no DCP/ICC camera look, dual-illuminant interpolation, lens correction or highlight reconstruction.".into()];
    let (illuminant, xyz2cam) = if planes == 1 {
        ensure!(
            options.calibration_illuminant.is_none(),
            "monochrome RAW has no color calibration selection"
        );
        ensure!(
            matches!(
                options.white_balance,
                RawWhiteBalance::AsShot | RawWhiteBalance::Unity
            ),
            "monochrome RAW does not support chromatic white balance"
        );
        (None, [[0.0; 3]; 4])
    } else {
        let (illuminant, matrix) = if let Some(tag) = options.calibration_illuminant {
            let i = Illuminant::try_from(tag)
                .map_err(|e| anyhow::anyhow!("unknown calibration illuminant tag: {e}"))?;
            (
                i,
                raw.color_matrix
                    .get(&i)
                    .context("requested calibration illuminant is not available")?
                    .clone(),
            )
        } else {
            raw.color_matrix_find_first([
                Illuminant::D65,
                Illuminant::A,
                Illuminant::D50,
                Illuminant::D55,
                Illuminant::D75,
                Illuminant::Daylight,
                Illuminant::Flash,
            ])
            .context("RAW has no supported camera calibration matrix")?
        };
        ensure!(
            matrix.len() == planes * 3 && matrix.iter().all(|v| v.is_finite()),
            "invalid RAW calibration matrix dimensions or values"
        );
        let matrix = if illuminant == Illuminant::D65 {
            matrix
        } else {
            ensure!(
                planes == 3,
                "non-D65 four-color camera matrix is unsupported"
            );
            // The camera matrix consumes XYZ under its calibration illuminant.
            // Feed D65 XYZ through CAT(D65 -> calibration), then that matrix.
            adapt_bradford(
                &Illuminant::D65,
                &illuminant,
                &transform_1d::<3, 3>(&matrix).context("invalid camera matrix")?,
            )
            .as_flattened()
            .to_vec()
        };
        let mut m = [[0.0; 3]; 4];
        for (i, row) in matrix.chunks_exact(3).enumerate() {
            m[i].copy_from_slice(row);
        }
        let cam2rgb = pseudo_inverse(normalize(multiply(&m, &SRGB_TO_XYZ_D65)));
        ensure!(
            cam2rgb.iter().flatten().all(|v| v.is_finite()),
            "singular RAW calibration matrix"
        );
        (Some(illuminant as u16), m)
    };
    if matches!(illuminant, Some(1 | 4)) {
        warnings.push("Generic Daylight/Flash calibration uses an approximate D65/D55 reference white, respectively.".into());
    }
    let temperature_xy =
        if let RawWhiteBalance::Temperature { kelvin, tint_duv } = &options.white_balance {
            Some(super::temperature_white_xy(*kelvin, *tint_duv)?.map(|v| v as f32))
        } else {
            None
        };
    let (mut wb, source) = if planes == 1 {
        ([1.0; 4], "monochrome")
    } else {
        match &options.white_balance {
            RawWhiteBalance::AsShot => {
                if raw.wb_coeffs[..planes]
                    .iter()
                    .all(|v| v.is_finite() && *v > 0.0)
                {
                    (raw.wb_coeffs, "as_shot")
                } else {
                    warnings.push("As-shot white balance absent or invalid; selected camera matrix used for D65 xy fallback.".into());
                    (
                        white_gains(&xyz2cam, planes, [0.3127, 0.3290])?,
                        "matrix_d65_fallback",
                    )
                }
            }
            RawWhiteBalance::Unity => ([1.0; 4], "unity"),
            RawWhiteBalance::CameraGains { gains } => {
                ensure!(
                    gains.len() == planes,
                    "white balance gain count must equal camera planes"
                );
                let mut v = [1.0; 4];
                v[..planes].copy_from_slice(gains);
                (v, "camera_gains")
            }
            RawWhiteBalance::Chromaticity { xy } => {
                (white_gains(&xyz2cam, planes, *xy)?, "chromaticity")
            }
            RawWhiteBalance::Temperature { .. } => (
                white_gains(&xyz2cam, planes, temperature_xy.unwrap())?,
                "temperature_kang2002_duv",
            ),
        }
    };
    let green = wb[1];
    for v in &mut wb {
        *v /= green;
    }
    ensure!(
        wb[..planes].iter().all(|v| v.is_finite() && *v > 0.0),
        "invalid RAW white balance"
    );
    Ok(RawPlan {
        options: options.clone(),
        sensor_dimensions: [raw.width, raw.height],
        components_per_pixel: raw.cpp,
        camera_planes: planes,
        photometric: match &raw.photometric {
            RawPhotometricInterpretation::Cfa(c) => format!("CFA / {:?}", c.sensor),
            RawPhotometricInterpretation::LinearRaw => "LinearRaw".into(),
            RawPhotometricInterpretation::BlackIsZero => "BlackIsZero".into(),
        },
        plane_order: order,
        cfa_pattern: cfa,
        black_levels: black,
        white_levels: white,
        white_balance_gains: wb[..planes].to_vec(),
        white_balance_source: source.into(),
        white_balance_white_xy: match &options.white_balance {
            RawWhiteBalance::Temperature { .. } => temperature_xy,
            RawWhiteBalance::Chromaticity { xy } => Some(*xy),
            RawWhiteBalance::AsShot if source == "matrix_d65_fallback" => Some([0.3127, 0.3290]),
            _ => None,
        },
        calibration_illuminant: illuminant,
        available_calibration_illuminants: {
            let mut tags: Vec<_> = raw.color_matrix.keys().map(|i| *i as u16).collect();
            tags.sort_unstable();
            tags
        },
        xyz_to_camera_d65: if planes == 1 {
            vec![]
        } else {
            xyz2cam[..planes].to_vec()
        },
        active_area: raw.active_area.map(rect_array),
        default_crop: raw.crop_area.map(rect_array),
        orientation: raw.orientation.to_u16(),
        warnings,
        sensor_probes: Vec::new(),
    })
}
/// Decode sensor metadata and resolve controls without demosaicing or project mutation.
pub fn plan(path: &Path, options: &RawDevelopOptions) -> Result<RawPlan> {
    plan_with_probes(path, options, &[])
}
pub fn plan_with_probes(
    path: &Path,
    options: &RawDevelopOptions,
    points: &[[usize; 2]],
) -> Result<RawPlan> {
    ensure!(is_raw(path), "RAW controls require a RAW input");
    ensure!(points.len() <= 256, "RAW sensor probe limit is 256");
    let raw = decode(path)?;
    let mut plan = prepare(&raw, options)?;
    let samples = raw.data.as_f32();
    ensure!(
        samples.len() == raw.width * raw.height * raw.cpp,
        "RAW sample count mismatch"
    );
    for &[x, y] in points {
        ensure!(
            x < raw.width && y < raw.height,
            "RAW sensor probe is outside sensor bounds"
        );
        let start = (y * raw.width + x) * raw.cpp;
        let native_values = samples[start..start + raw.cpp].to_vec();
        let normalized_before_wb: Vec<_> = native_values
            .iter()
            .enumerate()
            .map(|(c, v)| {
                let black = plan.black_levels.at(x, y, c, raw.cpp);
                let white = plan.white_levels.at(x, y, c, raw.cpp);
                let v = (*v - black) / (white - black);
                if options.below_black == BelowBlack::Clip {
                    v.max(0.0)
                } else {
                    v
                }
            })
            .collect();
        ensure!(
            native_values
                .iter()
                .chain(&normalized_before_wb)
                .all(|v| v.is_finite()),
            "non-finite RAW probe value"
        );
        plan.sensor_probes.push(SensorProbe {
            position: [x, y],
            native_values,
            normalized_before_wb,
        });
    }
    Ok(plan)
}
/// Version 1: full sensor repeat-pattern normalization and camera WB before demosaic.
pub fn load_with_options(path: &Path, options: &RawDevelopOptions) -> Result<Frame> {
    ensure!(is_raw(path), "RAW controls require a RAW input");
    let mut raw = decode(path)?;
    let plan = prepare(&raw, options)?;
    let mut samples = raw.data.as_f32().into_owned();
    ensure!(
        samples.len() == raw.width * raw.height * raw.cpp,
        "RAW sample count mismatch"
    );
    let lookup = if let RawPhotometricInterpretation::Cfa(config) = &raw.photometric {
        Some(config.colors.plane_lookup_table())
    } else {
        None
    };
    for (i, value) in samples.iter_mut().enumerate() {
        let c = i % raw.cpp;
        let x = (i / raw.cpp) % raw.width;
        let y = (i / raw.cpp) / raw.width;
        let black = plan.black_levels.at(x, y, c, raw.cpp);
        let white = plan.white_levels.at(x, y, c, raw.cpp);
        let normalized = (*value - black) / (white - black);
        let normalized = if options.below_black == BelowBlack::Clip {
            normalized.max(0.0)
        } else {
            normalized
        };
        let plane = if let RawPhotometricInterpretation::Cfa(config) = &raw.photometric {
            lookup.as_ref().unwrap()[config.cfa.color_at(y, x)]
        } else {
            c
        };
        *value = normalized * plan.white_balance_gains[plane];
        ensure!(
            value.is_finite(),
            "non-finite RAW sensor sample after normalization"
        );
    }
    raw.data = RawImageData::Float(samples);
    let mut steps = vec![ProcessingStep::Demosaic];
    let active_demosaic = options.crop != RawCrop::Sensor
        && matches!(raw.photometric, RawPhotometricInterpretation::Cfa(_))
        && raw.active_area.is_some();
    if active_demosaic {
        steps.push(ProcessingStep::CropActiveArea);
    }
    let developed = RawDevelop::new_with(&steps).develop_intermediate(&raw)?;
    let dim = developed.dim();
    let cam2rgb = if plan.camera_planes > 1 {
        let mut m = [[0.0; 3]; 4];
        m[..plan.camera_planes].copy_from_slice(&plan.xyz_to_camera_d65);
        pseudo_inverse(normalize(multiply(&m, &SRGB_TO_XYZ_D65)))
    } else {
        [[0.0; 4]; 3]
    };
    let rgb = |p: [f32; 4]| -> [f32; 4] {
        let mut v = [0.0; 4];
        for c in 0..3 {
            v[c] = (0..plan.camera_planes)
                .map(|i| cam2rgb[c][i] * p[i])
                .sum::<f32>()
                * options.exposure_ev.exp2();
        }
        v[3] = 1.0;
        v
    };
    let pixels = match developed {
        Intermediate::Monochrome(p) => p
            .data
            .into_iter()
            .map(|v| {
                let v = v * options.exposure_ev.exp2();
                [v, v, v, 1.0]
            })
            .collect(),
        Intermediate::ThreeColor(p) => p
            .data
            .into_iter()
            .map(|p| rgb([p[0], p[1], p[2], 0.0]))
            .collect(),
        Intermediate::FourColor(p) => p.data.into_iter().map(rgb).collect(),
    };
    let frame = Frame::new(dim.w as u32, dim.h as u32, pixels)?;
    let crop = match options.crop {
        RawCrop::Sensor => None,
        RawCrop::Active => raw.active_area,
        RawCrop::Default => raw.crop_area.or(raw.active_area),
    };
    let frame = if let Some(crop) = crop {
        let origin = if active_demosaic {
            raw.active_area.unwrap().p
        } else {
            rawler::imgop::Point::zero()
        };
        let x = crop
            .p
            .x
            .checked_sub(origin.x)
            .context("RAW crop precedes developed region")?;
        let y = crop
            .p
            .y
            .checked_sub(origin.y)
            .context("RAW crop precedes developed region")?;
        ensure!(
            x + crop.d.w <= dim.w && y + crop.d.h <= dim.h,
            "RAW crop exceeds developed region"
        );
        let mut out = Vec::with_capacity(crop.d.w * crop.d.h);
        for row in y..y + crop.d.h {
            let start = row * dim.w + x;
            out.extend_from_slice(&frame.pixels[start..start + crop.d.w]);
        }
        Frame::new(crop.d.w as u32, crop.d.h as u32, out)?
    } else {
        frame
    };
    if options.apply_orientation {
        orient(frame, plan.orientation)
    } else {
        Ok(frame)
    }
}
