//! Weight-free image keying, interactive GrabCut, and closed-form alpha matting.
mod graphcut;
mod matting;

use crate::{Frame, Mask, mask::smooth, recipe::range};
use anyhow::{Result, ensure};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use vibecolor_color::{from_display, to_display};

fn tolerance() -> f32 {
    0.08
}
fn softness() -> f32 {
    0.12
}
fn iterations() -> u32 {
    5
}
fn edge() -> u32 {
    512
}
fn smoothness() -> f32 {
    50.0
}
fn radius() -> f32 {
    0.01
}
fn refine_radius() -> u32 {
    3
}
fn refine_edge() -> u32 {
    1024
}
fn refine_iterations() -> u32 {
    120
}
fn solver_tolerance() -> f64 {
    1e-6
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    /// Radius relative to the shorter image dimension; strokes join consecutive points.
    #[serde(default = "radius")]
    pub radius: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selection {
    /// Distance in clamped, encoded sRGB; colors/samples describe the background.
    Color {
        #[serde(default)]
        color: Option<[f32; 3]>,
        #[serde(default)]
        samples: Vec<[f32; 2]>,
        #[serde(default = "tolerance")]
        tolerance: f32,
        #[serde(default = "softness")]
        softness: f32,
    },
    GrabCut {
        /// [min_x, min_y, max_x, max_y], normalized; outside is certain background.
        #[serde(default)]
        rect: Option<[f32; 4]>,
        #[serde(default)]
        foreground: Vec<Stroke>,
        #[serde(default)]
        background: Vec<Stroke>,
        #[serde(default = "iterations")]
        iterations: u32,
        #[serde(default = "edge")]
        max_edge: u32,
        #[serde(default = "smoothness")]
        smoothness: f32,
    },
    Mask {
        mask: Box<Mask>,
    },
    /// Grayscale data: <=0.01 background, >=0.99 foreground, otherwise unknown.
    Trimap {
        path: String,
    },
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RefinementMethod {
    #[default]
    None,
    ClosedForm,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MattingSpace {
    /// Physical alpha mixtures are linear; normalize HDR magnitude without clipping.
    #[default]
    Linear,
    /// Compatibility with matting workflows operating on encoded sRGB samples.
    Srgb,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Refinement {
    #[serde(default)]
    pub method: RefinementMethod,
    #[serde(default)]
    pub space: MattingSpace,
    /// Unknown boundary band in refinement-resolution pixels (ignored for explicit trimaps).
    #[serde(default = "refine_radius")]
    pub radius: u32,
    #[serde(default = "refine_edge")]
    pub max_edge: u32,
    #[serde(default = "refine_iterations")]
    pub iterations: u32,
    #[serde(default = "solver_tolerance")]
    pub tolerance: f64,
}
impl Default for Refinement {
    fn default() -> Self {
        Self {
            method: RefinementMethod::None,
            space: MattingSpace::Linear,
            radius: refine_radius(),
            max_edge: refine_edge(),
            iterations: refine_iterations(),
            tolerance: solver_tolerance(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Despill {
    /// Encoded sRGB screen color; must have a dominant color channel.
    pub color: [f32; 3],
    pub amount: f32,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CutoutOutput {
    #[default]
    Cutout,
    Matte,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CutoutOptions {
    pub selection: Selection,
    #[serde(default)]
    pub refinement: Refinement,
    /// Signed square-kernel dilation/erosion radius in original input pixels.
    #[serde(default)]
    pub grow: i32,
    /// Gaussian sigma in original input pixels, applied after refinement/grow.
    #[serde(default)]
    pub feather: f32,
    /// Local foreground/background color reconstruction; 0 preserves RGB, 1 applies fully.
    #[serde(default)]
    pub decontaminate: f32,
    #[serde(default)]
    pub despill: Option<Despill>,
    #[serde(default)]
    pub output: CutoutOutput,
}
#[derive(Debug, Serialize)]
pub struct CutoutReport {
    pub algorithm_version: u32,
    pub model_weights_required: bool,
    pub segmentation_size: Option<[u32; 2]>,
    pub refinement_size: Option<[u32; 2]>,
    pub solver: Option<matting::SolveReport>,
    pub transparent_pixels: usize,
    pub opaque_pixels: usize,
    pub partial_pixels: usize,
    pub warnings: Vec<String>,
}
fn coord(p: [f32; 2]) -> Result<()> {
    for v in p {
        range(v, 0.0, 1.0, "cutout coordinate")?;
    }
    Ok(())
}
impl CutoutOptions {
    pub fn validate(&self) -> Result<()> {
        match &self.selection {
            Selection::Color {
                color,
                samples,
                tolerance,
                softness,
            } => {
                ensure!(
                    color.is_some() || !samples.is_empty(),
                    "color key needs a color or background samples"
                );
                ensure!(samples.len() <= 256, "at most 256 background samples");
                if let Some(c) = color {
                    for v in c {
                        range(*v, 0.0, 1.0, "key color")?;
                    }
                }
                for p in samples {
                    coord(*p)?;
                }
                range(*tolerance, 0.0, 1.0, "key tolerance")?;
                range(*softness, 0.00001, 1.0, "key softness")?;
            }
            Selection::GrabCut {
                rect,
                foreground,
                background,
                iterations,
                max_edge,
                smoothness,
            } => {
                if let Some(r) = rect {
                    coord([r[0], r[1]])?;
                    coord([r[2], r[3]])?;
                    ensure!(r[0] < r[2] && r[1] < r[3], "invalid cutout rectangle");
                }
                ensure!(
                    rect.is_some() || (!foreground.is_empty() && !background.is_empty()),
                    "GrabCut needs a rectangle or foreground and background strokes"
                );
                ensure!(
                    (1..=20).contains(iterations),
                    "GrabCut iterations must be 1..20"
                );
                ensure!(
                    (16..=1024).contains(max_edge),
                    "GrabCut max_edge must be 16..1024"
                );
                range(*smoothness, 0.0, 100.0, "GrabCut smoothness")?;
                ensure!(
                    foreground.len() + background.len() <= 256,
                    "at most 256 cutout strokes"
                );
                for s in foreground.iter().chain(background) {
                    ensure!(
                        !s.points.is_empty() && s.points.len() <= 4096,
                        "stroke requires 1..4096 points"
                    );
                    for p in &s.points {
                        coord(*p)?;
                    }
                    range(s.radius, 0.0, 0.25, "stroke radius")?;
                }
            }
            Selection::Mask { mask } => mask.validate(0)?,
            Selection::Trimap { path } => ensure!(!path.is_empty(), "empty trimap path"),
        }
        let r = &self.refinement;
        ensure!(r.radius <= 32, "refinement radius must be 0..32");
        ensure!(
            (16..=2048).contains(&r.max_edge),
            "refinement max_edge must be 16..2048"
        );
        ensure!(
            (1..=1000).contains(&r.iterations),
            "matting iterations must be 1..1000"
        );
        ensure!(
            r.tolerance.is_finite() && (1e-10..=0.01).contains(&r.tolerance),
            "matting tolerance must be 1e-10..0.01"
        );
        ensure!(
            (-32..=32).contains(&self.grow),
            "grow must be -32..32 pixels"
        );
        range(self.feather, 0.0, 32.0, "feather")?;
        range(self.decontaminate, 0.0, 1.0, "decontaminate")?;
        if let Some(d) = &self.despill {
            for v in d.color {
                range(v, 0.0, 1.0, "despill color")?;
            }
            range(d.amount, 0.0, 1.0, "despill amount")?;
            let mut c = d.color;
            c.sort_by(f32::total_cmp);
            ensure!(
                c[2] - c[1] > 0.05,
                "despill screen color needs a dominant channel"
            );
        }
        Ok(())
    }
    /// Shared by immutable project freezing, cache identity, and output protection.
    pub fn map_assets(&mut self, mut f: impl FnMut(&str) -> Result<String>) -> Result<()> {
        fn visit(m: &mut Mask, f: &mut impl FnMut(&str) -> Result<String>) -> Result<()> {
            match m {
                Mask::Bitmap { path } => *path = f(path)?,
                Mask::Combine { masks, .. } => {
                    for m in masks {
                        visit(m, f)?;
                    }
                }
                Mask::Invert { mask } => visit(mask, f)?,
                _ => {}
            }
            Ok(())
        }
        match &mut self.selection {
            Selection::Trimap { path } => *path = f(path)?,
            Selection::Mask { mask } => visit(mask, &mut f)?,
            _ => {}
        }
        Ok(())
    }
    pub fn assets(&self) -> Vec<String> {
        let mut paths = Vec::new();
        self.clone()
            .map_assets(|p| {
                paths.push(p.to_string());
                Ok(p.to_string())
            })
            .expect("infallible collection");
        paths
    }
}
pub(crate) fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "cutout cancelled");
    Ok(())
}
pub(crate) fn rgb(frame: &Frame) -> Vec<[f64; 3]> {
    frame
        .pixels
        .iter()
        .map(|p| to_display([p[0], p[1], p[2]]).map(|v| v.clamp(0.0, 1.0) as f64))
        .collect()
}
fn sample_coord(p: [f32; 2], w: usize, h: usize) -> usize {
    let x = ((p[0] * w as f32).floor() as usize).min(w - 1);
    let y = ((p[1] * h as f32).floor() as usize).min(h - 1);
    y * w + x
}
/// Label 0: free, 1: hard background, 2: hard foreground. Conflicts are rejected.
pub(crate) fn hard_labels(
    w: usize,
    h: usize,
    rect: Option<[f32; 4]>,
    fg: &[Stroke],
    bg: &[Stroke],
) -> Result<Vec<u8>> {
    let mut labels = vec![0; w * h];
    if let Some(r) = rect {
        for (i, v) in labels.iter_mut().enumerate() {
            let x = (i % w) as f32 + 0.5;
            let y = (i / w) as f32 + 0.5;
            if x / (w as f32) < r[0]
                || y / (h as f32) < r[1]
                || x / (w as f32) >= r[2]
                || y / (h as f32) >= r[3]
            {
                *v = 1;
            }
        }
    }
    for (strokes, label) in [(bg, 1), (fg, 2)] {
        for s in strokes {
            let points: Vec<[f32; 2]> = s
                .points
                .iter()
                .map(|p| [p[0] * w as f32 - 0.5, p[1] * h as f32 - 0.5])
                .collect();
            let radius = (s.radius * w.min(h) as f32).max(0.71);
            for j in 0..points.len().max(2) - 1 {
                let a = points[j.min(points.len() - 1)];
                let b = points[(j + 1).min(points.len() - 1)];
                let minx = (a[0].min(b[0]) - radius).floor().max(0.0) as usize;
                let maxx = (a[0].max(b[0]) + radius)
                    .ceil()
                    .max(0.0)
                    .min((w - 1) as f32) as usize;
                let miny = (a[1].min(b[1]) - radius).floor().max(0.0) as usize;
                let maxy = (a[1].max(b[1]) + radius)
                    .ceil()
                    .max(0.0)
                    .min((h - 1) as f32) as usize;
                let v = [b[0] - a[0], b[1] - a[1]];
                let len = v[0] * v[0] + v[1] * v[1];
                for y in miny..=maxy {
                    for x in minx..=maxx {
                        let t = if len > 1e-12 {
                            (((x as f32 - a[0]) * v[0] + (y as f32 - a[1]) * v[1]) / len)
                                .clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        if (x as f32 - a[0] - t * v[0]).powi(2)
                            + (y as f32 - a[1] - t * v[1]).powi(2)
                            <= radius * radius
                        {
                            let p = &mut labels[y * w + x];
                            ensure!(
                                *p == 0 || *p == label,
                                "conflicting foreground/background strokes or foreground outside rectangle"
                            );
                            *p = label;
                        }
                    }
                }
            }
        }
    }
    Ok(labels)
}
pub(crate) fn resize_values(
    values: &[f32],
    w: usize,
    h: usize,
    nw: usize,
    nh: usize,
    nearest: bool,
) -> Vec<f32> {
    (0..nw * nh)
        .into_par_iter()
        .map(|i| {
            let x = ((i % nw) as f32 + 0.5) * w as f32 / nw as f32 - 0.5;
            let y = ((i / nw) as f32 + 0.5) * h as f32 / nh as f32 - 0.5;
            if nearest {
                return values[(y.round().clamp(0.0, (h - 1) as f32) as usize) * w
                    + x.round().clamp(0.0, (w - 1) as f32) as usize];
            }
            let x = x.clamp(0.0, (w - 1) as f32);
            let y = y.clamp(0.0, (h - 1) as f32);
            let x0 = x.floor() as usize;
            let y0 = y.floor() as usize;
            let tx = x - x0 as f32;
            let ty = y - y0 as f32;
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            (values[y0 * w + x0] * (1.0 - tx) + values[y0 * w + x1] * tx) * (1.0 - ty)
                + (values[y1 * w + x0] * (1.0 - tx) + values[y1 * w + x1] * tx) * ty
        })
        .collect()
}
/// O(N) separable square erosion/dilation using monotonic queues, clamped borders.
pub(crate) fn morphology(a: &[f32], w: usize, h: usize, r: usize, dilate: bool) -> Vec<f32> {
    if r == 0 {
        return a.to_vec();
    }
    let pass = |src: &[f32], horizontal: bool| {
        let (lines, len) = if horizontal { (h, w) } else { (w, h) };
        let rows: Vec<Vec<f32>> = (0..lines)
            .into_par_iter()
            .map(|line| {
                let at = |i: usize| {
                    if horizontal {
                        src[line * w + i]
                    } else {
                        src[i * w + line]
                    }
                };
                let mut q = std::collections::VecDeque::<usize>::new();
                let mut row = vec![0.0; len];
                let mut next = 0;
                for (i, out) in row.iter_mut().enumerate() {
                    let right = (i + r).min(len - 1);
                    while next <= right {
                        while q.back().is_some_and(|j| {
                            if dilate {
                                at(*j) <= at(next)
                            } else {
                                at(*j) >= at(next)
                            }
                        }) {
                            q.pop_back();
                        }
                        q.push_back(next);
                        next += 1;
                    }
                    while q.front().is_some_and(|j| *j < i.saturating_sub(r)) {
                        q.pop_front();
                    }
                    *out = at(*q.front().expect("nonempty window"));
                }
                row
            })
            .collect();
        let mut out = vec![0.0; w * h];
        for (line, row) in rows.into_iter().enumerate() {
            for (i, v) in row.into_iter().enumerate() {
                out[if horizontal {
                    line * w + i
                } else {
                    i * w + line
                }] = v;
            }
        }
        out
    };
    pass(&pass(a, true), false)
}

/// Callback reads scalar matte data at EXACT input dimensions, with no transfer conversion.
pub fn run(
    frame: &Frame,
    options: &CutoutOptions,
    load: &impl Fn(&str, u32, u32) -> Result<Vec<f32>>,
    cancel: &AtomicBool,
) -> Result<(Frame, CutoutReport)> {
    options.validate()?;
    cancelled(cancel)?;
    ensure!(
        frame.width > 0
            && frame.height > 0
            && frame.width as u64 * frame.height as u64 == frame.pixels.len() as u64,
        "invalid cutout input dimensions"
    );
    ensure!(
        frame.pixels.iter().flatten().all(|v| v.is_finite())
            && frame.pixels.iter().all(|p| (0.0..=1.0).contains(&p[3])),
        "cutout input must have finite RGB and alpha in [0,1]"
    );
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut report = CutoutReport {
        algorithm_version: 1,
        model_weights_required: false,
        segmentation_size: None,
        refinement_size: None,
        solver: None,
        transparent_pixels: 0,
        opaque_pixels: 0,
        partial_pixels: 0,
        warnings: vec![],
    };
    let explicit_trimap = matches!(options.selection, Selection::Trimap { .. });
    let mut alpha = match &options.selection {
        Selection::Color {
            color,
            samples,
            tolerance,
            softness,
        } => {
            let colors: Vec<[f32; 3]> = color
                .iter()
                .copied()
                .chain(samples.iter().map(|p| {
                    let v = frame.pixels[sample_coord(*p, w, h)];
                    to_display([v[0], v[1], v[2]]).map(|x| x.clamp(0.0, 1.0))
                }))
                .collect();
            frame
                .pixels
                .par_iter()
                .map(|p| {
                    let c = to_display([p[0], p[1], p[2]]).map(|v| v.clamp(0.0, 1.0));
                    let d = colors
                        .iter()
                        .map(|k| ((0..3).map(|j| (c[j] - k[j]).powi(2)).sum::<f32>() / 3.0).sqrt())
                        .fold(f32::INFINITY, f32::min);
                    smooth(*tolerance, tolerance + softness, d)
                })
                .collect()
        }
        Selection::GrabCut {
            rect,
            foreground,
            background,
            iterations,
            max_edge,
            smoothness,
        } => {
            let small = frame.resized(*max_edge)?;
            let (sw, sh) = (small.width as usize, small.height as usize);
            let labels = hard_labels(sw, sh, *rect, foreground, background)?;
            let values = graphcut::segment(
                &rgb(&small),
                sw,
                sh,
                &labels,
                *iterations,
                *smoothness as f64,
                cancel,
            )?;
            report.segmentation_size = Some([small.width, small.height]);
            if sw != w || sh != h {
                report.warnings.push("GrabCut uses a reduced image; features smaller than its working pixels can be lost.".into());
            }
            resize_values(&values, sw, sh, w, h, false)
        }
        Selection::Mask { mask } => mask.rasterize(frame, load)?,
        Selection::Trimap { path } => load(path, frame.width, frame.height)?,
    };
    ensure!(
        alpha.len() == w * h
            && alpha
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "invalid cutout matte data"
    );
    let original_trimap = explicit_trimap.then(|| alpha.clone());
    if options.refinement.method == RefinementMethod::ClosedForm || explicit_trimap {
        let small = frame.resized(options.refinement.max_edge)?;
        let (sw, sh) = (small.width as usize, small.height as usize);
        let a = resize_values(&alpha, w, h, sw, sh, explicit_trimap);
        let mut trimap = if explicit_trimap {
            a.iter()
                .map(|v| {
                    if *v <= 0.01 {
                        0.0
                    } else if *v >= 0.99 {
                        1.0
                    } else {
                        0.5
                    }
                })
                .collect()
        } else {
            let binary: Vec<_> = a
                .iter()
                .map(|v| if *v >= 0.5 { 1.0 } else { 0.0 })
                .collect();
            let lo = morphology(&binary, sw, sh, options.refinement.radius as usize, false);
            let hi = morphology(&binary, sw, sh, options.refinement.radius as usize, true);
            lo.iter()
                .zip(hi)
                .zip(&a)
                .map(|((lo, hi), v)| {
                    if *lo == hi && (*v <= 0.01 || *v >= 0.99) {
                        *lo
                    } else {
                        0.5
                    }
                })
                .collect::<Vec<_>>()
        };
        if let Selection::GrabCut {
            rect,
            foreground,
            background,
            ..
        } = &options.selection
        {
            for (v, label) in trimap
                .iter_mut()
                .zip(hard_labels(sw, sh, *rect, foreground, background)?)
            {
                if label > 0 {
                    *v = if label == 2 { 1.0 } else { 0.0 };
                }
            }
        }
        let (a, solver) = matting::solve(
            &matting::colors(&small, options.refinement.space),
            sw,
            sh,
            &trimap,
            &options.refinement,
            cancel,
        )?;
        if !solver.converged {
            report.warnings.push("Closed-form solver reached the iteration limit; inspect the matte or increase iterations.".into());
        }
        alpha = matting::upsample(&a, &small, frame, options.refinement.space);
        report.refinement_size = Some([small.width, small.height]);
        report.solver = Some(solver);
        if sw != w || sh != h {
            report.warnings.push("Matting uses a reduced image and joint bilateral upsampling; increase refinement.max_edge for finer detail.".into());
        }
        if let Some(known) = &original_trimap {
            for (v, k) in alpha.iter_mut().zip(known) {
                if *k <= 0.01 {
                    *v = 0.0;
                } else if *k >= 0.99 {
                    *v = 1.0;
                }
            }
        }
    }
    cancelled(cancel)?;
    if options.grow != 0 {
        alpha = morphology(
            &alpha,
            w,
            h,
            options.grow.unsigned_abs() as usize,
            options.grow > 0,
        );
    }
    if options.feather > 0.0 {
        alpha = matting::feather(&alpha, w, h, options.feather);
    }
    // Restore hard user labels after refinement/grow/feather: user marks remain authoritative.
    if let Selection::GrabCut {
        rect,
        foreground,
        background,
        ..
    } = &options.selection
    {
        for (v, label) in alpha
            .iter_mut()
            .zip(hard_labels(w, h, *rect, foreground, background)?)
        {
            if label > 0 {
                *v = if label == 2 { 1.0 } else { 0.0 };
            }
        }
    }
    let mut out = frame.clone();
    if options.decontaminate > 0.0 {
        matting::decontaminate(frame, &alpha, options.decontaminate, &mut out);
        report.warnings.push("Foreground color reconstruction uses nearby opaque foreground/background; inspect thin or fully translucent regions.".into());
    }
    out.pixels.par_iter_mut().enumerate().for_each(|(i, p)| {
        let a = (alpha[i] * frame.pixels[i][3]).clamp(0.0, 1.0);
        if options.output == CutoutOutput::Matte {
            *p = [a, a, a, 1.0];
            return;
        }
        if let Some(d) = &options.despill {
            let channel = (0..3)
                .max_by(|i, j| d.color[*i].total_cmp(&d.color[*j]))
                .unwrap();
            let mut c = to_display([p[0], p[1], p[2]]);
            let other = (0..3)
                .filter(|j| *j != channel)
                .map(|j| c[j])
                .fold(f32::NEG_INFINITY, f32::max);
            c[channel] -= (c[channel] - other).max(0.0) * d.amount;
            p[..3].copy_from_slice(&from_display(c));
        }
        p[3] = a;
        if a == 0.0 {
            p[..3].fill(0.0);
        }
    });
    for (a, p) in alpha.iter().zip(&frame.pixels) {
        let v = a * p[3];
        if v == 0.0 {
            report.transparent_pixels += 1;
        } else if v >= 1.0 {
            report.opaque_pixels += 1;
        } else {
            report.partial_pixels += 1;
        }
    }
    ensure!(
        out.pixels.iter().flatten().all(|v| v.is_finite()),
        "cutout produced non-finite values"
    );
    cancelled(cancel)?;
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn morphology_matches_brute_force_for_large_kernels_and_clamped_borders() {
        let (w, h) = (7, 5);
        let input: Vec<f32> = (0..w * h)
            .map(|i| ((i * 13 + 5) % 31) as f32 / 30.0)
            .collect();
        for r in [0, 1, 2, 8] {
            for dilate in [false, true] {
                let actual = morphology(&input, w, h, r, dilate);
                for (i, a) in actual.iter().enumerate() {
                    let (x, y) = (i % w, i / w);
                    let mut expected = if dilate { 0.0_f32 } else { 1.0_f32 };
                    for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                        for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                            expected = if dilate {
                                expected.max(input[yy * w + xx])
                            } else {
                                expected.min(input[yy * w + xx])
                            };
                        }
                    }
                    assert_eq!(*a, expected, "radius {r}, dilation {dilate}, pixel {i}");
                }
            }
        }
    }
}
