use crate::{Frame, recipe::range};
use anyhow::{Result, ensure};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tinge_color::{luminance, rgb_to_hsv, to_display};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mask {
    Ellipse {
        center: [f32; 2],
        radius: [f32; 2],
        #[serde(default)]
        rotation: f32,
        feather: f32,
    },
    Rectangle {
        min: [f32; 2],
        max: [f32; 2],
        feather: f32,
    },
    Polygon {
        points: Vec<[f32; 2]>,
        feather: f32,
    },
    LinearGradient {
        start: [f32; 2],
        end: [f32; 2],
    },
    Brush {
        points: Vec<[f32; 2]>,
        radius: f32,
        hardness: f32,
    },
    LumaRange {
        min: f32,
        max: f32,
        softness: f32,
    },
    HslRange {
        hue: f32,
        hue_width: f32,
        saturation: [f32; 2],
        luminance: [f32; 2],
        softness: f32,
    },
    /// Imported gray/alpha matte. Path is relative to recipe/project directory.
    Bitmap {
        path: String,
    },
    Combine {
        mode: CombineMode,
        masks: Vec<Mask>,
    },
    Invert {
        mask: Box<Mask>,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CombineMode {
    Union,
    Intersect,
    Subtract,
}
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-8)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
pub fn band(x: f32, min: f32, max: f32, soft: f32) -> f32 {
    smooth(min - soft, min, x) * (1.0 - smooth(max, max + soft, x))
}
pub fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs().rem_euclid(1.0);
    d.min(1.0 - d)
}
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let v = [b[0] - a[0], b[1] - a[1]];
    let len = v[0] * v[0] + v[1] * v[1];
    let t = if len < 1e-12 {
        0.0
    } else {
        ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len
    }
    .clamp(0.0, 1.0);
    ((p[0] - a[0] - t * v[0]).powi(2) + (p[1] - a[1] - t * v[1]).powi(2)).sqrt()
}
impl Mask {
    pub fn validate(&self, depth: usize) -> Result<()> {
        ensure!(depth <= 16, "mask nesting exceeds 16");
        let coord = |p: &[f32; 2]| -> Result<()> {
            for x in p {
                range(*x, 0.0, 1.0, "mask coordinate")?;
            }
            Ok(())
        };
        let feather = |f: f32| range(f, 0.0, 1.0, "feather");
        match self {
            Self::Ellipse {
                center,
                radius,
                rotation,
                feather: f,
            } => {
                coord(center)?;
                for x in radius {
                    range(*x, 0.00001, 2.0, "mask radius")?;
                }
                range(*rotation, -360.0, 360.0, "rotation")?;
                feather(*f)?;
            }
            Self::Rectangle {
                min,
                max,
                feather: f,
            } => {
                coord(min)?;
                coord(max)?;
                ensure!(min[0] < max[0] && min[1] < max[1], "invalid rectangle");
                feather(*f)?;
            }
            Self::Polygon { points, feather: f } => {
                ensure!(
                    points.len() >= 3 && points.len() <= 4096,
                    "polygon requires 3..4096 points"
                );
                for p in points {
                    coord(p)?;
                }
                feather(*f)?;
            }
            Self::LinearGradient { start, end } => {
                coord(start)?;
                coord(end)?;
                ensure!(start != end, "gradient start must differ from end");
            }
            Self::Brush {
                points,
                radius,
                hardness,
            } => {
                ensure!(
                    !points.is_empty() && points.len() <= 4096,
                    "brush needs 1..4096 points"
                );
                for p in points {
                    coord(p)?;
                }
                range(*radius, 0.00001, 2.0, "radius")?;
                range(*hardness, 0.0, 1.0, "hardness")?;
            }
            Self::LumaRange { min, max, softness } => {
                range(*min, 0.0, 10000.0, "luma min")?;
                range(*max, 0.0, 10000.0, "luma max")?;
                ensure!(min < max, "invalid luminance range");
                range(*softness, 0.00001, 100.0, "softness")?;
            }
            Self::HslRange {
                hue,
                hue_width,
                saturation,
                luminance,
                softness,
            } => {
                range(*hue, 0.0, 1.0, "hue")?;
                range(*hue_width, 0.00001, 0.5, "hue_width")?;
                for r in [saturation, luminance] {
                    for v in r {
                        range(*v, 0.0, 1.0, "range")?;
                    }
                    ensure!(r[0] <= r[1], "invalid HSL range");
                }
                range(*softness, 0.00001, 1.0, "softness")?;
            }
            Self::Bitmap { path } => ensure!(!path.is_empty(), "empty bitmap path"),
            Self::Combine { masks, .. } => {
                ensure!(
                    !masks.is_empty() && masks.len() <= 64,
                    "combine needs 1..64 masks"
                );
                for m in masks {
                    m.validate(depth + 1)?;
                }
            }
            Self::Invert { mask } => mask.validate(depth + 1)?,
        }
        Ok(())
    }
    /// `load` returns bitmap values at the node's dimensions; performed once by engine.
    pub fn rasterize(
        &self,
        frame: &Frame,
        load: &impl Fn(&str, u32, u32) -> Result<Vec<f32>>,
    ) -> Result<Vec<f32>> {
        self.rasterize_cancellable(frame, load, &AtomicBool::new(false))
    }
    pub fn rasterize_cancellable(
        &self,
        frame: &Frame,
        load: &impl Fn(&str, u32, u32) -> Result<Vec<f32>>,
        cancel: &AtomicBool,
    ) -> Result<Vec<f32>> {
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        match self {
            Self::Bitmap { path } => load(path, frame.width, frame.height),
            Self::Invert { mask } => mask
                .rasterize_cancellable(frame, load, cancel)?
                .into_par_iter()
                .map(|v| {
                    ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
                    Ok(1.0 - v)
                })
                .collect(),
            Self::Combine { mode, masks } => {
                let mut out = masks[0].rasterize_cancellable(frame, load, cancel)?;
                for m in &masks[1..] {
                    let b = m.rasterize_cancellable(frame, load, cancel)?;
                    out.par_iter_mut()
                        .zip(b.par_iter())
                        .try_for_each(|(a, b)| -> Result<()> {
                            ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
                            *a = match mode {
                                CombineMode::Union => 1.0 - (1.0 - *a) * (1.0 - b),
                                CombineMode::Intersect => *a * b,
                                CombineMode::Subtract => *a * (1.0 - b),
                            };
                            Ok(())
                        })?;
                }
                Ok(out)
            }
            _ => frame
                .pixels
                .par_iter()
                .enumerate()
                .map(|(i, p)| {
                    if i % 1024 == 0 {
                        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
                    }
                    let x = (i % frame.width as usize) as f32 + 0.5;
                    let y = (i / frame.width as usize) as f32 + 0.5;
                    let xy = [x / frame.width as f32, y / frame.height as f32];
                    let rgb = [p[0], p[1], p[2]];
                    Ok(match self {
                        Self::Ellipse {
                            center,
                            radius,
                            rotation,
                            feather,
                        } => {
                            let dx = xy[0] - center[0];
                            let dy = xy[1] - center[1];
                            let a = rotation.to_radians();
                            let r = (((dx * a.cos() + dy * a.sin()) / radius[0]).powi(2)
                                + ((-dx * a.sin() + dy * a.cos()) / radius[1]).powi(2))
                            .sqrt();
                            if *feather == 0.0 {
                                if r <= 1.0 { 1.0 } else { 0.0 }
                            } else {
                                1.0 - smooth(1.0 - *feather, 1.0, r)
                            }
                        }
                        Self::Rectangle { min, max, feather } => {
                            if xy[0] < min[0] || xy[0] > max[0] || xy[1] < min[1] || xy[1] > max[1]
                            {
                                0.0
                            } else if *feather == 0.0 {
                                1.0
                            } else {
                                smooth(
                                    0.0,
                                    *feather,
                                    (xy[0] - min[0])
                                        .min(max[0] - xy[0])
                                        .min(xy[1] - min[1])
                                        .min(max[1] - xy[1]),
                                )
                            }
                        }
                        Self::Polygon { points, feather } => {
                            let mut inside = false;
                            let mut dist = f32::MAX;
                            for j in 0..points.len() {
                                let a = points[j];
                                let b = points[(j + 1) % points.len()];
                                if (a[1] > xy[1]) != (b[1] > xy[1])
                                    && xy[0] < (b[0] - a[0]) * (xy[1] - a[1]) / (b[1] - a[1]) + a[0]
                                {
                                    inside = !inside;
                                }
                                dist = dist.min(segment_distance(xy, a, b));
                            }
                            if !inside {
                                0.0
                            } else if *feather == 0.0 {
                                1.0
                            } else {
                                smooth(0.0, *feather, dist)
                            }
                        }
                        Self::LinearGradient { start, end } => {
                            let v = [end[0] - start[0], end[1] - start[1]];
                            (((xy[0] - start[0]) * v[0] + (xy[1] - start[1]) * v[1])
                                / (v[0] * v[0] + v[1] * v[1]))
                                .clamp(0.0, 1.0)
                        }
                        Self::Brush {
                            points,
                            radius,
                            hardness,
                        } => {
                            let dist = if points.len() == 1 {
                                segment_distance(xy, points[0], points[0])
                            } else {
                                points
                                    .windows(2)
                                    .map(|p| segment_distance(xy, p[0], p[1]))
                                    .fold(f32::MAX, f32::min)
                            };
                            if *hardness == 1.0 {
                                if dist <= *radius { 1.0 } else { 0.0 }
                            } else {
                                1.0 - smooth(radius * hardness, *radius, dist)
                            }
                        }
                        Self::LumaRange { min, max, softness } => {
                            band(luminance(rgb), *min, *max, *softness)
                        }
                        Self::HslRange {
                            hue,
                            hue_width,
                            saturation,
                            luminance,
                            softness,
                        } => {
                            let enc = to_display(rgb);
                            let [h, _, v] = rgb_to_hsv(enc);
                            let min = enc.into_iter().fold(f32::MAX, f32::min);
                            let l = (v + min) * 0.5;
                            let s = if v - min < 1e-8 {
                                0.0
                            } else {
                                (v - min) / (1.0 - (2.0 * l - 1.0).abs()).max(1e-8)
                            };
                            (1.0 - smooth(*hue_width, *hue_width + softness, hue_distance(h, *hue)))
                                * band(s, saturation[0], saturation[1], *softness)
                                * band(l, luminance[0], luminance[1], *softness)
                        }
                        _ => unreachable!(),
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gradient_and_mask_algebra() {
        let f = Frame::new(4, 1, vec![[0.5, 0.5, 0.5, 1.0]; 4]).unwrap();
        let load = |_: &str, _: u32, _: u32| unreachable!();
        let m = Mask::LinearGradient {
            start: [0.0, 0.5],
            end: [1.0, 0.5],
        };
        assert_eq!(
            m.rasterize(&f, &load).unwrap(),
            [0.125, 0.375, 0.625, 0.875]
        );
        let inv = Mask::Invert {
            mask: Box::new(m.clone()),
        };
        let both = Mask::Combine {
            mode: CombineMode::Union,
            masks: vec![m, inv],
        };
        assert!((both.rasterize(&f, &load).unwrap()[0] - 0.890625).abs() < 1e-6);
    }
}
