//! GrabCut: deterministic 5-component full-covariance GMMs + 8-neighbor min cut.
//! Rother, Kolmogorov, Blake, SIGGRAPH 2004. No pretrained parameters.
use super::{cancelled, matting::invert3};
use anyhow::{Result, ensure};
use std::{collections::VecDeque, sync::atomic::AtomicBool};

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|c| (a[c] - b[c]).powi(2)).sum()
}
#[derive(Clone)]
struct Gaussian {
    mean: [f64; 3],
    inv: [[f64; 3]; 3],
    determinant: f64,
    weight: f64,
}
impl Gaussian {
    fn cost(&self, p: [f64; 3]) -> f64 {
        let v = std::array::from_fn::<_, 3, _>(|c| p[c] - self.mean[c]);
        let mahal: f64 = (0..3)
            .map(|r| v[r] * (0..3).map(|c| self.inv[r][c] * v[c]).sum::<f64>())
            .sum();
        -self.weight.max(1e-300).ln() + 0.5 * self.determinant.ln() + 0.5 * mahal
    }
}
struct Gmm(Vec<Gaussian>);
impl Gmm {
    fn from_assignments(pixels: &[[f64; 3]], members: &[usize], assignment: &[usize]) -> Self {
        let k = assignment.iter().max().copied().unwrap_or(0) + 1;
        let mut counts = vec![0; k];
        let mut means = vec![[0.0; 3]; k];
        for (&i, &j) in members.iter().zip(assignment) {
            counts[j] += 1;
            for c in 0..3 {
                means[j][c] += pixels[i][c];
            }
        }
        for (m, n) in means.iter_mut().zip(&counts) {
            if *n > 0 {
                for v in m {
                    *v /= *n as f64;
                }
            }
        }
        let mut cov = vec![[[0.0; 3]; 3]; k];
        for (&i, &j) in members.iter().zip(assignment) {
            for r in 0..3 {
                for c in 0..3 {
                    cov[j][r][c] += (pixels[i][r] - means[j][r]) * (pixels[i][c] - means[j][c]);
                }
            }
        }
        Self(
            (0..k)
                .filter(|j| counts[*j] > 0)
                .map(|j| {
                    for (r, row) in cov[j].iter_mut().enumerate() {
                        for value in row.iter_mut() {
                            *value /= counts[j] as f64;
                        }
                        row[r] += 1e-5;
                    }
                    let (inv, determinant) = invert3(cov[j]);
                    Gaussian {
                        mean: means[j],
                        inv,
                        determinant,
                        weight: counts[j] as f64 / members.len() as f64,
                    }
                })
                .collect(),
        )
    }
    fn init(pixels: &[[f64; 3]], members: &[usize]) -> Self {
        let mut centers = vec![pixels[members[0]]];
        for _ in 1..5.min(members.len()) {
            let (&i, d) = members
                .iter()
                .map(|i| {
                    (
                        i,
                        centers
                            .iter()
                            .map(|c| distance(pixels[*i], *c))
                            .fold(f64::INFINITY, f64::min),
                    )
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            if d < 1e-12 {
                break;
            }
            centers.push(pixels[i]);
        }
        let mut assignment = vec![0; members.len()];
        for _ in 0..8 {
            let mut sum = vec![[0.0; 3]; centers.len()];
            let mut counts = vec![0; centers.len()];
            for (a, &i) in assignment.iter_mut().zip(members) {
                *a = (0..centers.len())
                    .min_by(|a, b| {
                        distance(pixels[i], centers[*a])
                            .total_cmp(&distance(pixels[i], centers[*b]))
                    })
                    .unwrap();
                counts[*a] += 1;
                for c in 0..3 {
                    sum[*a][c] += pixels[i][c];
                }
            }
            for j in 0..centers.len() {
                if counts[j] > 0 {
                    centers[j] = sum[j].map(|v| v / counts[j] as f64);
                }
            }
        }
        Self::from_assignments(pixels, members, &assignment)
    }
    fn update(&self, pixels: &[[f64; 3]], members: &[usize]) -> Self {
        let assignment: Vec<_> = members
            .iter()
            .map(|i| {
                (0..self.0.len())
                    .min_by(|a, b| {
                        self.0[*a]
                            .cost(pixels[*i])
                            .total_cmp(&self.0[*b].cost(pixels[*i]))
                    })
                    .unwrap()
            })
            .collect();
        Self::from_assignments(pixels, members, &assignment)
    }
    fn cost(&self, p: [f64; 3]) -> f64 {
        // Log-sum-exp of the mixture density; common Gaussian normalization cancels.
        let costs: Vec<_> = self.0.iter().map(|g| g.cost(p)).collect();
        let min = costs.iter().copied().fold(f64::INFINITY, f64::min);
        (min - costs.iter().map(|v| (min - v).exp()).sum::<f64>().ln()).clamp(-100.0, 10000.0)
    }
}

const END: u32 = u32::MAX;
#[derive(Clone, Copy)]
struct Edge {
    to: u32,
    next: u32,
    cap: f64,
}
struct Graph {
    head: Vec<u32>,
    edges: Vec<Edge>,
}
impl Graph {
    fn new(n: usize) -> Self {
        Self {
            head: vec![END; n],
            edges: Vec::with_capacity(n * 12),
        }
    }
    fn add(&mut self, a: usize, b: usize, forward: f64, reverse: f64) {
        let j = self.edges.len() as u32;
        self.edges.push(Edge {
            to: b as u32,
            next: self.head[a],
            cap: forward,
        });
        self.edges.push(Edge {
            to: a as u32,
            next: self.head[b],
            cap: reverse,
        });
        self.head[a] = j;
        self.head[b] = j + 1;
    }
    fn relabel_all(&self, source: usize, sink: usize) -> Vec<usize> {
        let n = self.head.len();
        let mut height = vec![n + 1; n];
        height[sink] = 0;
        let mut queue = VecDeque::from([sink]);
        while let Some(v) = queue.pop_front() {
            let mut e = self.head[v];
            while e != END {
                let edge = self.edges[e as usize];
                let to = edge.to as usize;
                if to != source && height[to] == n + 1 && self.edges[e as usize ^ 1].cap > 1e-10 {
                    height[to] = height[v] + 1;
                    queue.push_back(to);
                }
                e = edge.next;
            }
        }
        // Also label sink-disconnected vertices by residual distance BACK to source.
        // Resetting all of these to n+1 can repeatedly undo return-flow progress.
        height[source] = n;
        queue.push_back(source);
        let mut seen = vec![false; n];
        seen[source] = true;
        while let Some(v) = queue.pop_front() {
            let mut e = self.head[v];
            while e != END {
                let edge = self.edges[e as usize];
                let to = edge.to as usize;
                if !seen[to] && height[to] > n && self.edges[e as usize ^ 1].cap > 1e-10 {
                    seen[to] = true;
                    height[to] = height[v] + 1;
                    queue.push_back(to);
                }
                e = edge.next;
            }
        }
        height
    }
    fn cut(&mut self, source: usize, sink: usize, cancel: &AtomicBool) -> Result<Vec<bool>> {
        let n = self.head.len();
        let mut excess = vec![0.0; n];
        let mut e = self.head[source];
        while e != END {
            let edge = self.edges[e as usize];
            self.edges[e as usize].cap = 0.0;
            self.edges[e as usize ^ 1].cap += edge.cap;
            excess[edge.to as usize] += edge.cap;
            e = edge.next;
        }
        let mut height = self.relabel_all(source, sink);
        let mut current = self.head.clone();
        let mut queued = vec![false; n];
        let mut queue = VecDeque::new();
        for v in 0..n {
            if v != source && v != sink && excess[v] > 1e-9 {
                queue.push_back(v);
                queued[v] = true;
            }
        }
        let mut work = 0usize;
        while let Some(v) = queue.pop_front() {
            queued[v] = false;
            while excess[v] > 1e-9 {
                let j = current[v];
                if j == END {
                    let mut lowest = usize::MAX;
                    let mut edge = self.head[v];
                    while edge != END {
                        let q = self.edges[edge as usize];
                        if q.cap > 1e-10 {
                            lowest = lowest.min(height[q.to as usize]);
                        }
                        edge = q.next;
                        work += 1;
                    }
                    ensure!(lowest != usize::MAX, "invalid residual graph");
                    height[v] = lowest + 1;
                    current[v] = self.head[v];
                    continue;
                }
                let q = self.edges[j as usize];
                let to = q.to as usize;
                if q.cap > 1e-10 && height[v] == height[to] + 1 {
                    let flow = excess[v].min(q.cap);
                    self.edges[j as usize].cap -= flow;
                    self.edges[j as usize ^ 1].cap += flow;
                    excess[v] -= flow;
                    excess[to] += flow;
                    if to != source && to != sink && !queued[to] && excess[to] > 1e-9 {
                        queue.push_back(to);
                        queued[to] = true;
                    }
                } else {
                    current[v] = q.next;
                }
                work += 1;
                if work.is_multiple_of(8192) {
                    cancelled(cancel)?;
                }
            }
            if work > self.edges.len() * 2 {
                cancelled(cancel)?;
                height = self.relabel_all(source, sink);
                current.clone_from(&self.head);
                queue.clear();
                queued.fill(false);
                for u in 0..n {
                    if u != source && u != sink && excess[u] > 1e-9 {
                        queue.push_back(u);
                        queued[u] = true;
                    }
                }
                work = 0;
            }
        }
        let mut reachable = vec![false; n];
        reachable[source] = true;
        queue.push_back(source);
        while let Some(v) = queue.pop_front() {
            let mut e = self.head[v];
            while e != END {
                let q = self.edges[e as usize];
                let to = q.to as usize;
                if q.cap > 1e-9 && !reachable[to] {
                    reachable[to] = true;
                    queue.push_back(to);
                }
                e = q.next;
            }
        }
        Ok(reachable)
    }
}
pub(super) fn segment(
    pixels: &[[f64; 3]],
    w: usize,
    h: usize,
    hard: &[u8],
    iterations: u32,
    smoothness: f64,
    cancel: &AtomicBool,
) -> Result<Vec<f32>> {
    let n = w * h;
    ensure!(n >= 4, "GrabCut requires at least four pixels");
    let mut labels: Vec<bool> = hard.iter().map(|v| *v != 1).collect();
    let members = |labels: &[bool], fg: bool| {
        labels
            .iter()
            .enumerate()
            .filter(|(_, v)| **v == fg)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let bg = members(&labels, false);
    let fg = members(&labels, true);
    ensure!(
        !bg.is_empty() && !fg.is_empty(),
        "GrabCut needs both foreground and background pixels; use an inset rectangle or strokes"
    );
    let mut bg_model = Gmm::init(pixels, &bg);
    let mut fg_model = Gmm::init(pixels, &fg);
    let mut neighbors = Vec::with_capacity(n * 4);
    let mut sum = 0.0;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            for (dx, dy) in [(1, 0), (0, 1), (1, 1), (-1, 1)] {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx >= 0 && xx < w as isize && yy < h as isize {
                    let j = yy as usize * w + xx as usize;
                    let d = distance(pixels[i], pixels[j]);
                    sum += d;
                    neighbors.push((
                        i,
                        j,
                        d,
                        if dx != 0 && dy != 0 {
                            std::f64::consts::SQRT_2
                        } else {
                            1.0
                        },
                    ));
                }
            }
        }
    }
    let beta = if sum > 1e-12 {
        neighbors.len() as f64 / (2.0 * sum)
    } else {
        0.0
    };
    for item in &mut neighbors {
        item.2 = smoothness * (-beta * item.2).exp() / item.3;
    }
    for _ in 0..iterations {
        cancelled(cancel)?;
        let bg = members(&labels, false);
        let fg = members(&labels, true);
        ensure!(
            !bg.is_empty() && !fg.is_empty(),
            "GrabCut removed a class; add a foreground/background stroke"
        );
        bg_model = bg_model.update(pixels, &bg);
        fg_model = fg_model.update(pixels, &fg);
        let (source, sink) = (n, n + 1);
        let mut graph = Graph::new(n + 2);
        for i in 0..n {
            let (mut cb, mut cf) = match hard[i] {
                1 => (0.0, 1e6),
                2 => (1e6, 0.0),
                _ => (bg_model.cost(pixels[i]), fg_model.cost(pixels[i])),
            };
            let offset = cb.min(cf).min(0.0);
            cb -= offset;
            cf -= offset;
            graph.add(source, i, cb, 0.0);
            graph.add(i, sink, cf, 0.0);
        }
        for &(a, b, weight, _) in &neighbors {
            graph.add(a, b, weight, weight);
        }
        let cut = graph.cut(source, sink, cancel)?;
        let next = cut[..n].to_vec();
        let unchanged = next == labels;
        labels = next;
        ensure!(
            labels
                .iter()
                .zip(hard)
                .all(|(v, k)| *k == 0 || *v == (*k == 2)),
            "mincut violated hard labels"
        );
        if unchanged {
            break;
        }
    }
    ensure!(
        labels.iter().any(|v| *v) && labels.iter().any(|v| !*v),
        "GrabCut result has no foreground or background; add strokes"
    );
    Ok(labels.iter().map(|v| if *v { 1.0 } else { 0.0 }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mincut_matches_exhaustive_energy() {
        // Independent enumeration of every labeling, including asymmetric links.
        for fixture in 0..24 {
            let unary: Vec<(f64, f64)> = (0..5)
                .map(|i| {
                    (
                        ((i * 7 + fixture * 3) % 13) as f64 / 3.0,
                        ((i * 5 + fixture * 11) % 17) as f64 / 4.0,
                    )
                })
                .collect();
            let links = [
                (0, 1, 1.1, 0.2),
                (1, 2, 0.6, 1.7),
                (2, 3, 2.3, 0.8),
                (3, 4, 0.9, 1.2),
                (0, 4, 0.4, 0.7),
            ];
            let energy = |bits: usize| {
                let node = (0..5)
                    .map(|i| {
                        if bits & (1 << i) != 0 {
                            unary[i].1
                        } else {
                            unary[i].0
                        }
                    })
                    .sum::<f64>();
                node + links
                    .iter()
                    .map(|(a, b, f, r)| {
                        if bits & (1 << a) != 0 && bits & (1 << b) == 0 {
                            *f
                        } else if bits & (1 << a) == 0 && bits & (1 << b) != 0 {
                            *r
                        } else {
                            0.0
                        }
                    })
                    .sum::<f64>()
            };
            let mut graph = Graph::new(7);
            for (i, (bg, fg)) in unary.iter().enumerate() {
                graph.add(5, i, *bg, 0.0);
                graph.add(i, 6, *fg, 0.0);
            }
            for (a, b, f, r) in links {
                graph.add(a, b, f, r);
            }
            let cut = graph.cut(5, 6, &AtomicBool::new(false)).unwrap();
            let bits = (0..5).fold(0, |v, i| v | ((cut[i] as usize) << i));
            let min = (0..32).map(energy).fold(f64::INFINITY, f64::min);
            assert!((energy(bits) - min).abs() < 1e-8, "fixture {fixture}");
        }
    }
}
