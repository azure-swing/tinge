//! Deterministic engineering fixture; no external photographs or image generator required.
use anyhow::Result;
use vibecolor_color::{from_display, hsv_to_rgb};
use vibecolor_core::Frame;
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "artifacts/test-chart.png".into());
    let (w, h) = (960u32, 640u32);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let rgb = if y < 160 {
                [fx; 3]
            } else if y < 320 {
                hsv_to_rgb([fx, 0.8, 0.82])
            } else if y < 480 {
                let row = (x / 160) as usize;
                [
                    [0.78, 0.56, 0.44],
                    [0.48, 0.28, 0.2],
                    [0.24, 0.5, 0.72],
                    [0.25, 0.5, 0.3],
                    [0.85, 0.72, 0.3],
                    [0.6, 0.28, 0.58],
                ][row]
            } else {
                let base = 0.18 + 0.35 * fx;
                let detail = if (x / 8 + y / 8) % 2 == 0 {
                    0.08
                } else {
                    -0.08
                };
                [base + detail, base + detail, base + detail]
            };
            let rgb = from_display(rgb);
            let rgb = if (fx - 0.5).powi(2) + (fy - 0.63).powi(2) < 0.014 {
                from_display([0.8, 0.6, 0.45])
            } else {
                rgb
            };
            pixels.push([rgb[0], rgb[1], rgb[2], 1.0]);
        }
    }
    let frame = Frame::new(w, h, pixels)?;
    let opt = vibecolor_io::ExportOptions {
        overwrite: true,
        ..Default::default()
    };
    let report = vibecolor_io::export(&frame, std::path::Path::new(&path), opt)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
