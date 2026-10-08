use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use vibecolor_core::{Frame, Recipe, lut::CubeLut, ops};
use vibecolor_project::{Project, base, resolve};
pub mod bake;

pub struct Engine {
    cache: BTreeMap<String, Arc<Frame>>,
    pub cache_budget_bytes: usize,
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug, Serialize)]
pub struct Progress {
    pub node: String,
    pub completed: usize,
    pub total: usize,
    pub cached: bool,
    pub elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocio_transform: Option<vibecolor_ocio::TransformReport>,
}
pub struct RenderContext<'a> {
    pub asset_base: &'a Path,
    pub color_pipeline: Option<&'a vibecolor_ocio::Pipeline>,
}
impl Engine {
    pub fn new() -> Self {
        Self {
            cache: BTreeMap::new(),
            cache_budget_bytes: 512 * 1024 * 1024,
        }
    }
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
    pub fn cache_usage_bytes(&self) -> usize {
        self.cache
            .values()
            .map(|frame| frame.pixels.len() * 16)
            .sum()
    }
    fn cache_put(&mut self, key: String, frame: Arc<Frame>) {
        let bytes = frame.pixels.len() * 16;
        if bytes > self.cache_budget_bytes {
            return;
        }
        // A large node must not evict the decoded RAW on every step. Keep decoded
        // inputs when intermediates cannot coexist within the existing budget.
        let decoded_bytes: usize = self
            .cache
            .iter()
            .filter(|(k, _)| k.starts_with("decode:"))
            .map(|(_, f)| f.pixels.len() * 16)
            .sum();
        if !key.starts_with("decode:") && decoded_bytes + bytes > self.cache_budget_bytes {
            return;
        }
        if self
            .cache
            .values()
            .map(|f| f.pixels.len() * 16)
            .sum::<usize>()
            + bytes
            > self.cache_budget_bytes
        {
            self.cache.retain(|k, _| k.starts_with("decode:"));
            if self
                .cache
                .values()
                .map(|f| f.pixels.len() * 16)
                .sum::<usize>()
                + bytes
                > self.cache_budget_bytes
            {
                self.clear_cache();
            }
        }
        self.cache.insert(key, frame);
    }
    pub fn render(
        &mut self,
        source: Arc<Frame>,
        recipe: &Recipe,
        dir: &Path,
        cancel: &AtomicBool,
        progress: &mut impl FnMut(Progress),
    ) -> Result<Arc<Frame>> {
        self.render_with_context(
            source,
            recipe,
            RenderContext {
                asset_base: dir,
                color_pipeline: None,
            },
            cancel,
            progress,
        )
    }
    pub fn render_with_context(
        &mut self,
        source: Arc<Frame>,
        recipe: &Recipe,
        context: RenderContext<'_>,
        cancel: &AtomicBool,
        progress: &mut impl FnMut(Progress),
    ) -> Result<Arc<Frame>> {
        let order = recipe.validate()?;
        let processors = vibecolor_project::compile_color_nodes(recipe, context.color_pipeline)?;
        let dir = context.asset_base;
        let start = Instant::now();
        let source_hash = hash_frame(&source, cancel)?;
        let mut frames = BTreeMap::from([("source".to_string(), source)]);
        let mut keys = BTreeMap::from([("source".to_string(), source_hash)]);
        // Execute only ancestors of output; validate still checks the entire graph.
        let mut needed = std::collections::BTreeSet::new();
        let mut pending = vec![recipe.output.clone()];
        while let Some(id) = pending.pop() {
            if id == "source" || !needed.insert(id.clone()) {
                continue;
            }
            let n = recipe
                .nodes
                .iter()
                .find(|n| n.id == id)
                .context("missing node")?;
            pending.extend(n.inputs.clone());
        }
        let order: Vec<_> = order.into_iter().filter(|id| needed.contains(id)).collect();
        // Reference counts permit freeing intermediate frames while retaining cache-budget limits.
        let mut uses = BTreeMap::<String, usize>::new();
        for n in recipe.nodes.iter().filter(|n| needed.contains(&n.id)) {
            for i in &n.inputs {
                *uses.entry(i.clone()).or_default() += 1;
            }
        }
        for (j, id) in order.iter().enumerate() {
            ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
            let node = recipe
                .nodes
                .iter()
                .find(|n| &n.id == id)
                .context("missing node")?;
            let inputs: Vec<_> = node
                .inputs
                .iter()
                .map(|i| frames.get(i).context("input not rendered"))
                .collect::<Result<_>>()?;
            let refs: Vec<_> = inputs.iter().map(|f| f.as_ref()).collect();
            let mut hasher = blake3::Hasher::new();
            hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
            hasher.update(&serde_json::to_vec(node)?);
            if let Some(processor) = processors.get(id) {
                let pipeline = context.color_pipeline.context("missing OCIO pipeline")?;
                hasher.update(&serde_json::to_vec(&(
                    pipeline.working_encoding,
                    processor.identity(),
                ))?);
            }
            for i in &node.inputs {
                hasher.update(keys[i].as_bytes());
            }
            // Hash external LUT/matte content into cache identity on every request.
            let mut assets = Vec::new();
            if let vibecolor_core::Operation::Lut { path, .. } = &node.op {
                assets.push(path.clone());
            }
            if let vibecolor_core::Operation::Cutout { options } = &node.op {
                assets.extend(options.assets());
            }
            let mask = node.mask.as_ref().and_then(|m| recipe.masks.get(m));
            if let Some(mask) = mask {
                hasher.update(&serde_json::to_vec(mask)?);
                mask_assets(mask, &mut assets);
            }
            for path in assets {
                hasher.update(vibecolor_io::hash_file(&resolve(dir, &path))?.as_bytes());
            }
            let key = hasher.finalize().to_hex().to_string();
            let cached = self.cache.contains_key(&key);
            let out = if let Some(frame) = self.cache.get(&key) {
                frame.clone()
            } else {
                let mut out = if !node.enabled {
                    (*refs[0]).clone()
                } else if let vibecolor_core::Operation::Cutout { options } = &node.op {
                    vibecolor_core::cutout::run(
                        refs[0],
                        options,
                        &|path, w, h| vibecolor_io::load_matte(&resolve(dir, path), w, h),
                        cancel,
                    )?
                    .0
                } else if let Some(processor) = processors.get(id) {
                    let mut frame = (*refs[0]).clone();
                    context
                        .color_pipeline
                        .context("missing OCIO pipeline")?
                        .apply_grade(processor, &mut frame.pixels)?;
                    frame
                } else {
                    ops::apply_cancellable(
                        &node.op,
                        &refs,
                        &|path| CubeLut::parse(&std::fs::read_to_string(resolve(dir, path))?),
                        cancel,
                    )?
                };
                if node.enabled
                    && !node.op.changes_dimensions()
                    && (node.mix < 1.0 || mask.is_some())
                {
                    let weights = mask
                        .map(|m| {
                            m.rasterize_cancellable(
                                refs[0],
                                &|path, w, h| vibecolor_io::load_matte(&resolve(dir, path), w, h),
                                cancel,
                            )
                        })
                        .transpose()?;
                    ensure!(
                        out.width == refs[0].width && out.height == refs[0].height,
                        "masked output dimensions mismatch"
                    );
                    out.pixels
                        .par_iter_mut()
                        .enumerate()
                        .try_for_each(|(i, p)| -> Result<()> {
                            if i % 1024 == 0 {
                                ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
                            }
                            let w = node.mix * weights.as_ref().map(|m| m[i]).unwrap_or(1.0);
                            let a = refs[0].pixels[i];
                            if p[3] == a[3] {
                                for c in 0..3 {
                                    p[c] = a[c] + (p[c] - a[c]) * w;
                                }
                            } else {
                                // Alpha-changing nodes are mixed in premultiplied space.
                                let alpha = a[3] * (1.0 - w) + p[3] * w;
                                for c in 0..3 {
                                    p[c] = if alpha > 1e-8 {
                                        (a[c] * a[3] * (1.0 - w) + p[c] * p[3] * w) / alpha
                                    } else {
                                        0.0
                                    };
                                }
                                p[3] = alpha;
                            }
                            Ok(())
                        })?;
                }
                let frame = Arc::new(out);
                self.cache_put(key.clone(), frame.clone());
                frame
            };
            let ocio_transform = processors
                .get(id)
                .map(|p| {
                    let mut report = p.identity().clone();
                    report.pixel_count = out.pixels.len();
                    report
                })
                .filter(|_| node.enabled);
            frames.insert(id.clone(), out);
            keys.insert(id.clone(), key);
            for input in &node.inputs {
                let count = uses
                    .get_mut(input)
                    .context("missing input reference count")?;
                *count -= 1;
                if *count == 0 && input != &recipe.output {
                    frames.remove(input);
                }
            }
            progress(Progress {
                node: id.clone(),
                completed: j + 1,
                total: order.len(),
                cached,
                elapsed_ms: start.elapsed().as_millis(),
                ocio_transform,
            });
        }
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        frames
            .remove(&recipe.output)
            .context("missing output frame")
    }
    pub fn project_render(
        &mut self,
        path: &Path,
        p: &Project,
        revision: Option<u64>,
        cancel: &AtomicBool,
        progress: &mut impl FnMut(Progress),
    ) -> Result<Arc<Frame>> {
        let frame = self.project_source(path, p, revision, cancel)?;
        let rev = p.get_revision(revision.unwrap_or(p.revision))?;
        let pipeline = rev
            .color_pipeline
            .as_ref()
            .map(|p| p.resolved_at(base(path)));
        verify_recipe_assets(&rev.recipe, base(path))?;
        self.render_with_context(
            frame,
            &rev.recipe,
            RenderContext {
                asset_base: base(path),
                color_pipeline: pipeline.as_ref(),
            },
            cancel,
            progress,
        )
    }
    /// Decode the managed source once, also used by the viewer's original image.
    pub fn project_source(
        &mut self,
        path: &Path,
        p: &Project,
        revision: Option<u64>,
        cancel: &AtomicBool,
    ) -> Result<Arc<Frame>> {
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        let rev = p.get_revision(revision.unwrap_or(p.revision))?;
        let pipeline = rev
            .color_pipeline
            .as_ref()
            .map(|p| p.resolved_at(base(path)));
        if let Some(pipeline) = &pipeline {
            pipeline.validate()?;
        }
        let input = resolve(base(path), &p.source.path);
        ensure!(
            vibecolor_io::hash_file(&input)? == p.source.hash,
            "source image hash mismatch; source has changed"
        );
        let decode_key = format!(
            "decode:{}:{}:{}:{}",
            p.source.hash,
            serde_json::to_string(&p.source.color_space)?,
            serde_json::to_string(&rev.color_pipeline)?,
            serde_json::to_string(&rev.raw_develop)?
        );
        let frame = if let Some(f) = self.cache.get(&decode_key) {
            f.clone()
        } else {
            let f = Arc::new(load_source_with_options(
                &input,
                p.source.color_space,
                pipeline.as_ref(),
                rev.raw_develop.as_ref(),
            )?);
            self.cache_put(decode_key, f.clone());
            f
        };
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        Ok(frame)
    }
}
pub fn verify_recipe_assets(recipe: &Recipe, dir: &Path) -> Result<()> {
    let mut assets = Vec::new();
    for n in &recipe.nodes {
        if let vibecolor_core::Operation::Lut { path, .. } = &n.op {
            assets.push(path.clone());
        }
        if let vibecolor_core::Operation::Cutout { options } = &n.op {
            assets.extend(options.assets());
        }
    }
    for m in recipe.masks.values() {
        mask_assets(m, &mut assets);
    }
    for asset in assets {
        let file = resolve(dir, &asset);
        let expected = file
            .file_stem()
            .and_then(|p| p.to_str())
            .context("invalid asset name")?;
        ensure!(
            vibecolor_io::hash_file(&file)? == expected,
            "immutable project asset was modified: {asset}"
        );
    }
    Ok(())
}
/// Decode explicitly typed source values into the canonical scene-linear Frame.
pub fn load_source(
    path: &Path,
    space: Option<vibecolor_color::ColorSpace>,
    pipeline: Option<&vibecolor_ocio::Pipeline>,
) -> Result<Frame> {
    load_source_with_options(path, space, pipeline, None)
}
pub fn load_source_with_options(
    path: &Path,
    space: Option<vibecolor_color::ColorSpace>,
    pipeline: Option<&vibecolor_ocio::Pipeline>,
    raw: Option<&vibecolor_io::RawDevelopOptions>,
) -> Result<Frame> {
    vibecolor_project::validate_source_options(path, space, pipeline, raw)?;
    if let Some(pipeline) = pipeline {
        pipeline.validate()?;
        if matches!(
            pipeline.input,
            vibecolor_ocio::InputEncoding::Encoded { .. }
        ) {
            ensure!(
                space.is_none(),
                "input_space and OCIO encoded input are mutually exclusive"
            );
            let mut image = vibecolor_io::load_signal(path)?;
            pipeline.to_working(&mut image.pixels)?;
            return Frame::new(image.width, image.height, image.pixels);
        }
    }
    if let Some(raw) = raw {
        vibecolor_io::raw::load_with_options(path, raw)
    } else {
        vibecolor_io::load(path, space)
    }
}

