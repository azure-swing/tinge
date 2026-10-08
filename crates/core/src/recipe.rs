use crate::Mask;
use anyhow::{Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub fn one() -> f32 {
    1.0
}
fn ones() -> [f32; 3] {
    [1.0; 3]
}
fn pivot() -> f32 {
    0.18
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    #[serde(default = "version")]
    pub schema_version: u32,
    #[serde(default)]
    pub nodes: Vec<Node>,
    /// Node ID, or "source" for the unmodified image.
    #[serde(default = "source")]
    pub output: String,
    #[serde(default)]
    pub masks: BTreeMap<String, Mask>,
}
fn version() -> u32 {
    1
}
fn source() -> String {
    "source".into()
}
impl Default for Recipe {
    fn default() -> Self {
        Self {
            schema_version: 1,
            nodes: vec![],
            output: source(),
            masks: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(default = "source_inputs")]
    pub inputs: Vec<String>,
    pub op: Operation,
    #[serde(default)]
    pub mask: Option<String>,
    #[serde(default = "one")]
    pub mix: f32,
    #[serde(default = "enabled")]
    pub enabled: bool,
}
fn source_inputs() -> Vec<String> {
    vec![source()]
}
fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Native weight-free keying / interactive segmentation / alpha matting.
    Cutout {
        options: crate::cutout::CutoutOptions,
    },
    /// Native OCIO grade in an explicit process space; requires color_pipeline.
    OcioGrade {
        grade: vibecolor_ocio::NodeGrade,
    },
    Exposure {
        stops: f32,
    },
    /// Gains in scene-linear RGB, green normalized by the caller when desired.
    WhiteBalance {
        gains: [f32; 3],
    },
    Primary {
        #[serde(default)]
        exposure: f32,
        #[serde(default)]
        lift: [f32; 3],
        #[serde(default = "ones")]
        gamma: [f32; 3],
        #[serde(default = "ones")]
        gain: [f32; 3],
        #[serde(default)]
        offset: [f32; 3],
        #[serde(default = "one")]
        contrast: f32,
        #[serde(default = "pivot")]
        pivot: f32,
        #[serde(default = "one")]
        saturation: f32,
        #[serde(default)]
        vibrance: f32,
    },
    /// Lightroom-style independent tone controls; values in [-1,1].
    Tone {
        #[serde(default)]
        shadows: f32,
        #[serde(default)]
        highlights: f32,
        #[serde(default)]
        whites: f32,
        #[serde(default)]
        blacks: f32,
    },
    Cdl {
        slope: [f32; 3],
        offset: [f32; 3],
        power: [f32; 3],
        #[serde(default = "one")]
        saturation: f32,
    },
    LogWheels {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
        #[serde(default = "low")]
        low: f32,
        #[serde(default = "high")]
        high: f32,
        #[serde(default = "falloff")]
        falloff: f32,
    },
    /// EV range is relative to linear 18% gray.
    HdrZone {
        min_ev: f32,
        max_ev: f32,
        stops: f32,
        #[serde(default)]
        color: [f32; 3],
        #[serde(default = "falloff")]
        falloff: f32,
    },
    Curves {
        channel: CurveChannel,
        points: Vec<[f32; 2]>,
    },
    HueCurves {
        mode: HueCurveMode,
        points: Vec<[f32; 2]>,
    },
    Hsl {
        bands: Vec<HslBand>,
    },
    ColorWarper {
        points: Vec<WarpPoint>,
    },
    RgbMixer {
        matrix: [[f32; 3]; 3],
        #[serde(default)]
        preserve_luminance: bool,
    },
    SplitTone {
        shadows: [f32; 3],
        highlights: [f32; 3],
        #[serde(default)]
        balance: f32,
        #[serde(default = "one")]
        strength: f32,
    },
    Lut {
        path: String,
        #[serde(default = "lut_domain")]
        domain: LutDomain,
        #[serde(default, skip_serializing_if = "crate::lut::is_trilinear")]
        interpolation: crate::lut::LutInterpolation,
    },
    ToneMap {
        method: ToneMap,
        #[serde(default = "one")]
        white: f32,
    },
    GamutCompress {
        #[serde(default = "one")]
        threshold: f32,
        #[serde(default = "softness")]
        softness: f32,
    },
    Blur {
        radius: f32,
    },
    Sharpen {
        radius: f32,
        amount: f32,
        #[serde(default)]
        threshold: f32,
    },
    Clarity {
        radius: f32,
        amount: f32,
    },
    Denoise {
        radius: u32,
        spatial_sigma: f32,
        range_sigma: f32,
    },
    Vignette {
        amount: f32,
        #[serde(default = "center")]
        center: [f32; 2],
        #[serde(default = "one")]
        radius: f32,
        #[serde(default = "falloff")]
        feather: f32,
    },
    Grain {
        amount: f32,
        seed: u64,
        #[serde(default)]
        monochrome: bool,
    },
    Glow {
        radius: f32,
        threshold: f32,
        amount: f32,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Rotate {
        degrees: f32,
    },
    LensDistortion {
        k1: f32,
        #[serde(default)]
        k2: f32,
        #[serde(default = "center")]
        center: [f32; 2],
    },
    /// Copy source region over target; optional feather blends the boundary.
    Clone {
        source: [f32; 2],
        target: [f32; 2],
        radius: f32,
        #[serde(default = "falloff")]
        feather: f32,
    },
    /// Two inputs. First is base; second is foreground, mixed using node mix/mask.
    Blend {
        mode: BlendMode,
    },
}
fn low() -> f32 {
    0.25
}
fn high() -> f32 {
    0.75
}
fn falloff() -> f32 {
    0.2
}
fn softness() -> f32 {
    0.5
}
fn center() -> [f32; 2] {
    [0.5, 0.5]
}
fn lut_domain() -> LutDomain {
    LutDomain::Srgb
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurveChannel {
    Rgb,
    Red,
    Green,
    Blue,
    Luma,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HueCurveMode {
    HueVsHue,
    HueVsSat,
    HueVsLum,
    LumVsSat,
    SatVsSat,
    SatVsLum,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Normal,
    Add,
    Multiply,
    Screen,
    Overlay,
    Difference,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToneMap {
    Reinhard,
    AcesFit,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LutDomain {
    Linear,
    Srgb,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HslBand {
    pub hue: f32,
    pub width: f32,
    #[serde(default)]
    pub hue_shift: f32,
    #[serde(default = "one")]
    pub saturation: f32,
    #[serde(default = "one")]
    pub luminance: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WarpPoint {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub radius: f32,
}

pub fn range(v: f32, min: f32, max: f32, name: &str) -> Result<()> {
    ensure!(
        v.is_finite() && v >= min && v <= max,
        "{name} must be finite in [{min},{max}], got {v}"
    );
    Ok(())
}
fn vec_range(v: [f32; 3], min: f32, max: f32, name: &str) -> Result<()> {
    for x in v {
        range(x, min, max, name)?;
    }
    Ok(())
}
fn points_valid(points: &[[f32; 2]]) -> Result<()> {
    ensure!(
        points.len() >= 2 && points.len() <= 4096,
        "curves require 2..4096 points"
    );
    for p in points {
        range(p[0], 0.0, 1.0, "curve x")?;
        range(p[1], -8.0, 8.0, "curve y")?;
    }
    ensure!(
        points.windows(2).all(|p| p[0][0] < p[1][0]),
        "curve x must be strictly increasing"
    );
    Ok(())
}
impl Operation {
    pub fn changes_dimensions(&self) -> bool {
        matches!(
            self,
            Self::Crop { .. } | Self::Resize { .. } | Self::Rotate { .. }
        )
    }
    pub fn validate(&self) -> Result<()> {
        use Operation::*;
        match self {
            Cutout { options } => options.validate()?,
            OcioGrade { grade } => grade.validate()?,
            Exposure { stops } => range(*stops, -24.0, 24.0, "stops")?,
            WhiteBalance { gains } => vec_range(*gains, 0.001, 100.0, "gains")?,
            Primary {
                exposure,
                lift,
                gamma,
                gain,
                offset,
                contrast,
                pivot,
                saturation,
                vibrance,
            } => {
                range(*exposure, -24.0, 24.0, "exposure")?;
                vec_range(*lift, -4.0, 4.0, "lift")?;
                vec_range(*gamma, 0.01, 10.0, "gamma")?;
                vec_range(*gain, 0.0, 100.0, "gain")?;
                vec_range(*offset, -4.0, 4.0, "offset")?;
                range(*contrast, 0.0, 4.0, "contrast")?;
                range(*pivot, 0.001, 1.0, "pivot")?;
                range(*saturation, 0.0, 4.0, "saturation")?;
                range(*vibrance, -1.0, 1.0, "vibrance")?;
            }
            Tone {
                shadows,
                highlights,
                whites,
                blacks,
            } => {
                for v in [shadows, highlights, whites, blacks] {
                    range(*v, -1.0, 1.0, "tone")?;
                }
            }
            Cdl {
                slope,
                offset,
                power,
                saturation,
            } => {
                vec_range(*slope, 0.0, 100.0, "slope")?;
                vec_range(*offset, -4.0, 4.0, "offset")?;
                vec_range(*power, 0.01, 10.0, "power")?;
                range(*saturation, 0.0, 4.0, "saturation")?;
            }
            LogWheels {
                shadows,
                midtones,
                highlights,
                low,
                high,
                falloff,
            } => {
                for v in [shadows, midtones, highlights] {
                    vec_range(*v, -4.0, 4.0, "wheel")?;
                }
                range(*low, 0.0, 1.0, "low")?;
                range(*high, 0.0, 1.0, "high")?;
                ensure!(low < high, "low must be below high");
                range(*falloff, 0.001, 1.0, "falloff")?;
            }
            HdrZone {
                min_ev,
                max_ev,
                stops,
                color,
                falloff,
            } => {
                range(*min_ev, -32.0, 32.0, "min_ev")?;
                range(*max_ev, -32.0, 32.0, "max_ev")?;
                ensure!(min_ev < max_ev, "invalid EV zone");
                range(*stops, -24.0, 24.0, "stops")?;
                vec_range(*color, -8.0, 8.0, "color")?;
                range(*falloff, 0.001, 8.0, "falloff")?;
            }
            Curves { points, .. } | HueCurves { points, .. } => points_valid(points)?,
            Hsl { bands } => {
                ensure!(bands.len() <= 64, "too many HSL bands");
                for b in bands {
                    range(b.hue, 0.0, 1.0, "hue")?;
                    range(b.width, 0.001, 0.5, "width")?;
                    range(b.hue_shift, -1.0, 1.0, "hue_shift")?;
                    range(b.saturation, 0.0, 4.0, "saturation")?;
                    range(b.luminance, 0.0, 4.0, "luminance")?;
                }
            }
            ColorWarper { points } => {
                ensure!(points.len() <= 256, "too many warp points");
                for p in points {
                    for v in p.from.into_iter().chain(p.to) {
                        range(v, 0.0, 1.0, "warp coordinate")?;
                    }
                    range(p.radius, 0.001, 2.0, "radius")?;
                }
            }
            RgbMixer { matrix, .. } => {
                for row in matrix {
                    vec_range(*row, -10.0, 10.0, "matrix")?;
                }
            }
            SplitTone {
                shadows,
                highlights,
                balance,
                strength,
            } => {
                vec_range(*shadows, -4.0, 4.0, "shadows")?;
                vec_range(*highlights, -4.0, 4.0, "highlights")?;
                range(*balance, -1.0, 1.0, "balance")?;
                range(*strength, 0.0, 4.0, "strength")?;
            }
            Lut { path, .. } => ensure!(!path.is_empty(), "LUT path cannot be empty"),
            ToneMap { white, .. } => range(*white, 0.001, 10000.0, "white")?,
            GamutCompress {
                threshold,
                softness,
            } => {
                range(*threshold, 0.01, 100.0, "threshold")?;
                range(*softness, 0.001, 10.0, "softness")?;
            }
            Blur { radius } => range(*radius, 0.01, 100.0, "radius")?,
            Sharpen {
                radius,
                amount,
                threshold,
            } => {
                range(*radius, 0.01, 100.0, "radius")?;
                range(*amount, 0.0, 10.0, "amount")?;
                range(*threshold, 0.0, 1.0, "threshold")?;
            }
            Clarity { radius, amount } => {
                range(*radius, 0.01, 100.0, "radius")?;
                range(*amount, -2.0, 2.0, "amount")?;
            }
            Denoise {
                radius,
                spatial_sigma,
                range_sigma,
            } => {
                ensure!(*radius > 0 && *radius <= 12, "denoise radius must be 1..12");
                range(*spatial_sigma, 0.01, 100.0, "spatial_sigma")?;
                range(*range_sigma, 0.0001, 10.0, "range_sigma")?;
            }
            Vignette {
                amount,
                center,
                radius,
                feather,
            } => {
                range(*amount, -8.0, 8.0, "amount")?;
                for x in center {
                    range(*x, 0.0, 1.0, "center")?;
                }
                range(*radius, 0.001, 2.0, "radius")?;
                range(*feather, 0.001, 1.0, "feather")?;
            }
            Grain { amount, .. } => range(*amount, 0.0, 1.0, "amount")?,
            Glow {
                radius,
                threshold,
                amount,
            } => {
                range(*radius, 0.01, 100.0, "radius")?;
                range(*threshold, 0.0, 100.0, "threshold")?;
                range(*amount, 0.0, 10.0, "amount")?;
            }
            Crop { width, height, .. } | Resize { width, height } => {
                ensure!(
                    *width > 0 && *height > 0 && *width as u64 * *height as u64 <= 100_000_000,
                    "dimensions must be positive, at most 100M pixels"
                );
            }
            Rotate { degrees } => {
                ensure!(
                    [0.0, 90.0, 180.0, 270.0].contains(&degrees.rem_euclid(360.0)),
                    "rotate currently supports multiples of 90 degrees"
                );
            }
            LensDistortion { k1, k2, center } => {
                range(*k1, -2.0, 2.0, "k1")?;
                range(*k2, -2.0, 2.0, "k2")?;
                for x in center {
                    range(*x, 0.0, 1.0, "center")?;
                }
            }
            Clone {
                source,
                target,
                radius,
                feather,
            } => {
                for v in source.iter().chain(target) {
                    range(*v, 0.0, 1.0, "coordinate")?;
                }
                range(*radius, 0.001, 1.0, "radius")?;
                range(*feather, 0.001, 1.0, "feather")?;
            }
            Blend { .. } => {}
        }
        Ok(())
    }
}
impl Recipe {
    /// All nodes are validated, including disconnected and disabled nodes.
    pub fn validate(&self) -> Result<Vec<String>> {
        ensure!(
            self.schema_version == 1,
            "unsupported recipe schema version {}",
            self.schema_version
        );
        ensure!(
            self.nodes.len() <= 512 && self.masks.len() <= 256,
            "recipe exceeds node/mask limits"
        );
        let mut ids = BTreeSet::new();
        for n in &self.nodes {
            ensure!(
                !n.id.is_empty() && n.id != "source",
                "invalid/reserved node ID"
            );
            ensure!(ids.insert(n.id.clone()), "duplicate node ID {}", n.id);
            n.op.validate()?;
            range(n.mix, 0.0, 1.0, "mix")?;
            let expected = if matches!(n.op, Operation::Blend { .. }) {
                2
            } else {
                1
            };
            ensure!(
                n.inputs.len() == expected,
                "node {} requires {expected} inputs",
                n.id
            );
            if let Some(id) = &n.mask {
                ensure!(self.masks.contains_key(id), "missing mask {id}");
            }
            if n.op.changes_dimensions() {
                ensure!(
                    n.mask.is_none() && n.mix == 1.0,
                    "geometry nodes cannot use masks or partial mix"
                );
            }
        }
        for m in self.masks.values() {
            m.validate(0)?;
        }
        for n in &self.nodes {
            for i in &n.inputs {
                ensure!(i == "source" || ids.contains(i), "missing input {i}");
            }
        }
        ensure!(
            self.output == "source" || ids.contains(&self.output),
            "missing output {}",
            self.output
        );
        let mut done = BTreeSet::from([source()]);
        let mut order = Vec::new();
        loop {
            let before = done.len();
            for n in &self.nodes {
                if !done.contains(&n.id) && n.inputs.iter().all(|i| done.contains(i)) {
                    done.insert(n.id.clone());
                    order.push(n.id.clone());
                }
            }
            if done.len() == self.nodes.len() + 1 {
                break;
            }
            if done.len() == before {
                bail!("node graph contains a cycle");
            }
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_rejects_cycles_missing_inputs_and_typos() {
        assert!(
            serde_json::from_str::<Operation>(r#"{"type":"exposure","stops":1,"stop":2}"#).is_err()
        );
        let mut r = Recipe::default();
        r.nodes.push(Node {
            id: "a".into(),
            inputs: vec!["a".into()],
            op: Operation::Exposure { stops: 0.0 },
            mask: None,
            mix: 1.0,
            enabled: true,
        });
        r.output = "a".into();
        assert!(r.validate().is_err());
        r.nodes[0].inputs = vec!["missing".into()];
        assert!(r.validate().is_err());
        r.nodes[0].inputs = vec!["source".into()];
        assert_eq!(r.validate().unwrap(), ["a"]);
    }
}
