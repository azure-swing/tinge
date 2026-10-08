use anyhow::{Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LutInterpolation {
    #[default]
    Trilinear,
    Tetrahedral,
}
pub fn is_trilinear(value: &LutInterpolation) -> bool {
    *value == LutInterpolation::Trilinear
}

#[derive(Debug, Clone)]
struct Table {
    size: usize,
    min: [f32; 3],
    max: [f32; 3],
    values: Vec<[f32; 3]>,
}
#[derive(Debug, Clone)]
pub struct CubeLut {
    title: Option<String>,
    shaper: Option<Table>,
    cube: Option<Table>,
}
#[derive(Debug, Serialize)]
pub struct CubeInfo {
    pub title: Option<String>,
    pub size_1d: Option<usize>,
    pub size_3d: Option<usize>,
    pub domain_1d: Option<[[f32; 3]; 2]>,
    pub domain_3d: Option<[[f32; 3]; 2]>,
    pub combined: bool,
    pub row_count: usize,
}
fn domain(min: [f32; 3], max: [f32; 3]) -> Result<()> {
    ensure!(
        (0..3).all(|c| min[c].is_finite() && max[c].is_finite() && min[c] < max[c]),
        "invalid LUT domain"
    );
    Ok(())
}
fn size(value: usize, is_3d: bool) -> Result<()> {
    ensure!(
        (2..=if is_3d { 129 } else { 65536 }).contains(&value),
        "LUT size out of range"
    );
    Ok(())
}
impl CubeLut {
    pub fn parse(text: &str) -> Result<Self> {
        let (mut size1d, mut size3d) = (None, None);
        let (mut minimum, mut maximum) = (None, None);
        let (mut range1d, mut range3d) = (None, None);
        let mut title = None;
        let mut values = Vec::new();
        for (line_no, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let first = raw.split_whitespace().next().unwrap();
            let directive = first.to_ascii_uppercase();
            if directive == "TITLE" {
                ensure!(
                    values.is_empty() && title.is_none(),
                    "duplicate/late TITLE at line {}",
                    line_no + 1
                );
                let value = raw[first.len()..]
                    .trim()
                    .strip_prefix('"')
                    .ok_or_else(|| anyhow::anyhow!("TITLE must be quoted"))?;
                let end = value
                    .find('"')
                    .ok_or_else(|| anyhow::anyhow!("unterminated TITLE"))?;
                let tail = value[end + 1..].trim();
                ensure!(
                    tail.is_empty() || tail.starts_with('#'),
                    "invalid TITLE suffix"
                );
                title = Some(value[..end].to_string());
                continue;
            }
            let line = raw.split('#').next().unwrap().trim();
            let parts: Vec<_> = line.split_whitespace().collect();
            let header = matches!(
                directive.as_str(),
                "LUT_1D_SIZE"
                    | "LUT_3D_SIZE"
                    | "DOMAIN_MIN"
                    | "DOMAIN_MAX"
                    | "LUT_1D_INPUT_RANGE"
                    | "LUT_3D_INPUT_RANGE"
            );
            ensure!(
                !header || values.is_empty(),
                "LUT header after data at line {}",
                line_no + 1
            );
            match directive.as_str() {
                "LUT_1D_SIZE" | "LUT_3D_SIZE" => {
                    ensure!(parts.len() == 2, "invalid LUT size at line {}", line_no + 1);
                    let is_3d = directive == "LUT_3D_SIZE";
                    let target = if is_3d { &mut size3d } else { &mut size1d };
                    ensure!(target.is_none(), "duplicate LUT size");
                    let n = parts[1].parse::<usize>()?;
                    size(n, is_3d)?;
                    *target = Some(n);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    ensure!(
                        parts.len() == 4,
                        "invalid LUT domain at line {}",
                        line_no + 1
                    );
                    let target = if directive == "DOMAIN_MIN" {
                        &mut minimum
                    } else {
                        &mut maximum
                    };
                    ensure!(target.is_none(), "duplicate LUT domain");
                    *target = Some([parts[1].parse()?, parts[2].parse()?, parts[3].parse()?]);
                }
                "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                    ensure!(
                        parts.len() == 3,
                        "invalid LUT input range at line {}",
                        line_no + 1
                    );
                    let target = if directive == "LUT_1D_INPUT_RANGE" {
                        &mut range1d
                    } else {
                        &mut range3d
                    };
                    ensure!(target.is_none(), "duplicate LUT input range");
                    *target = Some([parts[1].parse::<f32>()?, parts[2].parse::<f32>()?]);
                }
                _ => {
                    if parts[0].parse::<f32>().is_err() {
                        bail!("unsupported LUT directive {}", parts[0]);
                    }
                    ensure!(parts.len() == 3, "invalid LUT row at line {}", line_no + 1);
                    let count = size1d.unwrap_or(0) + size3d.map(|n: usize| n.pow(3)).unwrap_or(0);
                    ensure!(
                        count > 0 && values.len() < count,
                        "missing LUT size or excess rows at line {}",
                        line_no + 1
                    );
                    let row: [f32; 3] = [parts[0].parse()?, parts[1].parse()?, parts[2].parse()?];
                    ensure!(row.iter().all(|v| v.is_finite()), "non-finite LUT data");
                    values.push(row);
                }
            }
        }
        ensure!(size1d.is_some() || size3d.is_some(), "missing LUT size");
        ensure!(
            values.len() == size1d.unwrap_or(0) + size3d.map(|n| n.pow(3)).unwrap_or(0),
            "LUT data count does not match size"
        );
        let iridas_domain = minimum.is_some() || maximum.is_some();
        let combined = size1d.is_some() && size3d.is_some();
        ensure!(
            !iridas_domain || (!combined && range1d.is_none() && range3d.is_none()),
            "DOMAIN and Resolve INPUT_RANGE/combined headers cannot be mixed"
        );
        ensure!(
            !combined || title.is_none(),
            "Resolve combined LUT does not support TITLE; use a header comment"
        );
        ensure!(
            range1d.is_none() || size1d.is_some(),
            "1D input range without a 1D LUT"
        );
        ensure!(
            range3d.is_none() || size3d.is_some(),
            "3D input range without a 3D LUT"
        );
        ensure!(
            title.is_none() || (range1d.is_none() && range3d.is_none()),
            "Resolve INPUT_RANGE files do not support TITLE"
        );
        let min = minimum.unwrap_or([0.0; 3]);
        let max = maximum.unwrap_or([1.0; 3]);
        domain(min, max)?;
        let table = |n: usize, range: Option<[f32; 2]>, rows: Vec<[f32; 3]>| -> Result<Table> {
            let (min, max) = range.map(|r| ([r[0]; 3], [r[1]; 3])).unwrap_or((min, max));
            domain(min, max)?;
            Ok(Table {
                size: n,
                min,
                max,
                values: rows,
            })
        };
        let cube_values = values.split_off(size1d.unwrap_or(0));
        let shaper = size1d.map(|n| table(n, range1d, values)).transpose()?;
        let cube = size3d.map(|n| table(n, range3d, cube_values)).transpose()?;
        Ok(Self {
            title,
            shaper,
            cube,
        })
    }
    pub fn from_3d(
        n: usize,
        min: [f32; 3],
        max: [f32; 3],
        values: Vec<[f32; 3]>,
        title: String,
    ) -> Result<Self> {
        size(n, true)?;
        domain(min, max)?;
        ensure!(
            values.len() == n.pow(3) && values.iter().flatten().all(|v| v.is_finite()),
            "invalid 3D LUT rows"
        );
        ensure!(
            !title.contains(['"', '\n', '\r', '\0']),
            "LUT title must not contain quotes or control separators"
        );
        Ok(Self {
            title: Some(title),
            shaper: None,
            cube: Some(Table {
                size: n,
                min,
                max,
                values,
            }),
        })
    }
    pub fn info(&self) -> CubeInfo {
        CubeInfo {
            title: self.title.clone(),
            size_1d: self.shaper.as_ref().map(|t| t.size),
            size_3d: self.cube.as_ref().map(|t| t.size),
            domain_1d: self.shaper.as_ref().map(|t| [t.min, t.max]),
            domain_3d: self.cube.as_ref().map(|t| [t.min, t.max]),
            combined: self.shaper.is_some() && self.cube.is_some(),
            row_count: self.shaper.as_ref().map(|t| t.values.len()).unwrap_or(0)
                + self.cube.as_ref().map(|t| t.values.len()).unwrap_or(0),
        }
    }
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.sample_with(rgb, LutInterpolation::Trilinear)
    }
    pub fn sample_with(&self, mut rgb: [f32; 3], interpolation: LutInterpolation) -> [f32; 3] {
        if let Some(table) = &self.shaper {
            rgb = table.sample_1d(rgb);
        }
        if let Some(table) = &self.cube {
            rgb = table.sample_3d(rgb, interpolation);
        }
        rgb
    }
    /// Iridas 3D output with unclamped values and round-trip f32 precision.
    pub fn write_3d(&self) -> Result<String> {
        ensure!(
            self.shaper.is_none(),
            "3D writer does not flatten a shaper implicitly"
        );
        let t = self
            .cube
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("not a 3D LUT"))?;
        let mut text = String::new();
        if let Some(title) = &self.title {
            writeln!(text, "TITLE \"{title}\"")?;
        }
        writeln!(text, "LUT_3D_SIZE {}", t.size)?;
        writeln!(
            text,
            "DOMAIN_MIN {:.9e} {:.9e} {:.9e}",
            t.min[0], t.min[1], t.min[2]
        )?;
        writeln!(
            text,
            "DOMAIN_MAX {:.9e} {:.9e} {:.9e}",
            t.max[0], t.max[1], t.max[2]
        )?;
        for rgb in &t.values {
            writeln!(text, "{:.9e} {:.9e} {:.9e}", rgb[0], rgb[1], rgb[2])?;
        }
        Ok(text)
    }
}
impl Table {
    fn coordinates(&self, rgb: [f32; 3]) -> ([usize; 3], [usize; 3], [f32; 3]) {
        let pos: [f32; 3] = std::array::from_fn(|c| {
            (((rgb[c] as f64 - self.min[c] as f64) / (self.max[c] as f64 - self.min[c] as f64))
                .clamp(0.0, 1.0)
                * (self.size - 1) as f64) as f32
        });
        let lo = pos.map(|v| v.floor() as usize);
        let hi = lo.map(|v| (v + 1).min(self.size - 1));
        (lo, hi, std::array::from_fn(|c| pos[c] - lo[c] as f32))
    }
    fn sample_1d(&self, rgb: [f32; 3]) -> [f32; 3] {
        let (lo, hi, t) = self.coordinates(rgb);
        std::array::from_fn(|c| self.values[lo[c]][c] * (1.0 - t[c]) + self.values[hi[c]][c] * t[c])
    }
    fn sample_3d(&self, rgb: [f32; 3], interpolation: LutInterpolation) -> [f32; 3] {
        let (lo, hi, t) = self.coordinates(rgb);
        let value = |idx: [usize; 3]| {
            self.values[idx[0] + idx[1] * self.size + idx[2] * self.size * self.size]
        };
        let mut out = [0.0; 3];
        if interpolation == LutInterpolation::Tetrahedral {
            let axes = if t[0] >= t[1] {
                if t[1] >= t[2] {
                    [0, 1, 2]
                } else if t[0] >= t[2] {
                    [0, 2, 1]
                } else {
                    [2, 0, 1]
                }
            } else if t[0] >= t[2] {
                [1, 0, 2]
            } else if t[1] >= t[2] {
                [1, 2, 0]
            } else {
                [2, 1, 0]
            };
            let mut v1 = lo;
            v1[axes[0]] = hi[axes[0]];
            let mut v2 = v1;
            v2[axes[1]] = hi[axes[1]];
            let weights = [
                1.0 - t[axes[0]],
                t[axes[0]] - t[axes[1]],
                t[axes[1]] - t[axes[2]],
                t[axes[2]],
            ];
            for (idx, w) in [lo, v1, v2, hi].into_iter().zip(weights) {
                let v = value(idx);
                for c in 0..3 {
                    out[c] += w * v[c];
                }
            }
        } else {
            for z in 0..2 {
                for y in 0..2 {
                    for x in 0..2 {
                        let idx = [
                            if x == 0 { lo[0] } else { hi[0] },
                            if y == 0 { lo[1] } else { hi[1] },
                            if z == 0 { lo[2] } else { hi[2] },
                        ];
                        let w = (if x == 0 { 1.0 - t[0] } else { t[0] })
                            * (if y == 0 { 1.0 - t[1] } else { t[1] })
                            * (if z == 0 { 1.0 - t[2] } else { t[2] });
                        let v = value(idx);
                        for c in 0..3 {
                            out[c] += w * v[c];
                        }
                    }
                }
            }
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_order_interpolation_and_domain() {
        let mut text = String::from("LUT_3D_SIZE 2\nDOMAIN_MIN -1 -1 -1\nDOMAIN_MAX 1 1 1\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    writeln!(text, "{r} {g} {b}").unwrap();
                }
            }
        }
        let lut = CubeLut::parse(&text).unwrap();
        for interpolation in [LutInterpolation::Trilinear, LutInterpolation::Tetrahedral] {
            assert_eq!(
                lut.sample_with([-0.5, 0.0, 0.5], interpolation),
                [0.25, 0.5, 0.75]
            );
        }
        assert!(CubeLut::parse("LUT_3D_SIZE 2\n0 0 0").is_err());
    }
}
