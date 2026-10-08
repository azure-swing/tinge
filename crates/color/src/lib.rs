//! Explicit RGB colorimetry. Real OCIO lives in the separate vibecolor-ocio crate.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Primaries {
    #[default]
    Srgb,
    DisplayP3,
    Rec2020,
    AcesCg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transfer {
    Linear,
    #[default]
    Srgb,
    Gamma22,
    Gamma24,
    /// ST 2084, normalized to 10,000 nits. Linear 1 = 10,000 nits.
    Pq,
    /// Scene-referred HLG OETF only; no display OOTF.
    Hlg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ColorSpace {
    #[serde(default)]
    pub primaries: Primaries,
    #[serde(default)]
    pub transfer: Transfer,
}

pub fn decode(x: f32, t: Transfer) -> f32 {
    let a = x.abs();
    let v = match t {
        Transfer::Linear => a,
        Transfer::Srgb => {
            if a <= 0.04045 {
                a / 12.92
            } else {
                ((a + 0.055) / 1.055).powf(2.4)
            }
        }
        Transfer::Gamma22 => a.powf(2.2),
        Transfer::Gamma24 => a.powf(2.4),
        Transfer::Pq => {
            // f64 avoids amplified cancellation in the PQ denominator near HDR white.
            let p = (a as f64).powf(1.0 / (2523.0 / 32.0));
            ((p - 3424.0 / 4096.0).max(0.0) / (2413.0 / 128.0 - 2392.0 / 128.0 * p).max(1e-8))
                .powf(1.0 / (2610.0 / 16384.0)) as f32
        }
        Transfer::Hlg => {
            if a <= 0.5 {
                a * a / 3.0
            } else {
                (((a - 0.559_910_7) / 0.178_832_77).exp() + 0.284_668_92) / 12.0
            }
        }
    };
    x.signum() * v
}

pub fn encode(x: f32, t: Transfer) -> f32 {
    let a = x.abs();
    let v = match t {
        Transfer::Linear => a,
        Transfer::Srgb => {
            if a <= 0.0031308 {
                a * 12.92
            } else {
                1.055 * a.powf(1.0 / 2.4) - 0.055
            }
        }
        Transfer::Gamma22 => a.powf(1.0 / 2.2),
        Transfer::Gamma24 => a.powf(1.0 / 2.4),
        Transfer::Pq => {
            let p = (a as f64).powf(2610.0 / 16384.0);
            ((3424.0 / 4096.0 + 2413.0 / 128.0 * p) / (1.0 + 2392.0 / 128.0 * p))
                .powf(2523.0 / 32.0) as f32
        }
        Transfer::Hlg => {
            if a <= 1.0 / 12.0 {
                (3.0 * a).sqrt()
            } else {
                0.178_832_77 * (12.0 * a - 0.284_668_92).ln() + 0.559_910_7
            }
        }
    };
    x.signum() * v
}

type Matrix = [[f32; 3]; 3];
fn xyz(p: Primaries) -> Matrix {
    match p {
        Primaries::Srgb => [
            [0.4123908, 0.35758433, 0.1804808],
            [0.212639, 0.71516865, 0.07219231],
            [0.01933082, 0.11919478, 0.95053214],
        ],
        Primaries::DisplayP3 => [
            [0.48657095, 0.2656677, 0.19821729],
            [0.22897456, 0.69173855, 0.07928691],
            [0.0, 0.04511338, 1.0439444],
        ],
        Primaries::Rec2020 => [
            [0.636958, 0.1446169, 0.16888098],
            [0.2627002, 0.67799807, 0.05930172],
            [0.0, 0.02807269, 1.0609851],
        ],
        // AP1 adapted from D60 to D65 using Bradford.
        Primaries::AcesCg => [
            [0.65223754, 0.12823613, 0.16998225],
            [0.26767218, 0.674339, 0.05798885],
            [-0.005381815, 0.001369061, 1.0930704],
        ],
    }
}
fn inverse(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [
            (e * i - f * h) / det,
            (c * h - b * i) / det,
            (b * f - c * e) / det,
        ],
        [
            (f * g - d * i) / det,
            (a * i - c * g) / det,
            (c * d - a * f) / det,
        ],
        [
            (d * h - e * g) / det,
            (b * g - a * h) / det,
            (a * e - b * d) / det,
        ],
    ]
}
pub fn mul(m: Matrix, x: [f32; 3]) -> [f32; 3] {
    m.map(|r| r[0] * x[0] + r[1] * x[1] + r[2] * x[2])
}
pub fn convert_linear(x: [f32; 3], from: Primaries, to: Primaries) -> [f32; 3] {
    if from == to {
        x
    } else {
        // Only round once after both matrices. An intermediate f32 XYZ value
        // amplifies cancellation for saturated HDR colors and near-black PQ.
        let from = xyz(from).map(|row| row.map(f64::from));
        let to = inverse(xyz(to).map(|row| row.map(f64::from)));
        let xyz = from.map(|r| r[0] * x[0] as f64 + r[1] * x[1] as f64 + r[2] * x[2] as f64);
        to.map(|r| (r[0] * xyz[0] + r[1] * xyz[1] + r[2] * xyz[2]) as f32)
    }
}
pub fn luminance(x: [f32; 3]) -> f32 {
    x[0] * 0.212639 + x[1] * 0.71516865 + x[2] * 0.07219231
}
pub fn to_display(x: [f32; 3]) -> [f32; 3] {
    x.map(|v| encode(v, Transfer::Srgb))
}
pub fn from_display(x: [f32; 3]) -> [f32; 3] {
    x.map(|v| decode(v, Transfer::Srgb))
}
pub fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 1e-8 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    [h, if max <= 1e-8 { 0.0 } else { d / max }, max]
}
pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let r = match h as u32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    r.map(|a| a + m)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_vectors_and_roundtrip() {
        assert!((decode(0.04045, Transfer::Srgb) - 0.0031308).abs() < 1e-6);
        assert!((encode(0.01, Transfer::Pq) - 0.5080784).abs() < 2e-5);
        for t in [
            Transfer::Linear,
            Transfer::Srgb,
            Transfer::Gamma22,
            Transfer::Gamma24,
            Transfer::Pq,
            Transfer::Hlg,
        ] {
            for x in [0.0, 0.001, 0.18, 1.0] {
                assert!((decode(encode(x, t), t) - x).abs() < 0.0001, "{t:?} {x}");
            }
        }
    }
    #[test]
    fn primaries_white_and_inverse() {
        for p in [
            Primaries::Srgb,
            Primaries::DisplayP3,
            Primaries::Rec2020,
            Primaries::AcesCg,
        ] {
            let white = convert_linear([1.0; 3], p, Primaries::Srgb);
            assert!(
                white.iter().all(|v| (v - 1.0).abs() < 1e-5),
                "{p:?}: {white:?}"
            );
            let x = [0.1, 0.5, 1.5];
            let y = convert_linear(convert_linear(x, p, Primaries::Srgb), Primaries::Srgb, p);
            assert!((0..3).all(|c| (x[c] - y[c]).abs() < 1e-5));
        }
    }
}
