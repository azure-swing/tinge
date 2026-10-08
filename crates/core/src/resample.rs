use crate::Frame;
use anyhow::{Result, ensure};
use rayon::prelude::*;

// Widen the reconstruction filter before decimation, so source frequencies
// above the destination Nyquist limit cannot turn into large false patterns.
fn weights(source: u32, destination: u32) -> Vec<Vec<(usize, f32)>> {
    let ratio = source as f64 / destination as f64;
    (0..destination)
        .map(|index| {
            if source == destination {
                return vec![(index as usize, 1.0)];
            }
            let center = (index as f64 + 0.5) * ratio - 0.5;
            let down = ratio > 1.0;
            let support = if down { 3.0 * ratio } else { 1.0 };
            let mut taps = Vec::new();
            for position in (center - support).ceil() as i64..=(center + support).floor() as i64 {
                let distance = (position as f64 - center).abs();
                let weight = if down {
                    let x = distance / ratio;
                    if x < 1e-12 {
                        1.0
                    } else if x >= 3.0 {
                        0.0
                    } else {
                        let p = std::f64::consts::PI * x;
                        (p.sin() / p) * ((p / 3.0).sin() / (p / 3.0))
                    }
                } else {
                    (1.0 - distance).max(0.0)
                };
                if weight != 0.0 {
                    taps.push((position.clamp(0, source as i64 - 1) as usize, weight));
                }
            }
            let sum: f64 = taps.iter().map(|(_, weight)| weight).sum();
            taps.into_iter()
                .map(|(i, w)| (i, (w / sum) as f32))
                .collect()
        })
        .collect()
}

impl Frame {
    /// Resize in scene-linear light, filtering premultiplied color and alpha.
    /// Each shrinking axis uses a scale-aware Lanczos3 low-pass filter.
    pub fn resize_to(&self, width: u32, height: u32) -> Result<Self> {
        ensure!(width > 0 && height > 0, "image dimensions must be positive");
        ensure!(
            width as u64 * height as u64 <= 100_000_000,
            "resize exceeds 100M pixels"
        );
        if width == self.width && height == self.height {
            return Ok(self.clone());
        }
        let horizontal = weights(self.width, width);
        let vertical = weights(self.height, height);
        // Filter the axis that produces the smaller intermediate first; this
        // also bounds memory when one axis shrinks while the other enlarges.
        let horizontal_first =
            width as u64 * self.height as u64 <= self.width as u64 * height as u64;
        let intermediate_width = if horizontal_first { width } else { self.width } as usize;
        let intermediate_height = if horizontal_first {
            self.height
        } else {
            height
        } as usize;
        // Keep alpha premultiplied between passes. Unpremultiplying here would
        // discard signed filter contributions along transparent edges.
        let intermediate: Vec<[f32; 4]> = (0..intermediate_width * intermediate_height)
            .into_par_iter()
            .map(|i| {
                let x = i % intermediate_width;
                let y = i / intermediate_width;
                let taps = if horizontal_first {
                    &horizontal[x]
                } else {
                    &vertical[y]
                };
                let mut out = [0.0; 4];
                for &(position, weight) in taps {
                    let source_index = if horizontal_first {
                        y * self.width as usize + position
                    } else {
                        position * self.width as usize + x
                    };
                    let p = self.pixels[source_index];
                    for ch in 0..3 {
                        out[ch] += p[ch] * p[3] * weight;
                    }
                    out[3] += p[3] * weight;
                }
                out
            })
            .collect();
        let pixels = (0..width as usize * height as usize)
            .into_par_iter()
            .map(|i| {
                let mut out = [0.0; 4];
                let x = i % width as usize;
                let y = i / width as usize;
                let taps = if horizontal_first {
                    &vertical[y]
                } else {
                    &horizontal[x]
                };
                for &(position, weight) in taps {
                    let intermediate_index = if horizontal_first {
                        position * intermediate_width + x
                    } else {
                        y * intermediate_width + position
                    };
                    let p = intermediate[intermediate_index];
                    for ch in 0..4 {
                        out[ch] += p[ch] * weight;
                    }
                }
                if out[3] > 1e-8 {
                    for ch in 0..3 {
                        out[ch] /= out[3];
                    }
                    out[3] = out[3].clamp(0.0, 1.0);
                } else {
                    out = [0.0; 4];
                }
                out
            })
            .collect();
        Frame::new(width, height, pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinking_rejects_false_color_stripes_at_noninteger_scales() {
        let frame = Frame::new(
            121,
            119,
            (0..121 * 119)
                .map(|i| {
                    if (i / 121) % 2 == 0 {
                        [1.0, 0.0, 1.0, 1.0]
                    } else {
                        [0.0, 1.0, 0.0, 1.0]
                    }
                })
                .collect(),
        )
        .unwrap();
        let out = frame.resize_to(28, 27).unwrap();
        // Ignore the clamped image border: interior high-frequency chroma must
        // average to neutral, rather than form visible magenta/green bands.
        for y in 3..24 {
            for x in 3..25 {
                for ch in 0..3 {
                    assert!((out.pixels[y * 28 + x][ch] - 0.5).abs() < 0.01);
                }
            }
        }
    }

    #[test]
    fn resize_preserves_linear_hdr_constants_and_identity() {
        let frame = Frame::new(37, 23, vec![[2.5, -0.25, 0.5, 0.7]; 37 * 23]).unwrap();
        assert_eq!(frame.resize_to(37, 23).unwrap().pixels, frame.pixels);
        for (w, h) in [(11, 7), (73, 41), (11, 41), (1024, 1), (1, 1024)] {
            let out = frame.resize_to(w, h).unwrap();
            for p in out.pixels {
                for (v, expected) in p.into_iter().zip([2.5, -0.25, 0.5, 0.7]) {
                    assert!((v - expected).abs() < 2e-6);
                }
            }
        }
        assert_eq!(frame.resized(100).unwrap().pixels, frame.pixels);
        let small = frame.resized(11).unwrap();
        assert_eq!((small.width, small.height), (11, 7));
        assert!(frame.resize_to(0, 1).is_err());
    }

    #[test]
    fn transparent_color_does_not_bleed_and_alpha_stays_valid() {
        let frame = Frame::new(
            32,
            32,
            (0..1024)
                .map(|i| {
                    if i % 32 < 16 {
                        [8.0, 0.0, 0.0, 0.0]
                    } else {
                        [0.0, 0.4, 1.5, 1.0]
                    }
                })
                .collect(),
        )
        .unwrap();
        let out = frame.resize_to(9, 7).unwrap();
        for p in out.pixels {
            assert_eq!(p[0], 0.0);
            assert!((0.0..=1.0).contains(&p[3]));
            if p[3] > 0.0 {
                assert!((p[1] - 0.4).abs() < 1e-5);
                assert!((p[2] - 1.5).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn downsampling_averages_in_linear_light() {
        let frame = Frame::new(
            64,
            64,
            (0..4096)
                .map(|i| {
                    let value = if (i % 64 + i / 64) % 2 == 0 { 1.0 } else { 0.0 };
                    [value, value, value, 1.0]
                })
                .collect(),
        )
        .unwrap();
        let out = frame.resize_to(8, 8).unwrap();
        assert!((out.pixels[4 * 8 + 4][0] - 0.5).abs() < 0.001);
    }
}
