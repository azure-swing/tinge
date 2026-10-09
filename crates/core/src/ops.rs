use crate::{
    Frame, Operation,
    lut::CubeLut,
    mask::{band, hue_distance, smooth},
    recipe::*,
};
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tinge_color::{from_display, hsv_to_rgb, luminance, mul, rgb_to_hsv, to_display};

fn saturation(rgb: [f32; 3], amount: f32) -> [f32; 3] {
    let l = luminance(rgb);
    rgb.map(|v| l + (v - l) * amount)
}
fn signed_pow(x: f32, p: f32) -> f32 {
    x.signum() * x.abs().powf(p)
}
fn curve(points: &[[f32; 2]], x: f32) -> f32 {
    if x <= points[0][0] {
        return points[0][1] + x - points[0][0];
    }
    let last = points[points.len() - 1];
    if x >= last[0] {
        return last[1] + x - last[0];
    }
    let j = points.partition_point(|p| p[0] < x);
    let a = points[j - 1];
    let b = points[j];
    let t = (x - a[0]) / (b[0] - a[0]);
    a[1] + (b[1] - a[1]) * t
}
pub fn blur(frame: &Frame, radius: f32) -> Frame {
    let size = (radius * 3.0).ceil() as i32;
    let mut kernel: Vec<f32> = (-size..=size)
        .map(|i| (-(i * i) as f32 / (2.0 * radius * radius)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    for k in &mut kernel {
        *k /= sum;
    }
    let pass = |src: &Frame, horizontal: bool| -> Frame {
        let pixels = (0..src.pixels.len())
            .into_par_iter()
            .map(|i| {
                let x = (i % src.width as usize) as i32;
                let y = (i / src.width as usize) as i32;
                let mut out = [0.0; 4];
                for (j, k) in kernel.iter().enumerate() {
                    let off = j as i32 - size;
                    let px = (x + if horizontal { off } else { 0 }).clamp(0, src.width as i32 - 1)
                        as u32;
                    let py = (y + if horizontal { 0 } else { off }).clamp(0, src.height as i32 - 1)
                        as u32;
                    let v = src.pixels[(py * src.width + px) as usize];
                    // Blur premultiplied RGB; unpremultiply once after each pass.
                    for c in 0..3 {
                        out[c] += v[c] * v[3] * k;
                    }
                    out[3] += v[3] * k;
                }
                if out[3] > 1e-8 {
                    for c in 0..3 {
                        out[c] /= out[3];
                    }
                }
                out
            })
            .collect();
        Frame {
            width: src.width,
            height: src.height,
            pixels,
        }
    };
    pass(&pass(frame, true), false)
}
fn noise(seed: u64, index: usize, channel: usize) -> f32 {
    let mut x = seed
        .wrapping_add((index as u64).wrapping_mul(0x9e3779b97f4a7c15))
        .wrapping_add((channel as u64).wrapping_mul(0x632be59bd9b4e019));
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    ((x >> 40) as f32 / 16_777_215.0 - 0.5) * 2.0
}

/// Local operations run at native resolution. Preview resizing happens after render.
/// Pointwise color nodes check cancellation between small pixel blocks. Spatial
/// operators retain their native coordinates and existing execution path.
pub fn apply_cancellable(
    op: &Operation,
    inputs: &[&Frame],
    read_lut: &(impl Fn(&str) -> Result<CubeLut> + Sync),
    cancel: &AtomicBool,
) -> Result<Frame> {
    ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
    use Operation::*;
    let pointwise = matches!(
        op,
        Exposure { .. }
            | WhiteBalance { .. }
            | Primary { .. }
            | Tone { .. }
            | Cdl { .. }
            | LogWheels { .. }
            | HdrZone { .. }
            | Curves { .. }
            | HueCurves { .. }
            | Hsl { .. }
            | ColorWarper { .. }
            | RgbMixer { .. }
            | SplitTone { .. }
            | ToneMap { .. }
            | GamutCompress { .. }
    );
    if !pointwise {
        return apply(op, inputs, read_lut);
    }
    op.validate()?;
    let input = inputs.first().context("operation has no input")?;
    let mut out = Frame {
        width: input.width,
        height: input.height,
        pixels: vec![[0.0; 4]; input.pixels.len()],
    };
    out.pixels
        .par_chunks_mut(16384)
        .enumerate()
        .try_for_each(|(index, pixels)| -> Result<()> {
            ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
            let begin = index * 16384;
            let block = Frame {
                width: pixels.len() as u32,
                height: 1,
                pixels: input.pixels[begin..begin + pixels.len()].to_vec(),
            };
            let result = apply(op, &[&block], read_lut)?;
            ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
            pixels.copy_from_slice(&result.pixels);
            Ok(())
        })?;
    ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
    Ok(out)
}

pub fn apply(
    op: &Operation,
    inputs: &[&Frame],
    read_lut: &impl Fn(&str) -> Result<CubeLut>,
) -> Result<Frame> {
    op.validate()?;
    let f = inputs.first().context("operation has no input")?;
    use Operation::*;
    let out = match op {
        Cutout { .. } => anyhow::bail!("cutout requires engine matte loader"),
        OcioGrade { .. } => anyhow::bail!("OCIO grade requires engine color_pipeline"),
        Exposure { stops } => f.map_rgb(|rgb, _| rgb.map(|v| v * 2f32.powf(*stops))),
        WhiteBalance { gains } => f.map_rgb(|rgb, _| std::array::from_fn(|c| rgb[c] * gains[c])),
        Primary {
            exposure,
            lift,
            gamma,
            gain,
            offset,
            contrast,
            pivot,
            saturation: sat,
            vibrance,
        } => f.map_rgb(|rgb, _| {
            let rgb = rgb.map(|v| v * 2f32.powf(*exposure));
            let enc = to_display(rgb);
            let enc = std::array::from_fn(|c| {
                signed_pow(enc[c] + lift[c] * (1.0 - enc[c]), 1.0 / gamma[c]) * gain[c] + offset[c]
            });
            let mut rgb = from_display(enc);
            // Contrast around a scene-linear pivot using a signed power curve.
            rgb = rgb.map(|v| pivot * signed_pow(v / pivot, *contrast));
            let hsv = rgb_to_hsv(to_display(rgb));
            saturation(
                rgb,
                *sat * (1.0 + vibrance * (1.0 - hsv[1].clamp(0.0, 1.0))),
            )
        }),
        Tone {
            shadows,
            highlights,
            whites,
            blacks,
        } => f.map_rgb(|rgb, _| {
            let l = luminance(rgb).max(0.0);
            let d = to_display([l; 3])[0];
            let ev = shadows * (1.0 - smooth(0.0, 0.65, d))
                + highlights * smooth(0.35, 1.0, d)
                + blacks * (1.0 - smooth(0.0, 0.25, d))
                + whites * smooth(0.75, 1.0, d);
            rgb.map(|v| v * 2f32.powf(ev * 2.0))
        }),
        Cdl {
            slope,
            offset,
            power,
            saturation: sat,
        } => f.map_rgb(|rgb, _| {
            let enc = to_display(rgb);
            let enc =
                std::array::from_fn(|c| (enc[c] * slope[c] + offset[c]).max(0.0).powf(power[c]));
            // CDL saturation acts in the same encoded domain as SOP.
            from_display(saturation(enc, *sat))
        }),
        LogWheels {
            shadows,
            midtones,
            highlights,
            low,
            high,
            falloff,
        } => f.map_rgb(|rgb, _| {
            let enc = to_display(rgb);
            let l = luminance(enc);
            let s = 1.0 - smooth(low - falloff, low + falloff, l);
            let h = smooth(high - falloff, high + falloff, l);
            let m = (1.0 - s - h).max(0.0);
            from_display(std::array::from_fn(|c| {
                enc[c] + shadows[c] * s + midtones[c] * m + highlights[c] * h
            }))
        }),
        HdrZone {
            min_ev,
            max_ev,
            stops,
            color,
            falloff,
        } => f.map_rgb(|rgb, _| {
            let ev = (luminance(rgb).max(1e-10) / 0.18).log2();
            let w = band(ev, *min_ev, *max_ev, *falloff);
            std::array::from_fn(|c| rgb[c] * 2f32.powf(w * (stops + color[c])))
        }),
        Curves { channel, points } => f.map_rgb(|rgb, _| {
            let mut enc = to_display(rgb);
            match channel {
                CurveChannel::Rgb => enc = enc.map(|v| curve(points, v)),
                CurveChannel::Red => enc[0] = curve(points, enc[0]),
                CurveChannel::Green => enc[1] = curve(points, enc[1]),
                CurveChannel::Blue => enc[2] = curve(points, enc[2]),
                CurveChannel::Luma => {
                    let l = luminance(enc);
                    let target = curve(points, l);
                    enc = enc.map(|v| v + target - l);
                }
            }
            from_display(enc)
        }),
        HueCurves { mode, points } => f.map_rgb(|rgb, _| {
            let mut hsv = rgb_to_hsv(to_display(rgb));
            let x = match mode {
                HueCurveMode::LumVsSat => hsv[2],
                HueCurveMode::SatVsSat | HueCurveMode::SatVsLum => hsv[1],
                _ => hsv[0],
            };
            let v = curve(points, x);
            match mode {
                HueCurveMode::HueVsHue => hsv[0] += v,
                HueCurveMode::HueVsSat | HueCurveMode::LumVsSat | HueCurveMode::SatVsSat => {
                    hsv[1] = (hsv[1] * v).clamp(0.0, 1.0)
                }
                HueCurveMode::HueVsLum | HueCurveMode::SatVsLum => hsv[2] *= v.max(0.0),
            }
            from_display(hsv_to_rgb(hsv))
        }),
        Hsl { bands } => f.map_rgb(|rgb, _| {
            let mut hsv = rgb_to_hsv(to_display(rgb));
            let original = hsv;
            for b in bands {
                let w = 1.0 - smooth(0.0, b.width, hue_distance(original[0], b.hue));
                hsv[0] += w * b.hue_shift;
                hsv[1] *= 1.0 + w * (b.saturation - 1.0);
                hsv[2] *= 1.0 + w * (b.luminance - 1.0);
            }
            hsv[1] = hsv[1].clamp(0.0, 1.0);
            from_display(hsv_to_rgb(hsv))
        }),
        ColorWarper { points } => f.map_rgb(|rgb, _| {
            let mut hsv = rgb_to_hsv(to_display(rgb));
            let original = hsv;
            let mut delta = [0.0; 2];
            let mut total = 0.0;
            for p in points {
                let d = (hue_distance(original[0], p.from[0]).powi(2)
                    + (original[1] - p.from[1]).powi(2))
                .sqrt();
                let w = 1.0 - smooth(0.0, p.radius, d);
                let dh = (p.to[0] - p.from[0] + 0.5).rem_euclid(1.0) - 0.5;
                delta[0] += dh * w;
                delta[1] += (p.to[1] - p.from[1]) * w;
                total += w;
            }
            hsv[0] += delta[0] / total.max(1.0);
            hsv[1] = (hsv[1] + delta[1] / total.max(1.0)).clamp(0.0, 1.0);
            from_display(hsv_to_rgb(hsv))
        }),
        RgbMixer {
            matrix,
            preserve_luminance,
        } => f.map_rgb(|rgb, _| {
            let out = mul(*matrix, rgb);
            if *preserve_luminance {
                let l = luminance(out);
                if l.abs() > 1e-8 {
                    out.map(|v| v * luminance(rgb) / l)
                } else {
                    out
                }
            } else {
                out
            }
        }),
        SplitTone {
            shadows,
            highlights,
            balance,
            strength,
        } => f.map_rgb(|rgb, _| {
            let enc = to_display(rgb);
            let h = smooth(0.0, 1.0, luminance(enc) + balance * 0.5);
            from_display(std::array::from_fn(|c| {
                enc[c] + strength * (shadows[c] * (1.0 - h) + highlights[c] * h)
            }))
        }),
        Lut {
            path,
            domain,
            interpolation,
        } => {
            let lut = read_lut(path)?;
            f.map_rgb(|rgb, _| match domain {
                LutDomain::Linear => lut.sample_with(rgb, *interpolation),
                LutDomain::Srgb => from_display(lut.sample_with(to_display(rgb), *interpolation)),
            })
        }
        ToneMap { method, white } => f.map_rgb(|rgb, _| {
            rgb.map(|v| {
                let x = (v / white).max(0.0);
                match method {
                    crate::recipe::ToneMap::Reinhard => x / (1.0 + x),
                    crate::recipe::ToneMap::AcesFit => {
                        ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
                    }
                }
            })
        }),
        GamutCompress {
            threshold,
            softness,
        } => f.map_rgb(|rgb, _| {
            let l = luminance(rgb).max(0.0);
            let max = rgb.into_iter().fold(f32::NEG_INFINITY, f32::max);
            let min = rgb.into_iter().fold(f32::INFINITY, f32::min);
            let chroma = (max - l).max(l - min);
            let allowed = *threshold * l.max(0.01);
            let target = if chroma > allowed {
                allowed + softness * (1.0 - (-(chroma - allowed) / softness).exp())
            } else {
                chroma
            };
            let scale = if chroma > 1e-8 { target / chroma } else { 1.0 };
            rgb.map(|v| l + (v - l) * scale)
        }),
        Blur { radius } => blur(f, *radius),
        Sharpen {
            radius,
            amount,
            threshold,
        } => {
            let b = blur(f, *radius);
            f.map_rgb(|rgb, i| {
                std::array::from_fn(|c| {
                    let d = rgb[c] - b.pixels[i][c];
                    rgb[c]
                        + if d.abs() >= *threshold {
                            amount * d
                        } else {
                            0.0
                        }
                })
            })
        }
        Clarity { radius, amount } => {
            let b = blur(f, *radius);
            f.map_rgb(|rgb, i| {
                let l = luminance(rgb);
                let weight = band(to_display([l; 3])[0], 0.1, 0.9, 0.1);
                std::array::from_fn(|c| rgb[c] + amount * weight * (rgb[c] - b.pixels[i][c]))
            })
        }
        Denoise {
            radius,
            spatial_sigma,
            range_sigma,
        } => {
            let radius = *radius as i32;
            let pixels = (0..f.pixels.len())
                .into_par_iter()
                .map(|i| {
                    let x = (i % f.width as usize) as i32;
                    let y = (i / f.width as usize) as i32;
                    let p = f.pixels[i];
                    let mut out = [0.0; 3];
                    let mut total = 0.0;
                    for dy in -radius..=radius {
                        for dx in -radius..=radius {
                            let q = f.pixels[((y + dy).clamp(0, f.height as i32 - 1) as u32
                                * f.width
                                + (x + dx).clamp(0, f.width as i32 - 1) as u32)
                                as usize];
                            let d: f32 = (0..3).map(|c| (p[c] - q[c]).powi(2)).sum();
                            let w = (-(dx * dx + dy * dy) as f32 / (2.0 * spatial_sigma.powi(2))
                                - d / (2.0 * range_sigma.powi(2)))
                            .exp()
                                * q[3];
                            for c in 0..3 {
                                out[c] += q[c] * w;
                            }
                            total += w;
                        }
                    }
                    if total > 1e-8 {
                        [out[0] / total, out[1] / total, out[2] / total, p[3]]
                    } else {
                        p
                    }
                })
                .collect();
            Frame {
                width: f.width,
                height: f.height,
                pixels,
            }
        }
        Vignette {
            amount,
            center,
            radius,
            feather,
        } => f.map_rgb(|rgb, i| {
            let x = (i % f.width as usize) as f32 / f.width as f32 - center[0];
            let y = (i / f.width as usize) as f32 / f.height as f32 - center[1];
            let d = (x * x + y * y).sqrt() * 2.0;
            let w = smooth(radius * (1.0 - feather), *radius, d);
            rgb.map(|v| v * 2f32.powf(amount * w))
        }),
        Grain {
            amount,
            seed,
            monochrome,
        } => f.map_rgb(|rgb, i| {
            let enc = to_display(rgb);
            from_display(std::array::from_fn(|c| {
                enc[c] + amount * noise(*seed, i, if *monochrome { 0 } else { c })
            }))
        }),
        Glow {
            radius,
            threshold,
            amount,
        } => {
            let bright = f.map_rgb(|rgb, _| {
                let l = luminance(rgb);
                let w = if l > *threshold {
                    (l - threshold) / l.max(1e-8)
                } else {
                    0.0
                };
                rgb.map(|v| v * w)
            });
            let b = blur(&bright, *radius);
            f.map_rgb(|rgb, i| std::array::from_fn(|c| rgb[c] + amount * b.pixels[i][c]))
        }
        Crop {
            x,
            y,
            width,
            height,
        } => {
            ensure!(
                *x as u64 + *width as u64 <= f.width as u64
                    && *y as u64 + *height as u64 <= f.height as u64,
                "crop lies outside image"
            );
            let pixels = (0..*height)
                .flat_map(|row| {
                    let begin = ((row + y) * f.width + x) as usize;
                    f.pixels[begin..begin + *width as usize].iter().copied()
                })
                .collect();
            Frame::new(*width, *height, pixels)?
        }
        Resize { width, height } => f.resize_to(*width, *height)?,
        Rotate { degrees } => {
            let q = (degrees.rem_euclid(360.0) / 90.0) as u32;
            let (w, h) = if q % 2 == 1 {
                (f.height, f.width)
            } else {
                (f.width, f.height)
            };
            let pixels = (0..w * h)
                .map(|i| {
                    let x = i % w;
                    let y = i / w;
                    let (sx, sy) = match q {
                        0 => (x, y),
                        1 => (y, f.height - 1 - x),
                        2 => (f.width - 1 - x, f.height - 1 - y),
                        _ => (f.width - 1 - y, x),
                    };
                    f.pixels[(sy * f.width + sx) as usize]
                })
                .collect();
            Frame::new(w, h, pixels)?
        }
        LensDistortion { k1, k2, center } => {
            let mut out = (*f).clone();
            out.pixels.par_iter_mut().enumerate().for_each(|(i, p)| {
                let x = (i % f.width as usize) as f32;
                let y = (i / f.width as usize) as f32;
                let dx = x / f.width as f32 - center[0];
                let dy = y / f.height as f32 - center[1];
                let r2 = 4.0 * (dx * dx + dy * dy);
                let scale = 1.0 + k1 * r2 + k2 * r2 * r2;
                let sx = (center[0] + dx * scale) * f.width as f32;
                let sy = (center[1] + dy * scale) * f.height as f32;
                *p = if sx < 0.0
                    || sy < 0.0
                    || sx > (f.width - 1) as f32
                    || sy > (f.height - 1) as f32
                {
                    [0.0; 4]
                } else {
                    f.sample(sx, sy)
                };
            });
            out
        }
        Clone {
            source,
            target,
            radius,
            feather,
        } => {
            let mut out = (*f).clone();
            out.pixels.par_iter_mut().enumerate().for_each(|(i, p)| {
                let x = (i % f.width as usize) as f32 + 0.5;
                let y = (i / f.width as usize) as f32 + 0.5;
                let dx = x / f.width as f32 - target[0];
                let dy = y / f.height as f32 - target[1];
                let d = (dx * dx + dy * dy).sqrt();
                let w = 1.0 - smooth(radius * (1.0 - feather), *radius, d);
                if w > 0.0 {
                    let q = f.sample(
                        x - 0.5 + (source[0] - target[0]) * f.width as f32,
                        y - 0.5 + (source[1] - target[1]) * f.height as f32,
                    );
                    for c in 0..4 {
                        p[c] = p[c] * (1.0 - w) + q[c] * w;
                    }
                }
            });
            out
        }
        Blend { mode } => {
            ensure!(inputs.len() == 2, "blend requires two inputs");
            let b = inputs[1];
            ensure!(
                f.width == b.width && f.height == b.height,
                "blend inputs must have identical dimensions"
            );
            // Produce fully composited result; node mix/mask interpolates it with base.
            let mut out = (*f).clone();
            out.pixels.par_iter_mut().enumerate().for_each(|(i, p)| {
                let bg = f.pixels[i];
                let fg = b.pixels[i];
                let alpha = fg[3] + bg[3] * (1.0 - fg[3]);
                for c in 0..3 {
                    let m = match mode {
                        BlendMode::Normal => fg[c],
                        BlendMode::Add => bg[c] + fg[c],
                        BlendMode::Multiply => bg[c] * fg[c],
                        BlendMode::Screen => 1.0 - (1.0 - bg[c]) * (1.0 - fg[c]),
                        BlendMode::Overlay => {
                            if bg[c] <= 0.5 {
                                2.0 * bg[c] * fg[c]
                            } else {
                                1.0 - 2.0 * (1.0 - bg[c]) * (1.0 - fg[c])
                            }
                        }
                        BlendMode::Difference => (bg[c] - fg[c]).abs(),
                    };
                    p[c] = if alpha > 1e-8 {
                        ((1.0 - fg[3]) * bg[3] * bg[c]
                            + (1.0 - bg[3]) * fg[3] * fg[c]
                            + bg[3] * fg[3] * m)
                            / alpha
                    } else {
                        0.0
                    };
                }
                p[3] = alpha;
            });
            out
        }
    };
    ensure!(
        out.pixels.iter().flatten().all(|v| v.is_finite()),
        "operation produced non-finite pixels"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(op: Operation, f: &Frame) -> Frame {
        apply(&op, &[f], &|_| unreachable!()).unwrap()
    }
    #[test]
    fn exposure_preserves_hdr_negative_and_alpha() {
        let f = Frame::new(1, 1, vec![[-0.25, 0.18, 4.0, 0.3]]).unwrap();
        let o = run(Operation::Exposure { stops: 1.0 }, &f);
        assert_eq!(o.pixels[0], [-0.5, 0.36, 8.0, 0.3]);
    }
    #[test]
    fn identity_curves_and_primary() {
        let f = Frame::new(1, 1, vec![[0.2, 0.4, 1.2, 1.0]]).unwrap();
        let op: Operation = serde_json::from_str(r#"{"type":"primary"}"#).unwrap();
        let o = run(op, &f);
        assert!((0..3).all(|c| (o.pixels[0][c] - f.pixels[0][c]).abs() < 1e-5));
        let o = run(
            Operation::Curves {
                channel: CurveChannel::Rgb,
                points: vec![[0.0, 0.0], [1.0, 1.0]],
            },
            &f,
        );
        assert!((o.pixels[0][2] - 1.2).abs() < 1e-5);
    }
    #[test]
    fn grain_is_reproducible_and_geometry_is_exact() {
        let f = Frame::new(
            2,
            3,
            (0..6)
                .map(|v| [v as f32, v as f32, v as f32, 1.0])
                .collect(),
        )
        .unwrap();
        let op = Operation::Grain {
            amount: 0.02,
            seed: 42,
            monochrome: true,
        };
        assert_eq!(run(op.clone(), &f).pixels, run(op, &f).pixels);
        let r = run(Operation::Rotate { degrees: 90.0 }, &f);
        assert_eq!((r.width, r.height), (3, 2));
        assert_eq!(
            r.pixels.iter().map(|p| p[0]).collect::<Vec<_>>(),
            [4.0, 2.0, 0.0, 5.0, 3.0, 1.0]
        );
        assert!(
            apply(
                &Operation::Crop {
                    x: 1,
                    y: 2,
                    width: 2,
                    height: 2
                },
                &[&f],
                &|_| unreachable!()
            )
            .is_err()
        );
    }
    #[test]
    fn blur_does_not_leak_transparent_rgb() {
        let f = Frame::new(2, 1, vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 100.0, 0.0]]).unwrap();
        let b = blur(&f, 1.0);
        assert!(b.pixels.iter().all(|p| p[2] == 0.0));
    }
    #[test]
    fn cancellable_color_blocks_preserve_exact_pixels_and_spatial_fallback() {
        let f = Frame::new(
            137,
            241,
            (0..137 * 241)
                .map(|i| {
                    [
                        (i % 317) as f32 / 190.0 - 0.1,
                        (i % 127) as f32 / 140.0,
                        (i % 233) as f32 / 250.0,
                        (i % 7) as f32 / 6.0,
                    ]
                })
                .collect(),
        )
        .unwrap();
        let recipe: Recipe =
            serde_json::from_str(include_str!("../../../examples/cinematic.json")).unwrap();
        let mut ops: Vec<_> = recipe.nodes.into_iter().map(|n| n.op).collect();
        ops.push(Operation::Tone {
            shadows: 0.1,
            highlights: -0.2,
            whites: 0.04,
            blacks: -0.03,
        });
        ops.push(Operation::Hsl {
            bands: vec![HslBand {
                hue: 0.6,
                width: 0.2,
                hue_shift: -0.04,
                saturation: 1.2,
                luminance: 0.9,
            }],
        });
        for op in ops {
            let expected = apply(&op, &[&f], &|_| unreachable!()).unwrap();
            let actual =
                apply_cancellable(&op, &[&f], &|_| unreachable!(), &AtomicBool::new(false))
                    .unwrap();
            assert_eq!(actual.pixels, expected.pixels, "{op:?}");
            assert_eq!((actual.width, actual.height), (137, 241));
            assert!(
                apply_cancellable(&op, &[&f], &|_| unreachable!(), &AtomicBool::new(true)).is_err()
            );
        }
    }
}
