//! Pointwise RGB LUT baking. Spatial effects are rejected, never silently omitted.
use crate::{Engine, RenderContext};
use anyhow::{Context, Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use tinge_color::{ColorSpace, Primaries, convert_linear, decode, encode};
use tinge_core::{
    Frame, Mask, Operation, Recipe,
    lut::{CubeLut, LutInterpolation},
};
use tinge_ocio::{CompiledTransform, Pipeline, Transform, TransformReport};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LutEncoding {
    Standard {
        space: ColorSpace,
    },
    Ocio {
        color_space: String,
    },
    /// Output only: the pipeline's selected display/view and encoding.
    Display,
}
impl Default for LutEncoding {
    fn default() -> Self {
        Self::Standard {
            space: ColorSpace::default(),
        }
    }
}
fn cube_size() -> u32 {
    33
}
fn samples() -> u32 {
    2048
}
fn max_domain() -> [f32; 3] {
    [1.0; 3]
}
fn title() -> String {
    "Tinge RGB grade".into()
}
fn tetrahedral() -> LutInterpolation {
    LutInterpolation::Tetrahedral
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BakeOptions {
    #[serde(default = "cube_size")]
    pub size: u32,
    #[serde(default)]
    pub input_encoding: LutEncoding,
    #[serde(default)]
    pub output_encoding: LutEncoding,
    #[serde(default)]
    pub domain_min: [f32; 3],
    #[serde(default = "max_domain")]
    pub domain_max: [f32; 3],
    #[serde(default = "title")]
    pub title: String,
    #[serde(default = "samples")]
    pub validation_samples: u32,
    #[serde(default = "tetrahedral")]
    pub validation_interpolation: LutInterpolation,
    #[serde(default)]
    pub linear_unit_nits: Option<f32>,
}
impl Default for BakeOptions {
    fn default() -> Self {
        Self {
            size: cube_size(),
            input_encoding: Default::default(),
            output_encoding: Default::default(),
            domain_min: [0.0; 3],
            domain_max: max_domain(),
            title: title(),
            validation_samples: samples(),
            validation_interpolation: tetrahedral(),
            linear_unit_nits: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct SampledError {
    pub samples: u32,
    pub sequence: &'static str,
    pub interpolation: LutInterpolation,
    pub max_absolute_rgb: [f64; 3],
    pub mean_absolute_rgb: [f64; 3],
    pub rms_rgb: [f64; 3],
    pub worst_input_rgb: [[f32; 3]; 3],
}
#[derive(Debug, Serialize)]
pub struct BakeReport {
    pub options: BakeOptions,
    pub sampled_error: SampledError,
    pub snapshot_recipe_hash: String,
    pub opaque_rgb_only: bool,
    pub reachable_nodes: Vec<String>,
    pub input_transform: Option<TransformReport>,
    pub output_transform: Option<TransformReport>,
    pub node_transforms: Vec<crate::Progress>,
    pub output_below_zero_channels: usize,
    pub output_above_one_channels: usize,
    pub outside_domain: &'static str,
}
fn pointwise_mask(mask: &Mask) -> bool {
    match mask {
        Mask::LumaRange { .. } | Mask::HslRange { .. } => true,
        Mask::Combine { masks, .. } => masks.iter().all(pointwise_mask),
        Mask::Invert { mask } => pointwise_mask(mask),
        _ => false,
    }
}
pub fn eligible_nodes(recipe: &Recipe) -> Result<Vec<String>> {
    let order = recipe.validate()?;
    let mut needed = BTreeSet::new();
    let mut pending = vec![recipe.output.clone()];
    while let Some(id) = pending.pop() {
        if id == "source" || !needed.insert(id.clone()) {
            continue;
        }
        let node = recipe
            .nodes
            .iter()
            .find(|n| n.id == id)
            .context("missing LUT bake node")?;
        pending.extend(node.inputs.clone());
    }
    for node in recipe
        .nodes
        .iter()
        .filter(|n| needed.contains(&n.id) && n.enabled)
    {
        let pointwise = match node.op {
            Operation::OcioGrade { .. }
            | Operation::Exposure { .. }
            | Operation::WhiteBalance { .. }
            | Operation::Primary { .. }
            | Operation::Tone { .. }
            | Operation::Cdl { .. }
            | Operation::LogWheels { .. }
            | Operation::HdrZone { .. }
            | Operation::Curves { .. }
            | Operation::HueCurves { .. }
            | Operation::Hsl { .. }
            | Operation::ColorWarper { .. }
            | Operation::RgbMixer { .. }
            | Operation::SplitTone { .. }
            | Operation::Lut { .. }
            | Operation::ToneMap { .. }
            | Operation::GamutCompress { .. }
            | Operation::Blend { .. } => true,
            Operation::Cutout { .. }
            | Operation::Blur { .. }
            | Operation::Sharpen { .. }
            | Operation::Clarity { .. }
            | Operation::Denoise { .. }
            | Operation::Vignette { .. }
            | Operation::Grain { .. }
            | Operation::Glow { .. }
            | Operation::Crop { .. }
            | Operation::Resize { .. }
            | Operation::Rotate { .. }
            | Operation::LensDistortion { .. }
            | Operation::Clone { .. } => false,
        };
        ensure!(
            pointwise,
            "node {} depends on image position/neighbors and cannot be baked into an RGB LUT",
            node.id
        );
        if let Some(mask) = &node.mask {
            ensure!(
                pointwise_mask(&recipe.masks[mask]),
                "node {} uses a spatial/bitmap mask and cannot be baked into an RGB LUT",
                node.id
            );
        }
    }
    Ok(order.into_iter().filter(|id| needed.contains(id)).collect())
}
fn io_processor(
    pipeline: Option<&Pipeline>,
    encoding: &LutEncoding,
    input: bool,
) -> Result<Option<CompiledTransform>> {
    match encoding {
        LutEncoding::Standard { .. } => Ok(None),
        LutEncoding::Display => {
            ensure!(!input, "display is an output-only LUT encoding");
            let p = pipeline.context("display LUT output requires color_pipeline")?;
            // Use public display behavior without inferring an arbitrary custom name.
            let display = p.display_name.as_deref().unwrap_or(p.display.name());
            Ok(Some(tinge_ocio::compile_with_context(
                &p.source(),
                &Transform::DisplayView {
                    source: p.working_space.clone(),
                    display: display.into(),
                    view: p.view.clone(),
                    inverse: false,
                },
                &p.context,
            )?))
        }
        LutEncoding::Ocio { color_space } => {
            let p = pipeline.context("OCIO LUT encoding requires color_pipeline")?;
            tinge_ocio::validate_color_space(&p.source(), color_space)?;
            let (source, destination) = if input {
                (color_space.clone(), p.working_space.clone())
            } else {
                (p.working_space.clone(), color_space.clone())
            };
            Ok(Some(tinge_ocio::compile_with_context(
                &p.source(),
                &Transform::ColorSpace {
                    source,
                    destination,
                },
                &p.context,
            )?))
        }
    }
}
/// Actual files selected by native processors in an editable config, before freezing.
/// Used to prevent atomic export from replacing a transitive LUT input.
pub fn editable_dependencies(
    recipe: &Recipe,
    pipeline: Option<&Pipeline>,
    origin: &Path,
    options: &BakeOptions,
) -> Result<BTreeSet<String>> {
    let Some(p) = pipeline.map(|p| p.resolved_at(origin)) else {
        return Ok(BTreeSet::new());
    };
    if !matches!(p.source(), tinge_ocio::ConfigSource::File { .. }) {
        return Ok(BTreeSet::new());
    }
    let mut processors: Vec<_> = tinge_project::compile_color_nodes(recipe, Some(&p))?
        .into_values()
        .collect();
    for (encoding, input) in [
        (&options.input_encoding, true),
        (&options.output_encoding, false),
    ] {
        if let Some(processor) = io_processor(Some(&p), encoding, input)? {
            processors.push(processor);
        }
    }
    Ok(processors
        .into_iter()
        .flat_map(|p| p.identity().files.clone())
        .collect())
}
fn decode_input(
    pixels: &mut [[f32; 4]],
    encoding: &LutEncoding,
    pipeline: Option<&Pipeline>,
    processor: Option<&CompiledTransform>,
    linear_unit_nits: Option<f32>,
) -> Result<Option<TransformReport>> {
    if let LutEncoding::Standard { space } = encoding {
        for p in pixels {
            let mut rgb = convert_linear(
                [
                    decode(p[0], space.transfer),
                    decode(p[1], space.transfer),
                    decode(p[2], space.transfer),
                ],
                space.primaries,
                Primaries::Srgb,
            );
            if space.transfer == tinge_color::Transfer::Pq {
                rgb = rgb.map(|v| v * 10000.0 / linear_unit_nits.unwrap());
            }
            p[..3].copy_from_slice(&rgb);
        }
        return Ok(None);
    }
    let report = processor
        .context("missing input processor")?
        .apply(pixels)?;
    let anchor = pipeline
        .context("missing pipeline")?
        .working_encoding
        .primaries;
    for p in pixels {
        let rgb = convert_linear([p[0], p[1], p[2]], anchor, Primaries::Srgb);
        p[..3].copy_from_slice(&rgb);
    }
    Ok(Some(report))
}
fn encode_output(
    pixels: &mut [[f32; 4]],
    encoding: &LutEncoding,
    pipeline: Option<&Pipeline>,
    processor: Option<&CompiledTransform>,
    linear_unit_nits: Option<f32>,
) -> Result<Option<TransformReport>> {
    if let LutEncoding::Standard { space } = encoding {
        for p in pixels {
            let mut rgb = convert_linear([p[0], p[1], p[2]], Primaries::Srgb, space.primaries);
            if space.transfer == tinge_color::Transfer::Pq {
                rgb = rgb.map(|v| v * linear_unit_nits.unwrap() / 10000.0);
            }
            let rgb = rgb.map(|v| encode(v, space.transfer));
            p[..3].copy_from_slice(&rgb);
        }
        return Ok(None);
    }
    let anchor = pipeline
        .context("missing pipeline")?
        .working_encoding
        .primaries;
    for p in pixels.iter_mut() {
        let rgb = convert_linear([p[0], p[1], p[2]], Primaries::Srgb, anchor);
        p[..3].copy_from_slice(&rgb);
    }
    Ok(Some(
        processor
            .context("missing output processor")?
            .apply(pixels)?,
    ))
}
fn halton(mut index: u32, base: u32) -> f32 {
    let mut fraction = 1.0f64;
    let mut result = 0.0;
    while index > 0 {
        fraction /= base as f64;
        result += fraction * (index % base) as f64;
        index /= base;
    }
    result as f32
}

/// Returns bytes for atomic export only after dependency freezing and held-out QA.
pub fn bake(
    recipe: Recipe,
    pipeline: Option<Pipeline>,
    origin: &Path,
    options: BakeOptions,
) -> Result<(String, BakeReport)> {
    ensure!(
        (2..=129).contains(&options.size),
        "LUT bake size must be 2..129"
    );
    ensure!(
        (1..=65536).contains(&options.validation_samples),
        "LUT validation_samples must be 1..65536"
    );
    ensure!(
        (0..3).all(|c| options.domain_min[c].is_finite()
            && options.domain_max[c].is_finite()
            && options.domain_min[c] < options.domain_max[c]),
        "invalid LUT bake domain"
    );
    ensure!(
        !options.title.contains(['"', '\r', '\n', '\0']),
        "invalid LUT bake title"
    );
    let reachable_nodes = eligible_nodes(&recipe)?;
    let manual_pq = matches!(&options.input_encoding,LutEncoding::Standard {space} if space.transfer==tinge_color::Transfer::Pq)
        || matches!(&options.output_encoding,LutEncoding::Standard {space} if space.transfer==tinge_color::Transfer::Pq);
    if manual_pq {
        ensure!(
            options
                .linear_unit_nits
                .is_some_and(|v| v.is_finite() && v > 0.0),
            "standard PQ LUT input/output requires positive linear_unit_nits"
        );
    } else {
        ensure!(
            options.linear_unit_nits.is_none(),
            "linear_unit_nits is only for standard PQ LUT input/output"
        );
    }
    let temp = tempfile::tempdir()?;
    let snapshot = temp.path().join("bake.tinge");
    let (recipe, pipeline) = tinge_project::snapshot_recipe(recipe, pipeline, &snapshot, origin)?;
    let snapshot_recipe_hash = blake3::hash(&serde_json::to_vec(&(&recipe, &pipeline))?)
        .to_hex()
        .to_string();
    let pipeline = pipeline.map(|p| p.resolved_at(temp.path()));
    if let Some(p) = &pipeline {
        p.validate()?;
    }
    let input = io_processor(pipeline.as_ref(), &options.input_encoding, true)?;
    let output = io_processor(pipeline.as_ref(), &options.output_encoding, false)?;
    let mut engine = Engine::new();
    let cancel = AtomicBool::new(false);
    let context = || RenderContext {
        asset_base: temp.path(),
        color_pipeline: pipeline.as_ref(),
    };
    let n = options.size as usize;
    let map_domain = |t: [f32; 3]| {
        std::array::from_fn(|c| {
            (options.domain_min[c] as f64
                + t[c] as f64 * (options.domain_max[c] as f64 - options.domain_min[c] as f64))
                as f32
        })
    };
    let mut grid = Vec::with_capacity(n.pow(3));
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let rgb = map_domain([
                    r as f32 / (n - 1) as f32,
                    g as f32 / (n - 1) as f32,
                    b as f32 / (n - 1) as f32,
                ]);
                grid.push([rgb[0], rgb[1], rgb[2], 1.0]);
            }
        }
    }
    let input_transform = decode_input(
        &mut grid,
        &options.input_encoding,
        pipeline.as_ref(),
        input.as_ref(),
        options.linear_unit_nits,
    )?;
    let source = Arc::new(Frame::new(grid.len() as u32, 1, grid)?);
    let mut node_transforms = Vec::new();
    let rendered = engine.render_with_context(source, &recipe, context(), &cancel, &mut |p| {
        node_transforms.push(p)
    })?;
    let mut values = rendered.pixels.clone();
    let output_transform = encode_output(
        &mut values,
        &options.output_encoding,
        pipeline.as_ref(),
        output.as_ref(),
        options.linear_unit_nits,
    )?;
    ensure!(
        values.iter().flatten().all(|v| v.is_finite()),
        "LUT bake produced non-finite samples"
    );
    let output_below_zero_channels = values
        .iter()
        .flat_map(|p| &p[..3])
        .filter(|v| **v < 0.0)
        .count();
    let output_above_one_channels = values
        .iter()
        .flat_map(|p| &p[..3])
        .filter(|v| **v > 1.0)
        .count();
    let lut = CubeLut::from_3d(
        n,
        options.domain_min,
        options.domain_max,
        values.iter().map(|p| [p[0], p[1], p[2]]).collect(),
        options.title.clone(),
    )?;
    let text = lut.write_3d()?;
    // Validate the serialized artifact, not a higher-precision table before export.
    let lut = CubeLut::parse(&text)?;
    let held_out: Vec<[f32; 3]> = (1..=options.validation_samples)
        .map(|i| map_domain([halton(i, 2), halton(i, 3), halton(i, 5)]))
        .collect();
    let mut pixels: Vec<[f32; 4]> = held_out.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect();
    decode_input(
        &mut pixels,
        &options.input_encoding,
        pipeline.as_ref(),
        input.as_ref(),
        options.linear_unit_nits,
    )?;
    let reference = engine.render_with_context(
        Arc::new(Frame::new(pixels.len() as u32, 1, pixels)?),
        &recipe,
        context(),
        &cancel,
        &mut |_| {},
    )?;
    let mut reference = reference.pixels.clone();
    encode_output(
        &mut reference,
        &options.output_encoding,
        pipeline.as_ref(),
        output.as_ref(),
        options.linear_unit_nits,
    )?;
    ensure!(
        reference.iter().flatten().all(|v| v.is_finite()),
        "LUT bake validation produced non-finite values"
    );
    let mut error = SampledError {
        samples: options.validation_samples,
        sequence: "Halton bases 2,3,5; inside declared input domain; sampled error, not a global bound",
        interpolation: options.validation_interpolation,
        max_absolute_rgb: [0.0; 3],
        mean_absolute_rgb: [0.0; 3],
        rms_rgb: [0.0; 3],
        worst_input_rgb: [[0.0; 3]; 3],
    };
    for (rgb, expected) in held_out.iter().zip(&reference) {
        let actual = lut.sample_with(*rgb, options.validation_interpolation);
        for c in 0..3 {
            let e = (actual[c] as f64 - expected[c] as f64).abs();
            if e > error.max_absolute_rgb[c] {
                error.max_absolute_rgb[c] = e;
                error.worst_input_rgb[c] = *rgb;
            }
            error.mean_absolute_rgb[c] += e;
            error.rms_rgb[c] += e * e;
        }
    }
    for c in 0..3 {
        error.mean_absolute_rgb[c] /= options.validation_samples as f64;
        error.rms_rgb[c] = (error.rms_rgb[c] / options.validation_samples as f64).sqrt();
    }
    Ok((
        text,
        BakeReport {
            options,
            sampled_error: error,
            snapshot_recipe_hash,
            opaque_rgb_only: true,
            reachable_nodes,
            input_transform,
            output_transform,
            node_transforms,
            output_below_zero_channels,
            output_above_one_channels,
            outside_domain: "input clamped to declared domain; finite RGB outputs are not clipped",
        },
    ))
}
