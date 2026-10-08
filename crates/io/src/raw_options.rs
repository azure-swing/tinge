//! Versioned sensor development controls, independent of the optional RAW backend.
use anyhow::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SensorLevels {
    /// [columns, rows], anchored at the uncropped sensor origin.
    pub repeat: [usize; 2],
    /// Row-major cells with interleaved components per pixel; native sensor units.
    pub values: Vec<f32>,
}
impl SensorLevels {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.repeat.iter().all(|v| (1..=16).contains(v)),
            "sensor level repeat dimensions must be 1..16"
        );
        ensure!(
            !self.values.is_empty()
                && self.values.len() <= 1024
                && self.values.iter().all(|v| v.is_finite()),
            "sensor levels require 1..1024 finite values"
        );
        Ok(())
    }
    pub fn validate_components(&self, cpp: usize) -> Result<()> {
        self.validate()?;
        ensure!(
            self.values.len() == self.repeat[0] * self.repeat[1] * cpp,
            "sensor level count must equal repeat columns * rows * components"
        );
        Ok(())
    }
    pub fn at(&self, x: usize, y: usize, channel: usize, cpp: usize) -> f32 {
        self.values[((y % self.repeat[1]) * self.repeat[0] + x % self.repeat[0]) * cpp + channel]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RawWhiteBalance {
    /// Use camera metadata; report an explicit D65 fallback when absent.
    #[default]
    AsShot,
    Unity,
    /// Camera plane order, normalized to plane 1. Three or four positive gains.
    CameraGains {
        gains: Vec<f32>,
    },
    /// CIE 1931 xy white; computed with the selected D65-adapted camera matrix.
    Chromaticity {
        xy: [f32; 2],
    },
    /// Kang 2002 Planckian-locus fit with signed normal offset in CIE 1960 uv.
    /// Positive Duv describes a greener source white, compensated by camera gains.
    Temperature {
        kelvin: f64,
        #[serde(default)]
        tint_duv: f64,
    },
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RawCrop {
    Sensor,
    Active,
    #[default]
    Default,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BelowBlack {
    #[default]
    Clip,
    Preserve,
}
fn version() -> u32 {
    1
}
fn yes() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RawDevelopOptions {
    /// Version 1 applies sensor normalization and WB before demosaicing.
    #[serde(default = "version")]
    pub algorithm_version: u32,
    #[serde(default)]
    pub white_balance: RawWhiteBalance,
    /// Applied after demosaicing and camera conversion; does not change sensor saturation.
    #[serde(default)]
    pub exposure_ev: f32,
    #[serde(default)]
    pub black_levels: Option<SensorLevels>,
    #[serde(default)]
    pub white_levels: Option<SensorLevels>,
    #[serde(default)]
    pub below_black: BelowBlack,
    #[serde(default)]
    pub crop: RawCrop,
    #[serde(default = "yes")]
    pub apply_orientation: bool,
    /// DNG/EXIF illuminant numeric tag, selecting a decoder-supplied camera matrix.
    #[serde(default)]
    pub calibration_illuminant: Option<u16>,
}
impl Default for RawDevelopOptions {
    fn default() -> Self {
        Self {
            algorithm_version: 1,
            white_balance: Default::default(),
            exposure_ev: 0.0,
            black_levels: None,
            white_levels: None,
            below_black: Default::default(),
            crop: Default::default(),
            apply_orientation: true,
            calibration_illuminant: None,
        }
    }
}
impl RawDevelopOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.algorithm_version == 1,
            "unsupported RAW algorithm_version"
        );
        ensure!(
            self.exposure_ev.is_finite() && (-20.0..=20.0).contains(&self.exposure_ev),
            "RAW exposure_ev must be finite and in -20..20"
        );
        if let Some(tag) = self.calibration_illuminant {
            // rawler's Bradford helper panics for illuminants without a defined white.
            ensure!(
                [1, 4, 17, 18, 19, 20, 21, 22, 23].contains(&tag),
                "RAW calibration illuminant has no supported reference white point"
            );
        }
        match &self.white_balance {
            RawWhiteBalance::CameraGains { gains } => ensure!(
                [3, 4].contains(&gains.len())
                    && gains
                        .iter()
                        .all(|v| v.is_finite() && (0.0001..=10000.0).contains(v)),
                "RAW camera gains require 3 or 4 finite positive values in 0.0001..10000"
            ),
            RawWhiteBalance::Chromaticity { xy } => ensure!(
                xy.iter().all(|v| v.is_finite() && *v > 0.0) && xy[0] + xy[1] < 1.0,
                "RAW white xy must be positive with x+y < 1"
            ),
            RawWhiteBalance::Temperature { kelvin, tint_duv } => {
                temperature_white_xy(*kelvin, *tint_duv)?;
            }
            _ => {}
        }
        for levels in [&self.black_levels, &self.white_levels]
            .into_iter()
            .flatten()
        {
            levels.validate()?;
        }
        Ok(())
    }
}

