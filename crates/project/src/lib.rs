use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tinge_color::ColorSpace;
use tinge_core::{Mask, Node, Operation, Recipe};
pub mod selections;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: String,
    pub hash: String,
    pub color_space: Option<ColorSpace>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub id: u64,
    pub parent: Option<u64>,
    pub timestamp: u64,
    pub label: String,
    pub recipe: Recipe,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_pipeline: Option<tinge_ocio::Pipeline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_develop: Option<tinge_io::RawDevelopOptions>,
    pub engine_version: String,
    pub recipe_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub schema_version: u32,
    pub source: Source,
    pub revision: u64,
    pub current_branch: String,
    pub branches: BTreeMap<String, u64>,
    pub tags: BTreeMap<String, u64>,
    pub history: Vec<Revision>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    UpsertNode {
        node: Node,
    },
    RemoveNode {
        id: String,
    },
    SetOutput {
        id: String,
    },
    SetMask {
        id: String,
        mask: Mask,
    },
    RemoveMask {
        id: String,
    },
    ReplaceRecipe {
        recipe: Recipe,
    },
    SetColorPipeline {
        pipeline: Option<tinge_ocio::Pipeline>,
    },
    SetRawDevelop {
        options: Option<tinge_io::RawDevelopOptions>,
    },
}
#[derive(Debug)]
pub struct RevisionConflict {
    pub expected: u64,
    pub actual: u64,
}
impl std::fmt::Display for RevisionConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "revision conflict: expected {}, actual {}",
            self.expected, self.actual
        )
    }
}
impl std::error::Error for RevisionConflict {}
fn check_revision(p: &Project, expected: u64) -> Result<()> {
    if p.revision != expected {
        return Err(RevisionConflict {
            expected,
            actual: p.revision,
        }
        .into());
    }
    Ok(())
}
struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
fn lock(path: &Path) -> Result<Lock> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(PathBuf::from(name))?;
    file.lock_exclusive()?;
    Ok(Lock(file))
}
pub fn base(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}
pub fn resolve(base: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.into()
    } else {
        base.join(p)
    }
}
fn asset_dir(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".assets");
    PathBuf::from(name)
}
fn freeze(path: &Path, source: &Path) -> Result<String> {
    let dir = asset_dir(path);
    fs::create_dir_all(&dir)?;
    let hash =
        tinge_io::hash_file(source).with_context(|| format!("asset {}", source.display()))?;
    let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("bin");
    let dest = dir.join(format!("{hash}.{ext}"));
    if dest.exists() {
        ensure!(
            tinge_io::hash_file(&dest)? == hash,
            "stored asset hash mismatch"
        );
    } else {
        let bytes = fs::read(source)?;
        ensure!(
            blake3::hash(&bytes).to_hex().as_str() == hash,
            "asset changed during import"
        );
        match tinge_io::atomic_bytes(&dest, &bytes, false) {
            Ok(()) => {}
            Err(e) => {
                if !dest.exists() || tinge_io::hash_file(&dest)? != hash {
                    return Err(e);
                }
            }
        }
    }
    // Store relative to project parent; moving the project and assets together works.
    let relative = dest.strip_prefix(base(path)).unwrap_or(&dest);
    Ok(relative.to_string_lossy().into_owned())
}
fn freeze_mask(m: &mut Mask, project: &Path, origin: &Path) -> Result<()> {
    match m {
        Mask::Bitmap { path } => *path = freeze(project, &resolve(origin, path))?,
        Mask::Combine { masks, .. } => {
            for m in masks {
                freeze_mask(m, project, origin)?;
            }
        }
        Mask::Invert { mask } => freeze_mask(mask, project, origin)?,
        _ => {}
    }
    Ok(())
}
fn freeze_recipe(recipe: &mut Recipe, project: &Path, origin: &Path) -> Result<()> {
    for n in &mut recipe.nodes {
        if let Operation::Lut { path, .. } = &mut n.op {
            *path = freeze(project, &resolve(origin, path))?;
        }
        if let Operation::Cutout { options } = &mut n.op {
            options.map_assets(|p| freeze(project, &resolve(origin, p)))?;
        }
    }
    for m in recipe.masks.values_mut() {
        freeze_mask(m, project, origin)?;
    }
    Ok(())
}
/// Read-only recipe export work freezes dependencies into an isolated asset set.
/// No project JSON is created; the caller owns the temporary directory lifetime.
pub fn snapshot_recipe(
    mut recipe: Recipe,
    pipeline: Option<tinge_ocio::Pipeline>,
    destination: &Path,
    origin: &Path,
) -> Result<(Recipe, Option<tinge_ocio::Pipeline>)> {
    recipe.validate()?;
    freeze_recipe(&mut recipe, destination, origin)?;
    let pipeline = pipeline
        .map(|p| freeze_pipeline(p.resolved_at(origin), destination, origin))
        .transpose()?;
    let resolved = pipeline.as_ref().map(|p| p.resolved_at(base(destination)));
    compile_color_nodes(&recipe, resolved.as_ref())?;
    Ok((recipe, pipeline))
}
fn freeze_pipeline(
    mut pipeline: tinge_ocio::Pipeline,
    project: &Path,
    origin: &Path,
) -> Result<tinge_ocio::Pipeline> {
    use tinge_ocio::{ConfigSource, PipelineConfig};
    if let PipelineConfig::Source(
        source @ (ConfigSource::File { .. } | ConfigSource::Frozen { .. }),
    ) = &pipeline.config
    {
        let bytes = match source {
            ConfigSource::File { path } => {
                let source = if path.is_absolute() {
                    path.clone()
                } else {
                    origin.join(path)
                };
                tinge_ocio::archive_file(&source)?
            }
            ConfigSource::Frozen { path, hash } => {
                let source = if path.is_absolute() {
                    path.clone()
                } else {
                    base(project).join(path)
                };
                ensure!(
                    tinge_io::hash_file(&source)? == *hash,
                    "incoming OCIO package hash mismatch"
                );
                let bytes = fs::read(source)?;
                ensure!(
                    blake3::hash(&bytes).to_hex().as_str() == hash,
                    "incoming OCIO package changed during import"
                );
                bytes
            }
            _ => unreachable!(),
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let dest = asset_dir(project).join(format!("{hash}.ocioz"));
        fs::create_dir_all(asset_dir(project))?;
        if dest.exists() {
            ensure!(
                tinge_io::hash_file(&dest)? == hash,
                "stored OCIO archive hash mismatch"
            );
        } else {
            tinge_io::atomic_bytes(&dest, &bytes, false)?;
        }
        let relative = dest.strip_prefix(base(project)).unwrap_or(&dest);
        pipeline.config = PipelineConfig::Source(ConfigSource::Frozen {
            path: relative.into(),
            hash,
        });
    }
    pipeline.resolved_at(base(project)).validate()?;
    Ok(pipeline)
}
fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn revision_hash(
    recipe: &Recipe,
    pipeline: Option<&tinge_ocio::Pipeline>,
    raw: Option<&tinge_io::RawDevelopOptions>,
) -> Result<String> {
    // Preserve existing schema-v1 hashes when no pipeline is selected.
    let bytes = if let Some(raw) = raw {
        serde_json::to_vec(&(recipe, pipeline, raw))?
    } else if let Some(pipeline) = pipeline {
        serde_json::to_vec(&(recipe, pipeline))?
    } else {
        serde_json::to_vec(recipe)?
    };
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
/// Compile all declared color nodes before committing or looking up graph caches.
/// Even disabled/unreachable nodes must reference a valid, reproducible processor.
pub fn compile_color_nodes(
    recipe: &Recipe,
    pipeline: Option<&tinge_ocio::Pipeline>,
) -> Result<BTreeMap<String, tinge_ocio::CompiledTransform>> {
    let mut processors = BTreeMap::new();
    if recipe
        .nodes
        .iter()
        .any(|n| matches!(n.op, Operation::OcioGrade { .. }))
    {
        let pipeline = pipeline.context("ocio_grade nodes require color_pipeline")?;
        pipeline.validate()?;
        for node in &recipe.nodes {
            if let Operation::OcioGrade { grade } = &node.op {
                processors.insert(
                    node.id.clone(),
                    pipeline
                        .compile_grade(grade)
                        .with_context(|| format!("OCIO grade node {}", node.id))?,
                );
            }
        }
    }
    Ok(processors)
}
fn revision(
    id: u64,
    parent: Option<u64>,
    label: String,
    recipe: Recipe,
    color_pipeline: Option<tinge_ocio::Pipeline>,
    raw_develop: Option<tinge_io::RawDevelopOptions>,
) -> Result<Revision> {
    let recipe_hash = revision_hash(&recipe, color_pipeline.as_ref(), raw_develop.as_ref())?;
    Ok(Revision {
        id,
        parent,
        timestamp: stamp(),
        label,
        recipe,
        color_pipeline,
        raw_develop,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        recipe_hash,
    })
}
impl Project {
    pub fn head(&self) -> Result<&Revision> {
        self.history
            .iter()
            .find(|h| h.id == self.revision)
            .context("project head is missing")
    }
    pub fn get_revision(&self, id: u64) -> Result<&Revision> {
        self.history
            .iter()
            .find(|h| h.id == id)
            .with_context(|| format!("revision {id} does not exist"))
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported project version");
        self.head()?;
        ensure!(
            !self.source.path.is_empty() && self.source.hash.len() == 64,
            "invalid source reference"
        );
        ensure!(
            self.branches.get(&self.current_branch) == Some(&self.revision),
            "current branch/head mismatch"
        );
        let mut ids = std::collections::BTreeSet::new();
        for h in &self.history {
            ensure!(ids.insert(h.id), "duplicate revision");
            if let Some(p) = h.parent {
                ensure!(p < h.id && ids.contains(&p), "invalid parent revision");
            }
            h.recipe.validate()?;
            ensure!(
                h.color_pipeline.is_some()
                    || !h
                        .recipe
                        .nodes
                        .iter()
                        .any(|n| matches!(n.op, Operation::OcioGrade { .. })),
                "ocio_grade nodes require color_pipeline at revision {}",
                h.id
            );
            validate_source_options(
                Path::new(&self.source.path),
                self.source.color_space,
                h.color_pipeline.as_ref(),
                h.raw_develop.as_ref(),
            )?;
            let hash = revision_hash(&h.recipe, h.color_pipeline.as_ref(), h.raw_develop.as_ref())?;
            ensure!(
                h.recipe_hash == hash,
                "recipe hash mismatch at revision {}",
                h.id
            );
        }
        for id in self.branches.values().chain(self.tags.values()) {
            self.get_revision(*id)?;
        }
        Ok(())
    }
    fn commit(
        &mut self,
        label: String,
        recipe: Recipe,
        pipeline: Option<tinge_ocio::Pipeline>,
        raw: Option<tinge_io::RawDevelopOptions>,
    ) -> Result<u64> {
        recipe.validate()?;
        let id = self.history.last().context("empty history")?.id + 1;
        self.history.push(revision(
            id,
            Some(self.revision),
            label,
            recipe,
            pipeline,
            raw,
        )?);
        self.revision = id;
        self.branches.insert(self.current_branch.clone(), id);
        Ok(id)
    }
}
pub fn load(path: &Path) -> Result<Project> {
    let _lock = lock(path)?;
    read(path)
}
/// Run project-side metadata work while excluding concurrent project commits.
pub fn with_locked_project<T>(
    path: &Path,
    action: impl FnOnce(&Project) -> Result<T>,
) -> Result<T> {
    let _lock = lock(path)?;
    action(&read(path)?)
}
fn read(path: &Path) -> Result<Project> {
    let p: Project = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read project {}", path.display()))?,
    )?;
    p.validate()?;
    Ok(p)
}
fn save(path: &Path, p: &Project, overwrite: bool) -> Result<()> {
    p.validate()?;
    tinge_io::atomic_bytes(path, &serde_json::to_vec(p)?, overwrite)
}
pub fn init(path: &Path, input: &Path, space: Option<ColorSpace>) -> Result<Project> {
    init_with_pipeline(path, input, space, None)
}
/// Check source interpretation independently from decoding, including old revision reads.
pub fn validate_source_options(
    input: &Path,
    space: Option<ColorSpace>,
    pipeline: Option<&tinge_ocio::Pipeline>,
    raw: Option<&tinge_io::RawDevelopOptions>,
) -> Result<()> {
    if let Some(raw) = raw {
        raw.validate()?;
        ensure!(
            tinge_io::raw::is_raw(input),
            "RAW controls require a RAW source"
        );
        ensure!(space.is_none(), "RAW controls conflict with input_space");
        ensure!(
            !pipeline.is_some_and(|p| matches!(p.input, tinge_ocio::InputEncoding::Encoded { .. })),
            "RAW controls conflict with OCIO encoded input"
        );
    }
    Ok(())
}
fn load_managed_source(
    input: &Path,
    space: Option<ColorSpace>,
    raw: Option<&tinge_io::RawDevelopOptions>,
) -> Result<tinge_core::Frame> {
    if let Some(raw) = raw {
        tinge_io::raw::load_with_options(input, raw)
    } else {
        tinge_io::load(input, space)
    }
}
pub fn init_with_pipeline(
    path: &Path,
    input: &Path,
    space: Option<ColorSpace>,
    pipeline: Option<tinge_ocio::Pipeline>,
) -> Result<Project> {
    init_with_source_options(path, input, space, pipeline, None)
}
pub fn init_with_source_options(
    path: &Path,
    input: &Path,
    space: Option<ColorSpace>,
    pipeline: Option<tinge_ocio::Pipeline>,
    raw_develop: Option<tinge_io::RawDevelopOptions>,
) -> Result<Project> {
    let _lock = lock(path)?;
    ensure!(!path.exists(), "project already exists");
    let pipeline = pipeline
        .map(|p| freeze_pipeline(p, path, Path::new(".")))
        .transpose()?;
    validate_source_options(input, space, pipeline.as_ref(), raw_develop.as_ref())?;
    let frozen = freeze(path, input)?;
    let source = Source {
        hash: tinge_io::hash_file(&resolve(base(path), &frozen))?,
        path: frozen,
        color_space: space,
    };
    let frozen_input = resolve(base(path), &source.path);
    let input = frozen_input.as_path();
    if let Some(pipeline) = &pipeline {
        let pipeline = pipeline.resolved_at(base(path));
        pipeline.validate()?;
        if matches!(pipeline.input, tinge_ocio::InputEncoding::Encoded { .. }) {
            ensure!(
                space.is_none(),
                "input_space and OCIO encoded input are mutually exclusive"
            );
            let mut image = tinge_io::load_signal(input)?;
            pipeline.to_working(&mut image.pixels)?;
        } else {
            load_managed_source(input, space, raw_develop.as_ref())?;
        }
    } else {
        load_managed_source(input, space, raw_develop.as_ref())?;
    }
    let p = Project {
        schema_version: 1,
        source,
        revision: 0,
        current_branch: "main".into(),
        branches: BTreeMap::from([("main".into(), 0)]),
        tags: BTreeMap::new(),
        history: vec![revision(
            0,
            None,
            "import".into(),
            Recipe::default(),
            pipeline,
            raw_develop,
        )?],
    };
    save(path, &p, false)?;
    Ok(p)
}
pub fn transaction(
    path: &Path,
    expected: u64,
    edits: Vec<Edit>,
    label: String,
    origin: &Path,
) -> Result<Project> {
    let _lock = lock(path)?;
    let mut p = read(path)?;
    check_revision(&p, expected)?;
    ensure!(!edits.is_empty(), "transaction requires at least one edit");
    let mut recipe = p.head()?.recipe.clone();
    let mut pipeline = p.head()?.color_pipeline.clone();
    let mut raw_develop = p.head()?.raw_develop.clone();
    let mut raw_changed = false;
    for edit in edits {
        match edit {
            Edit::SetRawDevelop { options } => {
                raw_develop = options;
                raw_changed = true;
            }
            Edit::SetColorPipeline { pipeline: next } => {
                let next = next.map(|p| freeze_pipeline(p, path, origin)).transpose()?;
                if let Some(next) = &next {
                    let next = next.resolved_at(base(path));
                    next.validate()?;
                    if matches!(next.input, tinge_ocio::InputEncoding::Encoded { .. }) {
                        ensure!(
                            p.source.color_space.is_none(),
                            "explicit input_space conflicts with OCIO encoded input"
                        );
                        let mut samples =
                            tinge_io::load_signal(&resolve(base(path), &p.source.path))?;
                        next.to_working(&mut samples.pixels)?;
                    }
                }
                pipeline = next;
            }
            Edit::UpsertNode { mut node } => {
                if let Operation::Lut { path: asset, .. } = &mut node.op {
                    *asset = freeze(path, &resolve(origin, asset))?;
                }
                if let Operation::Cutout { options } = &mut node.op {
                    options.map_assets(|asset| freeze(path, &resolve(origin, asset)))?;
                }
                if let Some(old) = recipe.nodes.iter_mut().find(|n| n.id == node.id) {
                    *old = node;
                } else {
                    recipe.nodes.push(node);
                }
            }
            Edit::RemoveNode { id } => {
                let n = recipe.nodes.len();
                recipe.nodes.retain(|v| v.id != id);
                ensure!(n != recipe.nodes.len(), "node {id} not found");
            }
            Edit::SetOutput { id } => recipe.output = id,
            Edit::SetMask { id, mut mask } => {
                ensure!(!id.is_empty(), "mask ID cannot be empty");
                freeze_mask(&mut mask, path, origin)?;
                recipe.masks.insert(id, mask);
            }
            Edit::RemoveMask { id } => {
                ensure!(recipe.masks.remove(&id).is_some(), "mask {id} not found");
            }
            Edit::ReplaceRecipe { recipe: mut next } => {
                next.validate()?;
                freeze_recipe(&mut next, path, origin)?;
                recipe = next;
            }
        }
    }
    recipe.validate()?;
    let resolved_pipeline = pipeline.as_ref().map(|p| p.resolved_at(base(path)));
    compile_color_nodes(&recipe, resolved_pipeline.as_ref())?;
    let source = resolve(base(path), &p.source.path);
    validate_source_options(
        &source,
        p.source.color_space,
        resolved_pipeline.as_ref(),
        raw_develop.as_ref(),
    )?;
    if raw_changed {
        ensure!(
            tinge_io::hash_file(&source)? == p.source.hash,
            "source image hash mismatch; source has changed"
        );
        if let Some(options) = &raw_develop {
            // Metadata validation alone cannot detect overflow from extreme levels.
            tinge_io::raw::load_with_options(&source, options)?;
        }
    }
    p.commit(label, recipe, pipeline, raw_develop)?;
    save(path, &p, true)?;
    Ok(p)
}
pub fn restore(path: &Path, expected: u64, target: u64, label: String) -> Result<Project> {
    let _lock = lock(path)?;
    let mut p = read(path)?;
    check_revision(&p, expected)?;
    let recipe = p.get_revision(target)?.recipe.clone();
    let pipeline = p.get_revision(target)?.color_pipeline.clone();
    let raw = p.get_revision(target)?.raw_develop.clone();
    p.commit(label, recipe, pipeline, raw)?;
    save(path, &p, true)?;
    Ok(p)
}
pub fn branch(path: &Path, expected: u64, name: String, checkout: bool) -> Result<Project> {
    let _lock = lock(path)?;
    let mut p = read(path)?;
    check_revision(&p, expected)?;
    ensure!(!name.is_empty(), "branch name cannot be empty");
    let target = if let Some(id) = p.branches.get(&name) {
        *id
    } else {
        p.branches.insert(name.clone(), p.revision);
        p.revision
    };
    if checkout {
        let recipe = p.get_revision(target)?.recipe.clone();
        let pipeline = p.get_revision(target)?.color_pipeline.clone();
        let raw = p.get_revision(target)?.raw_develop.clone();
        p.current_branch = name.clone();
        p.commit(format!("checkout {name}"), recipe, pipeline, raw)?;
    } else {
        let recipe = p.head()?.recipe.clone();
        let pipeline = p.head()?.color_pipeline.clone();
        let raw = p.head()?.raw_develop.clone();
        p.commit(format!("branch {name}"), recipe, pipeline, raw)?;
    }
    save(path, &p, true)?;
    Ok(p)
}
pub fn tag(path: &Path, expected: u64, name: String) -> Result<Project> {
    tag_revision(path, expected, expected, name)
}
/// Name an immutable revision while preserving the current grading head.
pub fn tag_revision(path: &Path, expected: u64, target: u64, name: String) -> Result<Project> {
    let _lock = lock(path)?;
    let mut p = read(path)?;
    check_revision(&p, expected)?;
    ensure!(
        !name.is_empty() && !p.tags.contains_key(&name),
        "tag name empty or already exists"
    );
    p.get_revision(target)?;
    ensure!(name.len() <= 256, "tag name exceeds 256 bytes");
    p.tags.insert(name.clone(), target);
    let recipe = p.head()?.recipe.clone();
    let pipeline = p.head()?.color_pipeline.clone();
    let raw = p.head()?.raw_develop.clone();
    p.commit(format!("tag {name}"), recipe, pipeline, raw)?;
    save(path, &p, true)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipeline_is_revisioned_hashed_and_restored_with_branches() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("input.png");
        let path = d.path().join("pipeline.tinge");
        let frame = tinge_core::Frame::new(1, 1, vec![[0.18, 0.18, 0.18, 1.0]]).unwrap();
        tinge_io::export(&frame, &input, Default::default()).unwrap();
        let pipeline: tinge_ocio::Pipeline =
            serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
        let original = init_with_pipeline(&path, &input, None, Some(pipeline)).unwrap();
        let original_hash = original.head().unwrap().recipe_hash.clone();
        let p = branch(&path, 0, "aces".into(), false).unwrap();
        assert_eq!(p.head().unwrap().recipe_hash, original_hash);
        let p = transaction(
            &path,
            1,
            vec![Edit::SetColorPipeline { pipeline: None }],
            "disable".into(),
            d.path(),
        )
        .unwrap();
        assert_ne!(p.head().unwrap().recipe_hash, original_hash);
        let p = branch(&path, 2, "aces".into(), true).unwrap();
        assert_eq!(p.head().unwrap().recipe_hash, original_hash);
        let mut p = restore(&path, 3, 0, "restore".into()).unwrap();
        assert_eq!(p.head().unwrap().recipe_hash, original_hash);
        p.history
            .last_mut()
            .unwrap()
            .color_pipeline
            .as_mut()
            .unwrap()
            .view = "Un-tone-mapped".into();
        assert!(p.validate().is_err());
        let before = fs::read(&path).unwrap();
        let mut bad = original.head().unwrap().color_pipeline.clone().unwrap();
        bad.engine_version = "stub".into();
        assert!(
            transaction(
                &path,
                4,
                vec![Edit::SetColorPipeline {
                    pipeline: Some(bad)
                }],
                "bad".into(),
                d.path()
            )
            .is_err()
        );
        assert_eq!(before, fs::read(path).unwrap());
    }
    #[test]
    fn concurrent_same_revision_has_one_winner() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("input.png");
        let path = d.path().join("race.tinge");
        let f = tinge_core::Frame::new(1, 1, vec![[0.2, 0.2, 0.2, 1.0]]).unwrap();
        tinge_io::export(&f, &input, Default::default()).unwrap();
        init(&path, &input, None).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let barrier = barrier.clone();
            let origin = d.path().to_path_buf();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                transaction(
                    &path,
                    0,
                    vec![Edit::SetOutput {
                        id: "source".into(),
                    }],
                    "concurrent".into(),
                    &origin,
                )
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .find_map(|r| r.as_ref().err())
                .unwrap()
                .downcast_ref::<RevisionConflict>()
                .is_some()
        );
        assert_eq!(load(&path).unwrap().revision, 1);
    }
    #[test]
    fn transaction_conflict_atomic_failure_restore_and_portable_source() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("input.png");
        let path = d.path().join("p.tinge");
        let f = tinge_core::Frame::new(1, 1, vec![[0.2, 0.4, 0.6, 1.0]]).unwrap();
        tinge_io::export(&f, &input, Default::default()).unwrap();
        init(&path, &input, None).unwrap();
        fs::remove_file(input).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            transaction(
                &path,
                0,
                vec![Edit::SetOutput {
                    id: "missing".into()
                }],
                "bad".into(),
                d.path()
            )
            .is_err()
        );
        assert_eq!(before, fs::read(&path).unwrap());
        let p = transaction(
            &path,
            0,
            vec![
                Edit::UpsertNode {
                    node: Node {
                        id: "exp".into(),
                        inputs: vec!["source".into()],
                        op: Operation::Exposure { stops: 1.0 },
                        mask: None,
                        mix: 1.0,
                        enabled: true,
                    },
                },
                Edit::SetOutput { id: "exp".into() },
            ],
            "test".into(),
            d.path(),
        )
        .unwrap();
        assert_eq!(p.revision, 1);
        assert!(
            transaction(
                &path,
                0,
                vec![Edit::SetOutput {
                    id: "source".into()
                }],
                "stale".into(),
                d.path()
            )
            .is_err()
        );
        let p = restore(&path, 1, 0, "undo".into()).unwrap();
        assert_eq!(p.revision, 2);
        assert_eq!(p.head().unwrap().recipe.output, "source");
        assert!(resolve(base(&path), &p.source.path).exists());
        let p = branch(&path, 2, "warm".into(), true).unwrap();
        assert_eq!(p.current_branch, "warm");
        assert_eq!(p.branches["main"], 2);
        assert_eq!(p.revision, 3);
    }
}
