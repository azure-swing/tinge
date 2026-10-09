//! Levin/Lischinski/Weiss closed-form matting Laplacian, 3x3 windows, epsilon 1e-7.
//! Dirichlet known pixels are eliminated; Jacobi-preconditioned conjugate gradient.
use super::{MattingSpace, Refinement, cancelled, rgb};
use crate::Frame;
use anyhow::{Result, ensure};
use rayon::prelude::*;
use serde::Serialize;
use std::{collections::VecDeque, sync::atomic::AtomicBool};

pub(super) fn invert3(a: [[f64; 3]; 3]) -> ([[f64; 3]; 3], f64) {
    let c = [
        [
            a[1][1] * a[2][2] - a[1][2] * a[2][1],
            a[1][2] * a[2][0] - a[1][0] * a[2][2],
            a[1][0] * a[2][1] - a[1][1] * a[2][0],
        ],
        [
            a[0][2] * a[2][1] - a[0][1] * a[2][2],
            a[0][0] * a[2][2] - a[0][2] * a[2][0],
            a[0][1] * a[2][0] - a[0][0] * a[2][1],
        ],
        [
            a[0][1] * a[1][2] - a[0][2] * a[1][1],
            a[0][2] * a[1][0] - a[0][0] * a[1][2],
            a[0][0] * a[1][1] - a[0][1] * a[1][0],
        ],
    ];
    let determinant = (0..3).map(|j| a[0][j] * c[0][j]).sum::<f64>();
    (
        std::array::from_fn(|i| std::array::from_fn(|j| c[j][i] / determinant)),
        determinant,
    )
}
#[derive(Debug, Serialize)]
pub struct SolveReport {
    pub unknown_pixels: usize,
    pub iterations: u32,
    pub relative_residual: f64,
    pub converged: bool,
}
pub(super) fn solve(
    colors: &[[f64; 3]],
    w: usize,
    h: usize,
    trimap: &[f32],
    options: &Refinement,
    cancel: &AtomicBool,
) -> Result<(Vec<f32>, SolveReport)> {
    let unknown: Vec<usize> = trimap
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0.0 && **v < 1.0)
        .map(|(i, _)| i)
        .collect();
    let count = unknown.len();
    if count == 0 {
        return Ok((
            trimap.to_vec(),
            SolveReport {
                unknown_pixels: 0,
                iterations: 0,
                relative_residual: 0.0,
                converged: true,
            },
        ));
    }
    ensure!(
        w >= 3 && h >= 3,
        "closed-form matting requires dimensions at least 3x3"
    );
    ensure!(
        trimap.contains(&0.0) && trimap.contains(&1.0),
        "matting needs certain foreground and background; narrow the unknown band or add strokes"
    );
    ensure!(
        count <= 500_000,
        "matting unknown region exceeds 500000 pixels; lower refinement.max_edge or narrow the trimap"
    );
    let mut index = vec![u32::MAX; w * h];
    for (i, p) in unknown.iter().enumerate() {
        index[*p] = i as u32;
    }
    let mut rows = vec![[0.0_f64; 25]; count];
    let mut rhs = vec![0.0; count];
    for y in 1..h - 1 {
        if y.is_multiple_of(16) {
            cancelled(cancel)?;
        }
        for x in 1..w - 1 {
            let ids: [usize; 9] = std::array::from_fn(|j| (y + j / 3 - 1) * w + x + j % 3 - 1);
            if !ids.iter().any(|i| index[*i] != u32::MAX) {
                continue;
            }
            let mean: [f64; 3] =
                std::array::from_fn(|c| ids.iter().map(|i| colors[*i][c]).sum::<f64>() / 9.0);
            let centered: [[f64; 3]; 9] =
                ids.map(|i| std::array::from_fn(|c| colors[i][c] - mean[c]));
            let cov = std::array::from_fn(|r| {
                std::array::from_fn(|c| {
                    centered.iter().map(|v| v[r] * v[c]).sum::<f64>() / 9.0
                        + if r == c { 1e-7 / 9.0 } else { 0.0 }
                })
            });
            let (inv, det) = invert3(cov);
            ensure!(det > 0.0 && det.is_finite(), "invalid matting covariance");
            for a in 0..9 {
                let row = index[ids[a]];
                if row == u32::MAX {
                    continue;
                }
                let q: [f64; 3] =
                    std::array::from_fn(|r| (0..3).map(|c| inv[r][c] * centered[a][c]).sum());
                for b in 0..9 {
                    let coefficient = if a == b { 1.0 } else { 0.0 }
                        - (1.0 + (0..3).map(|c| q[c] * centered[b][c]).sum::<f64>()) / 9.0;
                    if index[ids[b]] == u32::MAX {
                        rhs[row as usize] -= coefficient * trimap[ids[b]] as f64;
                    } else {
                        let dy = b as isize / 3 - a as isize / 3;
                        let dx = b as isize % 3 - a as isize % 3;
                        rows[row as usize][((dy + 2) * 5 + dx + 2) as usize] += coefficient;
                    }
                }
            }
        }
    }
    // Tiny regularizer handles local rank deficiency without changing hard labels.
    for (row, b) in rows.iter_mut().zip(&mut rhs) {
        row[12] += 1e-8;
        *b += 0.5e-8;
    }
    let multiply = |values: &[f64]| -> Vec<f64> {
        rows.par_iter()
            .enumerate()
            .map(|(i, row)| {
                let p = unknown[i];
                let (x, y) = (p % w, p / w);
                let mut sum = 0.0;
                for (j, c) in row.iter().enumerate().filter(|(_, v)| **v != 0.0) {
                    let (xx, yy) = (
                        x as isize + j as isize % 5 - 2,
                        y as isize + j as isize / 5 - 2,
                    );
                    if xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize {
                        let k = index[yy as usize * w + xx as usize];
                        if k != u32::MAX {
                            sum += c * values[k as usize];
                        }
                    }
                }
                sum
            })
            .collect()
    };
    // Deterministic reductions, independent of Rayon thread count.
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let mut values = vec![0.5; count];
    let ax = multiply(&values);
    let mut residual: Vec<f64> = rhs.iter().zip(ax).map(|(b, a)| b - a).collect();
    let norm = dot(&rhs, &rhs).sqrt().max(1e-15);
    let mut relative = dot(&residual, &residual).sqrt() / norm;
    let mut z: Vec<f64> = residual
        .iter()
        .zip(&rows)
        .map(|(r, row)| r / row[12])
        .collect();
    let mut direction = z.clone();
    let mut rz = dot(&residual, &z);
    let mut iterations = 0;
    while relative > options.tolerance && iterations < options.iterations {
        cancelled(cancel)?;
        let ad = multiply(&direction);
        let denominator = dot(&direction, &ad);
        ensure!(
            denominator.is_finite() && denominator > 0.0,
            "matting linear solve lost positive definiteness"
        );
        let step = rz / denominator;
        for i in 0..count {
            values[i] += step * direction[i];
            residual[i] -= step * ad[i];
        }
        iterations += 1;
        relative = dot(&residual, &residual).sqrt() / norm;
        if relative <= options.tolerance {
            break;
        }
        for i in 0..count {
            z[i] = residual[i] / rows[i][12];
        }
        let next = dot(&residual, &z);
        let beta = next / rz;
        rz = next;
        for i in 0..count {
            direction[i] = z[i] + beta * direction[i];
        }
    }
    ensure!(
        relative.is_finite() && values.iter().all(|v| v.is_finite()),
        "matting produced non-finite alpha"
    );
    let mut out = trimap.to_vec();
    for (i, v) in unknown.into_iter().zip(values) {
        out[i] = v.clamp(0.0, 1.0) as f32;
    }
    Ok((
        out,
        SolveReport {
            unknown_pixels: count,
            iterations,
            relative_residual: relative,
            converged: relative <= options.tolerance,
        },
    ))
}