/// Float exports preserve working scene values; integer outputs use the view.
pub fn export_frame(
    frame: &Frame,
    path: &Path,
    options: vibecolor_io::ExportOptions,
    pipeline: Option<&vibecolor_ocio::Pipeline>,
) -> Result<(
    vibecolor_io::ExportReport,
    Option<vibecolor_ocio::TransformReport>,
)> {
    if let Some(pipeline) = pipeline {
        pipeline.validate()?;
        if options.bit_depth != 32 {
            ensure!(
                options.space == pipeline.display.color_space(),
                "output_space must match the OCIO display encoding; select the display in color_pipeline"
            );
            let mut signal = vibecolor_io::SignalImage {
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels.clone(),
            };
            let transform = pipeline.to_display(&mut signal.pixels)?;
            return Ok((
                vibecolor_io::export_signal(&signal, path, options)?,
                Some(transform),
            ));
        }
    }
    Ok((vibecolor_io::export(frame, path, options)?, None))
}
pub fn preview_png_bytes(
    frame: &Frame,
    pipeline: Option<&vibecolor_ocio::Pipeline>,
) -> Result<(
    Vec<u8>,
    vibecolor_io::ExportReport,
    Option<vibecolor_ocio::TransformReport>,
)> {
    if let Some(pipeline) = pipeline {
        pipeline.validate()?;
        ensure!(
            pipeline.display.color_space() == vibecolor_color::ColorSpace::default(),
            "memory preview requires sRGB SDR"
        );
        let mut pixels = frame.pixels.clone();
        let transform = pipeline.to_display(&mut pixels)?;
        let (bytes, report) =
            vibecolor_io::preview_png_bytes(frame.width, frame.height, &pixels, false)?;
        return Ok((bytes, report, Some(transform)));
    }
    let (bytes, report) =
        vibecolor_io::preview_png_bytes(frame.width, frame.height, &frame.pixels, true)?;
    Ok((bytes, report, None))
}
fn hash_frame(f: &Frame, cancel: &AtomicBool) -> Result<String> {
    let mut h = blake3::Hasher::new();
    h.update(&f.width.to_le_bytes());
    h.update(&f.height.to_le_bytes());
    let mut bytes = [0u8; 65536];
    for pixels in f.pixels.chunks(4096) {
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        for (i, p) in pixels.iter().enumerate() {
            for (c, value) in p.iter().enumerate() {
                let begin = i * 16 + c * 4;
                bytes[begin..begin + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        h.update(&bytes[..pixels.len() * 16]);
    }
    Ok(h.finalize().to_hex().to_string())
}
fn mask_assets(mask: &vibecolor_core::Mask, paths: &mut Vec<String>) {
    use vibecolor_core::Mask;
    match mask {
        Mask::Bitmap { path } => paths.push(path.clone()),
        Mask::Combine { masks, .. } => {
            for m in masks {
                mask_assets(m, paths);
            }
        }
        Mask::Invert { mask } => mask_assets(mask, paths),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_source_survives_large_node_cache_pressure() {
        let frame = Arc::new(Frame::new(4, 4, vec![[0.1, 0.2, 0.3, 1.0]; 16]).unwrap());
        let mut engine = Engine::new();
        engine.cache_budget_bytes = 300;
        engine.cache_put("decode:raw".into(), frame.clone());
        for index in 0..12 {
            engine.cache_put(format!("node-{index}"), frame.clone());
        }
        assert!(engine.cache.contains_key("decode:raw"));
        assert!(
            engine
                .cache
                .values()
                .map(|f| f.pixels.len() * 16)
                .sum::<usize>()
                <= 300
        );
        engine.cache_put("decode:other-wb".into(), frame);
        assert!(engine.cache.contains_key("decode:other-wb"));
        assert!(!engine.cache.contains_key("decode:raw"));
    }
    #[test]
    fn block_hash_matches_previous_identity_and_can_cancel() {
        let frame = Frame::new(137, 241, vec![[-0.1, 0.234567, 2.5, 0.6]; 137 * 241]).unwrap();
        let mut reference = blake3::Hasher::new();
        reference.update(&frame.width.to_le_bytes());
        reference.update(&frame.height.to_le_bytes());
        for pixel in &frame.pixels {
            for value in pixel {
                reference.update(&value.to_le_bytes());
            }
        }
        assert_eq!(
            hash_frame(&frame, &AtomicBool::new(false)).unwrap(),
            reference.finalize().to_hex().to_string()
        );
        assert!(hash_frame(&frame, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn native_log_cdl_is_mixed_in_canonical_space_and_cached_with_processor_report() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
        ))
        .unwrap();
        let case = &fixture["cases"][0];
        let input: Vec<[f32; 4]> = serde_json::from_value(case["input"].clone()).unwrap();
        let expected: Vec<[f32; 4]> = serde_json::from_value(case["expected"].clone()).unwrap();
        let recipe: Recipe = serde_json::from_value(serde_json::json!({
            "nodes":[{"id":"cdl","op":{"type":"ocio_grade","grade":case["transform"]["grade"]},"mask":"ramp","mix":0.4},
                     {"id":"linear_ev","inputs":["cdl"],"op":{"type":"exposure","stops":1}}],
            "output":"linear_ev","masks":{"ramp":{"type":"linear_gradient","start":[0,0.5],"end":[1,0.5]}}
        })).unwrap();
        let pipeline: vibecolor_ocio::Pipeline =
            serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
        let source = Arc::new(Frame::new(input.len() as u32, 1, input.clone()).unwrap());
        let mut engine = Engine::new();
        let cancel = AtomicBool::new(false);
        let mut progress = Vec::new();
        let output = engine
            .render_with_context(
                source.clone(),
                &recipe,
                RenderContext {
                    asset_base: Path::new("."),
                    color_pipeline: Some(&pipeline),
                },
                &cancel,
                &mut |p| progress.push(p),
            )
            .unwrap();
        for (i, p) in output.pixels.iter().enumerate() {
            let weight = 0.4 * (i as f32 + 0.5) / input.len() as f32;
            for c in 0..3 {
                let expected = 2.0 * (input[i][c] + weight * (expected[i][c] - input[i][c]));
                assert!((p[c] - expected).abs() < 6e-5 * expected.abs().max(1.0));
            }
            assert_eq!(p[3], input[i][3]);
        }
        assert!(
            progress[0]
                .ocio_transform
                .as_ref()
                .unwrap()
                .processor_cache_id
                .is_some()
        );
        assert!(progress[1].ocio_transform.is_none());
        progress.clear();
        let again = engine
            .render_with_context(
                source.clone(),
                &recipe,
                RenderContext {
                    asset_base: Path::new("."),
                    color_pipeline: Some(&pipeline),
                },
                &cancel,
                &mut |p| progress.push(p),
            )
            .unwrap();
        assert_eq!(again.pixels, output.pixels);
        assert!(progress.iter().all(|p| p.cached));
        assert_eq!(
            progress[0].ocio_transform.as_ref().unwrap().pixel_count,
            input.len()
        );
        // An AP1/D60 anchor must return the same canonical scene result.
        let mut ap1 = pipeline.clone();
        ap1.working_space = "ACEScg".into();
        ap1.working_encoding.primaries = vibecolor_color::Primaries::AcesCg;
        let alternate = engine
            .render_with_context(
                source.clone(),
                &recipe,
                RenderContext {
                    asset_base: Path::new("."),
                    color_pipeline: Some(&ap1),
                },
                &cancel,
                &mut |_| {},
            )
            .unwrap();
        for (a, b) in alternate.pixels.iter().zip(&output.pixels) {
            for c in 0..3 {
                assert!(
                    (a[c] - b[c]).abs() < 8e-5 * b[c].abs().max(1.0),
                    "{a:?} {b:?}"
                );
            }
            assert_eq!(a[3], b[3]);
        }
        assert!(
            engine
                .render(source, &recipe, Path::new("."), &cancel, &mut |_| {})
                .is_err()
        );
    }

    #[test]
    fn editable_look_lut_changes_invalidate_node_cache_and_missing_dependencies_fail() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.ocio");
        std::fs::write(
            &config,
            include_str!("../../../examples/node-ocio/config.ocio"),
        )
        .unwrap();
        let lut_dir = dir.path().join("luts/neutral");
        std::fs::create_dir_all(&lut_dir).unwrap();
        let lut = lut_dir.join("look.cube");
        std::fs::write(&lut, "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        let pipeline: vibecolor_ocio::Pipeline = serde_json::from_value(serde_json::json!({
            "config":{"type":"file","path":config},"working_space":"linear","display_name":"Photo sRGB","view":"Standard"
        })).unwrap();
        let recipe: Recipe = serde_json::from_value(serde_json::json!({"nodes":[{"id":"look","op":{"type":"ocio_grade","grade":{"type":"look","looks":"ContextGrade"}}}],"output":"look"})).unwrap();
        let mut engine = Engine::new();
        let frame = Arc::new(Frame::new(1, 1, vec![[0.6, 0.4, 0.2, 0.3]]).unwrap());
        let cancel = AtomicBool::new(false);
        let context = || RenderContext {
            asset_base: dir.path(),
            color_pipeline: Some(&pipeline),
        };
        let original = engine
            .render_with_context(frame.clone(), &recipe, context(), &cancel, &mut |_| {})
            .unwrap();
        let mut cached = false;
        engine
            .render_with_context(frame.clone(), &recipe, context(), &cancel, &mut |p| {
                cached = p.cached
            })
            .unwrap();
        assert!(cached);
        std::fs::write(&lut, "LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n").unwrap();
        let half = engine
            .render_with_context(frame.clone(), &recipe, context(), &cancel, &mut |p| {
                cached = p.cached
            })
            .unwrap();
        assert!(!cached);
        for c in 0..3 {
            assert!((half.pixels[0][c] - original.pixels[0][c] * 0.5).abs() < 1e-6);
        }
        std::fs::remove_file(lut).unwrap();
        assert!(
            engine
                .render_with_context(frame, &recipe, context(), &cancel, &mut |_| {})
                .is_err()
        );
    }
    #[test]
    fn local_node_cache_and_cancel() {
        let mut e = Engine::new();
        let f = Arc::new(Frame::new(4, 1, vec![[0.2, 0.2, 0.2, 1.0]; 4]).unwrap());
        let r:Recipe=serde_json::from_str(r#"{"nodes":[{"id":"e","op":{"type":"exposure","stops":1},"mask":"g"}],"output":"e","masks":{"g":{"type":"linear_gradient","start":[0,0.5],"end":[1,0.5]}}}"#).unwrap();
        let cancel = AtomicBool::new(false);
        let out = e
            .render(f.clone(), &r, Path::new("."), &cancel, &mut |_| {})
            .unwrap();
        assert!((out.pixels[0][0] - 0.225).abs() < 1e-6);
        assert!((out.pixels[3][0] - 0.375).abs() < 1e-6);
        let mut hit = false;
        e.render(f.clone(), &r, Path::new("."), &cancel, &mut |p| {
            hit = p.cached
        })
        .unwrap();
        assert!(hit);
        cancel.store(true, Ordering::Relaxed);
        assert!(
            e.render(f, &r, Path::new("."), &cancel, &mut |_| {})
                .is_err()
        );
    }
}
