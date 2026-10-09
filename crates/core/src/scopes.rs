use crate::Frame;
use serde::Serialize;
use tinge_color::{luminance, to_display};

#[derive(Debug, Serialize)]
pub struct Analysis {
    pub width: u32,
    pub height: u32,
    pub working_space: &'static str,
    pub pixels: u64,
    pub rgb_mean: [f64; 3],
    pub rgb_min: [f32; 3],
    pub rgb_max: [f32; 3],
    pub luminance_percentiles: [f32; 5],
    pub percentile_positions: [u32; 5],
    pub below_zero_fraction: f64,
    pub above_one_fraction: f64,
    /// Gray-world estimate only. Does not identify a neutral object or illuminant.
    pub suggested_white_balance_gains: [f64; 3],
    pub histogram: Histogram,
}
#[derive(Debug, Serialize)]
pub struct Histogram {
    pub domain: &'static str,
    pub bins: usize,
    pub rgb: Vec<Vec<u64>>,
    pub luma: Vec<u64>,
}
pub fn analyze(frame: &Frame) -> Analysis {
    let bins = 256;
    let mut rgb = vec![vec![0; bins]; 3];
    let mut hist = vec![0; bins];
    let mut sum = [0.0; 3];
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut ls = Vec::with_capacity(frame.pixels.len());
    let mut low = 0;
    let mut high = 0;
    for p in &frame.pixels {
        for c in 0..3 {
            sum[c] += p[c] as f64;
            min[c] = min[c].min(p[c]);
            max[c] = max[c].max(p[c]);
        }
        if p[..3].iter().any(|v| *v < 0.0) {
            low += 1;
        }
        if p[..3].iter().any(|v| *v > 1.0) {
            high += 1;
        }
        let v = [p[0], p[1], p[2]];
        let enc = to_display(v);
        for c in 0..3 {
            rgb[c][(enc[c].clamp(0.0, 1.0) * 255.0).round() as usize] += 1;
        }
        let l = luminance(v);
        ls.push(l);
        let d = to_display([l; 3])[0];
        hist[(d.clamp(0.0, 1.0) * 255.0).round() as usize] += 1;
    }
    ls.sort_unstable_by(f32::total_cmp);
    let positions = [1, 10, 50, 90, 99];
    let percentiles =
        positions.map(|p| ls[((ls.len() - 1) as f64 * p as f64 / 100.0).round() as usize]);
    let n = frame.pixels.len() as f64;
    let mean = sum.map(|v| v / n);
    let neutral = (mean[0] + mean[1] + mean[2]) / 3.0;
    Analysis {
        width: frame.width,
        height: frame.height,
        working_space: "scene-linear sRGB D65, straight alpha, float32",
        pixels: frame.pixels.len() as u64,
        rgb_mean: mean,
        rgb_min: min,
        rgb_max: max,
        luminance_percentiles: percentiles,
        percentile_positions: positions,
        below_zero_fraction: low as f64 / n,
        above_one_fraction: high as f64 / n,
        suggested_white_balance_gains: mean.map(|m| (neutral / m.max(1e-8)).clamp(0.1, 10.0)),
        histogram: Histogram {
            domain: "sRGB encoded; overflow counted at edges",
            bins,
            rgb,
            luma: hist,
        },
    }
}

/// RGB parade, luma waveform, and vectorscope density data for agents/viewers.
#[derive(Debug, Serialize)]
pub struct Scopes {
    pub width: usize,
    pub height: usize,
    pub domain: &'static str,
    pub waveform: Vec<Vec<u32>>,
    pub parade: Vec<Vec<Vec<u32>>>,
    pub vectorscope: Vec<Vec<u32>>,
}
pub fn scopes(f: &Frame) -> Scopes {
    let (w, h) = (256, 256);
    let mut waveform = vec![vec![0u32; w]; h];
    let mut parade = vec![vec![vec![0u32; w]; h]; 3];
    let mut vectorscope = vec![vec![0u32; w]; h];
    for (i, p) in f.pixels.iter().enumerate() {
        let enc = to_display([p[0], p[1], p[2]]);
        let col = (i % f.width as usize) * w / f.width as usize;
        let row = |v: f32| 255 - (v.clamp(0.0, 1.0) * 255.0).round() as usize;
        waveform[row(luminance(enc))][col] += 1;
        for c in 0..3 {
            parade[c][row(enc[c])][col] += 1;
        }
        let y = luminance(enc);
        let cb = (enc[2] - y) / 1.8556;
        let cr = (enc[0] - y) / 1.5748;
        let x = ((cb + 0.5) * 255.0).clamp(0.0, 255.0).round() as usize;
        let y = ((0.5 - cr) * 255.0).clamp(0.0, 255.0).round() as usize;
        vectorscope[y][x] += 1;
    }
    Scopes {
        width: w,
        height: h,
        domain: "sRGB display encoded, Rec.709 luma, Cb/Cr",
        waveform,
        parade,
        vectorscope,
    }
}