fn magnitude(frame: &Frame) -> f64 {
    frame
        .pixels
        .iter()
        .flat_map(|p| p[..3].iter())
        .fold(1.0_f64, |a, v| a.max((*v as f64).abs()))
}
pub(super) fn colors(frame: &Frame, space: MattingSpace) -> Vec<[f64; 3]> {
    if space == MattingSpace::Srgb {
        return rgb(frame);
    }
    let scale = magnitude(frame);
    frame
        .pixels
        .iter()
        .map(|p| {
            [
                p[0] as f64 / scale,
                p[1] as f64 / scale,
                p[2] as f64 / scale,
            ]
        })
        .collect()
}
pub(super) fn upsample(
    alpha: &[f32],
    small: &Frame,
    full: &Frame,
    space: MattingSpace,
) -> Vec<f32> {
    if small.width == full.width && small.height == full.height {
        return alpha.to_vec();
    }
    let scale = magnitude(full);
    let encode = |p: &[f32; 4]| {
        if space == MattingSpace::Linear {
            [
                p[0] as f64 / scale,
                p[1] as f64 / scale,
                p[2] as f64 / scale,
            ]
        } else {
            tinge_color::to_display([p[0], p[1], p[2]]).map(|v| v.clamp(0.0, 1.0) as f64)
        }
    };
    let colors: Vec<_> = small.pixels.iter().map(encode).collect();
    let w = small.width as usize;
    let h = small.height as usize;
    full.pixels
        .par_iter()
        .enumerate()
        .map(|(i, p)| {
            let color = encode(p);
            let x = ((i % full.width as usize) as f64 + 0.5) * w as f64 / full.width as f64 - 0.5;
            let y = ((i / full.width as usize) as f64 + 0.5) * h as f64 / full.height as f64 - 0.5;
            let (cx, cy) = (x.round() as isize, y.round() as isize);
            let mut sum = 0.0;
            let mut weights = 0.0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let xx = (cx + dx).clamp(0, w as isize - 1);
                    let yy = (cy + dy).clamp(0, h as isize - 1);
                    let j = yy as usize * w + xx as usize;
                    let distance = (0..3)
                        .map(|c| (color[c] - colors[j][c]).powi(2))
                        .sum::<f64>();
                    let spatial = (xx as f64 - x).powi(2) + (yy as f64 - y).powi(2);
                    let weight = (-distance / 0.02 - spatial / 2.0).exp();
                    sum += weight * alpha[j] as f64;
                    weights += weight;
                }
            }
            (sum / weights.max(1e-300)).clamp(0.0, 1.0) as f32
        })
        .collect()
}
pub(super) fn feather(alpha: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let size = (sigma * 3.0).ceil() as isize;
    let mut kernel: Vec<f32> = (-size..=size)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total = kernel.iter().sum::<f32>();
    for v in &mut kernel {
        *v /= total;
    }
    let pass = |src: &[f32], horizontal: bool| {
        (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                kernel
                    .iter()
                    .enumerate()
                    .map(|(j, k)| {
                        let d = j as isize - size;
                        let xx = (x + if horizontal { d } else { 0 }).clamp(0, w as isize - 1);
                        let yy = (y + if horizontal { 0 } else { d }).clamp(0, h as isize - 1);
                        src[yy as usize * w + xx as usize] * k
                    })
                    .sum::<f32>()
                    .clamp(0.0, 1.0)
            })
            .collect::<Vec<_>>()
    };
    pass(&pass(alpha, true), false)
}
fn nearest(alpha: &[f32], w: usize, h: usize, foreground: bool) -> Vec<usize> {
    let mut owner = vec![usize::MAX; w * h];
    let mut queue = VecDeque::new();
    for (i, v) in alpha.iter().enumerate() {
        if if foreground { *v >= 0.99 } else { *v <= 0.01 } {
            owner[i] = i;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        for j in [
            if x > 0 { Some(i - 1) } else { None },
            if x + 1 < w { Some(i + 1) } else { None },
            if y > 0 { Some(i - w) } else { None },
            if y + 1 < h { Some(i + w) } else { None },
        ]
        .into_iter()
        .flatten()
        {
            if owner[j] == usize::MAX {
                owner[j] = owner[i];
                queue.push_back(j);
            }
        }
    }
    owner
}
pub(super) fn decontaminate(frame: &Frame, alpha: &[f32], amount: f32, out: &mut Frame) {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let fg = nearest(alpha, w, h, true);
    let bg = nearest(alpha, w, h, false);
    out.pixels.par_iter_mut().enumerate().for_each(|(i, p)| {
        let a = alpha[i];
        if a <= 0.0 || a >= 1.0 || fg[i] == usize::MAX || bg[i] == usize::MAX {
            return;
        }
        // Regularized least squares: I = alpha*F + (1-alpha)*B, with local F prior.
        // This bounds amplification near transparent pixels and works in scene-linear RGB.
        let lambda = 0.01 * (1.0 - a).powi(2);
        for (c, v) in p[..3].iter_mut().enumerate() {
            let background = frame.pixels[bg[i]][c];
            let prior = frame.pixels[fg[i]][c];
            let recovered = (a * (frame.pixels[i][c] - (1.0 - a) * background) + lambda * prior)
                / (a * a + lambda).max(1e-8);
            *v += amount * (recovered - *v);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_form_recovers_known_linear_mixture_and_hard_constraints() {
        let (w, h) = (15, 7);
        let colors: Vec<_> = (0..w * h)
            .map(|i| {
                let a = (i % w) as f64 / (w - 1) as f64;
                [0.1 + 0.7 * a, 0.8 - 0.6 * a, 0.2 + 0.2 * a]
            })
            .collect();
        let trimap: Vec<f32> = (0..w * h)
            .map(|i| {
                if i % w == 0 {
                    0.0
                } else if i % w == w - 1 {
                    1.0
                } else {
                    0.5
                }
            })
            .collect();
        let options = Refinement {
            iterations: 500,
            tolerance: 1e-8,
            ..Default::default()
        };
        let (alpha, report) =
            solve(&colors, w, h, &trimap, &options, &AtomicBool::new(false)).unwrap();
        assert!(report.converged, "{report:?}");
        for (i, a) in alpha.iter().enumerate() {
            assert!(
                (*a - (i % w) as f32 / (w - 1) as f32).abs() < 2e-4,
                "pixel {i}: {a}"
            );
            if trimap[i] != 0.5 {
                assert_eq!(*a, trimap[i]);
            }
        }
    }
    #[test]
    fn decontamination_restores_linear_foreground() {
        let frame = Frame::new(
            3,
            1,
            vec![
                [0.0, 1.0, 0.0, 1.0],
                [0.5, 0.5, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
            ],
        )
        .unwrap();
        let mut out = frame.clone();
        decontaminate(&frame, &[0.0, 0.5, 1.0], 1.0, &mut out);
        assert!((out.pixels[1][0] - 1.0).abs() < 1e-6);
        assert!(out.pixels[1][1].abs() < 1e-6);
    }
}