/// CCT is limited to the published Kang 2002 fit domain; D65 is not 6504K/Duv=0.
/// Duv is a geometric normal displacement from that approximate fitted locus,
/// not the Lightroom Tint slider or an exact spectral/Robertson CCT solution.
pub fn temperature_white_xy(kelvin: f64, tint_duv: f64) -> Result<[f64; 2]> {
    ensure!(
        kelvin.is_finite() && (1667.0..=25000.0).contains(&kelvin),
        "RAW Kelvin must be finite and in 1667..25000"
    );
    ensure!(
        tint_duv.is_finite() && (-0.05..=0.05).contains(&tint_duv),
        "RAW tint_duv must be finite and in -0.05..0.05 (CIE 1960 uv units)"
    );
    let [a, b, c, d] = if kelvin <= 4000.0 {
        [-0.2661239e9, -0.2343589e6, 0.8776956e3, 0.179910]
    } else {
        [-3.0258469e9, 2.1070379e6, 0.2226347e3, 0.240390]
    };
    let t = kelvin.recip();
    let x = a * t.powi(3) + b * t.powi(2) + c * t + d;
    let dx = -3.0 * a * t.powi(4) - 2.0 * b * t.powi(3) - c * t.powi(2);
    let [a, b, c, d] = if kelvin <= 2222.0 {
        [-1.1063814, -1.34811020, 2.18555832, -0.20219683]
    } else if kelvin <= 4000.0 {
        [-0.9549476, -1.37418593, 2.09137015, -0.16748867]
    } else {
        [3.0817580, -5.87338670, 3.75112997, -0.37001483]
    };
    let y = a * x.powi(3) + b * x.powi(2) + c * x + d;
    let dy = (3.0 * a * x.powi(2) + 2.0 * b * x + c) * dx;
    let denom = -2.0 * x + 12.0 * y + 3.0;
    let denom_d = -2.0 * dx + 12.0 * dy;
    let du = 4.0 * (dx * denom - x * denom_d) / denom.powi(2);
    let dv = 6.0 * (dy * denom - y * denom_d) / denom.powi(2);
    let length = du.hypot(dv);
    ensure!(
        length.is_finite() && length > 0.0,
        "undefined Planckian-locus tangent"
    );
    let mut normal = [-dv / length, du / length];
    if normal[1] < 0.0 {
        normal = normal.map(|v| -v);
    }
    let u = 4.0 * x / denom + tint_duv * normal[0];
    let v = 6.0 * y / denom + tint_duv * normal[1];
    let denom = 2.0 * u - 8.0 * v + 4.0;
    let xy = [3.0 * u / denom, 2.0 * v / denom];
    ensure!(
        xy.iter().all(|v| v.is_finite() && *v > 0.0) && xy[0] + xy[1] < 1.0,
        "requested temperature/tint white is outside positive XYZ chromaticities"
    );
    Ok(xy)
}
