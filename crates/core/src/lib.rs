pub mod cutout;
pub mod lut;
pub mod mask;
pub mod ops;
pub mod recipe;
mod resample;
pub mod scopes;

use anyhow::{Result, ensure};
pub use mask::Mask;
use rayon::prelude::*;
pub use recipe::{Node, Operation, Recipe};
use serde::{Deserialize, Serialize};

/// Straight alpha, scene-linear sRGB/D65, unclipped f32 RGB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}
impl Frame {
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> Result<Self> {
        ensure!(width > 0 && height > 0, "image dimensions must be positive");
        ensure!(
            width as u64 * height as u64 == pixels.len() as u64,
            "pixel count does not match dimensions"
        );
        ensure!(
            pixels.iter().flatten().all(|v| v.is_finite()),
            "image contains non-finite values"
        );
        ensure!(
            pixels.iter().all(|p| (0.0..=1.0).contains(&p[3])),
            "alpha must be in [0,1]"
        );
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn map_rgb(&self, f: impl Fn([f32; 3], usize) -> [f32; 3] + Send + Sync) -> Self {
        let mut out = self.clone();
        out.pixels.par_iter_mut().enumerate().for_each(|(i, p)| {
            let rgb = f([p[0], p[1], p[2]], i);
            p[..3].copy_from_slice(&rgb);
        });
        out
    }
    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = x.clamp(0.0, (self.width - 1) as f32);
        let y = y.clamp(0.0, (self.height - 1) as f32);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let get = |x: u32, y: u32| self.pixels[(y * self.width + x) as usize];
        let a = get(x0, y0);
        let b = get(x1, y0);
        let c = get(x0, y1);
        let d = get(x1, y1);
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let weights = [
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ];
        let samples = [a, b, c, d];
        let alpha: f32 = samples.iter().zip(weights).map(|(p, w)| p[3] * w).sum();
        let mut out = [0.0, 0.0, 0.0, alpha];
        if alpha > 1e-8 {
            for ch in 0..3 {
                out[ch] = samples
                    .iter()
                    .zip(weights)
                    .map(|(p, w)| p[ch] * p[3] * w)
                    .sum::<f32>()
                    / alpha;
            }
        }
        out
    }
    pub fn resized(&self, max_edge: u32) -> Result<Self> {
        ensure!(max_edge > 0, "max edge must be positive");
        if self.width.max(self.height) <= max_edge {
            return Ok(self.clone());
        }
        let scale = max_edge as f32 / self.width.max(self.height) as f32;
        let w = (self.width as f32 * scale).round().max(1.0) as u32;
        let h = (self.height as f32 * scale).round().max(1.0) as u32;
        self.resize_to(w, h)
    }
}
