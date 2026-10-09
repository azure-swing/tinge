use anyhow::{Context, Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
use tinge_color::{ColorSpace, Primaries, Transfer};
use tinge_core::{Frame, Recipe};
use tinge_engine::Engine;
use tinge_io::{self as io, ExportOptions};
use tinge_project::{self as project, Edit};

fn label() -> String {
    "agent edit".into()
}
fn edge() -> u32 {
    1600
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LutBakeSource {
    Recipe {
        recipe: Recipe,
        #[serde(default)]
        color_pipeline: Option<Box<tinge_ocio::Pipeline>>,
    },
    Project {
        project: PathBuf,
        #[serde(default)]
        revision: Option<u64>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CdlExportSource {
    Document {
        document: Box<tinge_ocio::cdl::CdlDocument>,
    },
    Project {
        project: PathBuf,
        #[serde(default)]
        revision: Option<u64>,
        nodes: Vec<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Adjust(crate::adjust::Adjust),
    Selections {
        project: PathBuf,
    },
    SelectionSave {
        project: PathBuf,
        expect_revision: u64,
        #[serde(default)]
        revision: Option<u64>,
        mask: tinge_core::Mask,
        #[serde(default)]
        note: String,
    },
    Cutout {
        input: PathBuf,
        output: PathBuf,
        options: tinge_core::cutout::CutoutOptions,
        #[serde(default)]
        matte: Option<PathBuf>,
        #[serde(default)]
        input_space: Option<ColorSpace>,
        #[serde(default)]
        color_pipeline: Option<tinge_ocio::Pipeline>,
        #[serde(default)]
        raw_develop: Option<io::RawDevelopOptions>,
        #[serde(default)]
        bit_depth: Option<u8>,
        #[serde(default)]
        output_space: Option<ColorSpace>,
        #[serde(default)]
        linear_unit_nits: Option<f32>,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        asset_base: Option<PathBuf>,
    },
    CdlInspect {
        input: PathBuf,
    },
    CdlImport {
        input: PathBuf,
        #[serde(default)]
        selector: Option<tinge_ocio::cdl::CdlSelector>,
        color_space: String,
        style: tinge_ocio::CdlStyle,
        #[serde(default)]
        inverse: bool,
    },
    CdlExport {
        source: CdlExportSource,
        output: PathBuf,
        #[serde(default)]
        overwrite: bool,
    },
    LutInspect {
        input: PathBuf,
    },
    LutBake {
        source: LutBakeSource,
        output: PathBuf,
        #[serde(default)]
        options: tinge_engine::bake::BakeOptions,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        asset_base: Option<PathBuf>,
    },
    Capabilities {},
    CleanupPlan {
        project: PathBuf,
        revision: u64,
    },
    Finalize {
        project: PathBuf,
        expect_revision: u64,
        revision: u64,
    },
    WorkfileRegister {
        project: PathBuf,
        file: PathBuf,
        revision: u64,
        role: crate::workfiles::Role,
    },
    OcioConfigs {},
    OcioInspect {
        config: tinge_ocio::ConfigSource,
        #[serde(default)]
        context: BTreeMap<String, String>,
    },
    OcioTransform {
        config: tinge_ocio::ConfigSource,
        transform: tinge_ocio::Transform,
        pixels: Vec<[f32; 4]>,
        #[serde(default)]
        context: BTreeMap<String, String>,
    },
    Schema {
        #[serde(default)]
        kind: SchemaKind,
        #[serde(default)]
        target: Option<String>,
    },
    Inspect {
        input: PathBuf,
    },
    RawPlan {
        input: PathBuf,
        #[serde(default)]
        options: io::RawDevelopOptions,
        #[serde(default)]
        sensor_points: Vec<[usize; 2]>,
    },
    Init {
        input: PathBuf,
        project: PathBuf,
        #[serde(default)]
        input_space: Option<ColorSpace>,
        #[serde(default)]
        color_pipeline: Option<tinge_ocio::Pipeline>,
        #[serde(default)]
        raw_develop: Option<io::RawDevelopOptions>,
    },
    Show {
        project: PathBuf,
    },
    ProjectInfo {
        project: PathBuf,
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default)]
        include_recipe: bool,
        #[serde(default)]
        history_limit: usize,
        #[serde(default)]
        before_revision: Option<u64>,
    },
    EditPreview {
        project: PathBuf,
        expect_revision: u64,
        edits: Vec<Edit>,
        #[serde(default)]
        output: Option<PathBuf>,
        #[serde(default = "label")]
        label: String,
        #[serde(default)]
        asset_base: Option<PathBuf>,
        #[serde(default = "edge")]
        max_edge: u32,
        #[serde(default)]
        overwrite: bool,
    },
    JobSubmit {
        request: Box<Request>,
        #[serde(default)]
        idempotency_key: Option<String>,
    },
    JobStatus {
        job: u64,
    },
    JobCancel {
        job: u64,
    },
    Configure {
        cache_budget_mib: usize,
        #[serde(default = "idle_seconds")]
        idle_seconds: u64,
    },
    CacheInfo {},
    ViewerOpen {
        project: PathBuf,
        #[serde(default)]
        port: u16,
    },
    ViewerStatus {},
    ViewerClose {
        project: PathBuf,
    },
    Apply {
        project: PathBuf,
        expect_revision: u64,
        edits: Vec<Edit>,
        #[serde(default = "label")]
        label: String,
        #[serde(default)]
        asset_base: Option<PathBuf>,
    },
    Restore {
        project: PathBuf,
        expect_revision: u64,
        revision: u64,
    },
    Branch {
        project: PathBuf,
        expect_revision: u64,
        name: String,
        #[serde(default)]
        checkout: bool,
    },
    Tag {
        project: PathBuf,
        expect_revision: u64,
        name: String,
    },
    Validate {
        recipe: Recipe,
        #[serde(default)]
        color_pipeline: Option<tinge_ocio::Pipeline>,
    },
    Grade {
        input: PathBuf,
        recipe: Recipe,
        output: PathBuf,
        #[serde(default)]
        input_space: Option<ColorSpace>,
        #[serde(default)]
        color_pipeline: Option<tinge_ocio::Pipeline>,
        #[serde(default)]
        raw_develop: Option<io::RawDevelopOptions>,
        #[serde(default)]
        bit_depth: Option<u8>,
        #[serde(default)]
        output_space: Option<ColorSpace>,
        #[serde(default)]
        linear_unit_nits: Option<f32>,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        asset_base: Option<PathBuf>,
    },
    Render {
        project: PathBuf,
        output: PathBuf,
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default)]
        bit_depth: Option<u8>,
        #[serde(default)]
        output_space: Option<ColorSpace>,
        #[serde(default)]
        linear_unit_nits: Option<f32>,
        #[serde(default)]
        max_edge: Option<u32>,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        temporary: bool,
    },
    Analyze {
        input: PathBuf,
        #[serde(default)]
        input_space: Option<ColorSpace>,
        #[serde(default)]
        color_pipeline: Option<tinge_ocio::Pipeline>,
        #[serde(default)]
        raw_develop: Option<io::RawDevelopOptions>,
        #[serde(default)]
        recipe: Option<Recipe>,
        #[serde(default)]
        asset_base: Option<PathBuf>,
    },
    Stats {
        project: PathBuf,
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default)]
        scopes: bool,
    },
    Preview {
        project: PathBuf,
        #[serde(default)]
        output: Option<PathBuf>,
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default = "edge")]
        max_edge: u32,
        #[serde(default)]
        overwrite: bool,
        #[serde(default = "include_analysis")]
        include_analysis: bool,
    },
    Compare {
        project: PathBuf,
        #[serde(default)]
        output: Option<PathBuf>,
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default = "edge")]
        max_edge: u32,
        #[serde(default)]
        overwrite: bool,
    },
    Batch {
        jobs: Vec<Request>,
        #[serde(default)]
        stop_on_error: bool,
    },
    ClearCache {},
}
fn idle_seconds() -> u64 {
    60
}
fn include_analysis() -> bool {
    true
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    #[default]
    Request,
    Recipe,
    Edits,
}
pub fn error_json(e: &anyhow::Error) -> Value {
    if let Some(c) = e.downcast_ref::<project::RevisionConflict>() {
        json!({"code":"revision_conflict","message":e.to_string(),"expected_revision":c.expected,"actual_revision":c.actual})
    } else if let Some(e) = e.downcast_ref::<crate::jobs::JobError>() {
        json!({"code":e.code,"message":e.message,"retryable":e.code == "queue_full"})
    } else {
        json!({"code":"operation_failed","message":format!("{e:#}")})
    }
}
pub struct Session {
    pub engine: Engine,
    pub previews: BTreeMap<String, PathBuf>,
    pub progress: bool,
    pub cancel: Arc<AtomicBool>,
    pub observer: Option<Arc<dyn Fn(Value) + Send + Sync>>,
    pub jobs: Option<crate::jobs::Jobs>,
    pub viewers: crate::viewers::Viewers,
    pub analyses: BTreeMap<String, Value>,
    pub idle_seconds: u64,
    pub persistent: bool,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            engine: Engine::new(),
            previews: BTreeMap::new(),
            progress: false,
            cancel: Arc::new(AtomicBool::new(false)),
            observer: None,
            jobs: None,
            viewers: Default::default(),
            analyses: Default::default(),
            idle_seconds: idle_seconds(),
            persistent: false,
        }
    }
}
pub fn options(path: &Path, bits: Option<u8>, overwrite: bool) -> ExportOptions {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bits = bits.unwrap_or(match ext.as_str() {
        "jpg" | "jpeg" => 8,
        "exr" => 32,
        _ => 16,
    });
    ExportOptions {
        bit_depth: bits,
        space: ColorSpace {
            primaries: Primaries::Srgb,
            transfer: if bits == 32 {
                Transfer::Linear
            } else {
                Transfer::Srgb
            },
        },
        overwrite,
        ..Default::default()
    }
}
fn export_options(
    path: &Path,
    bits: Option<u8>,
    overwrite: bool,
    requested: Option<ColorSpace>,
    linear_unit_nits: Option<f32>,
    pipeline: Option<&tinge_ocio::Pipeline>,
) -> ExportOptions {
    let mut opt = options(path, bits, overwrite);
    opt.linear_unit_nits = linear_unit_nits;
    if let Some(space) = requested {
        opt.space = space;
    } else if opt.bit_depth != 32
        && let Some(pipeline) = pipeline
    {
        opt.space = pipeline.display.color_space();
    }
    opt
}
fn revision_json(path: &Path, p: &project::Project) -> Value {
    json!({"project":path,"revision":p.revision,"branch":p.current_branch,"recipe_hash":p.head().map(|h|h.recipe_hash.clone()).unwrap_or_default()})
}
pub fn project_info(
    path: &Path,
    revision: Option<u64>,
    include_recipe: bool,
    history_limit: usize,
    before_revision: Option<u64>,
) -> Result<Value> {
    ensure!(history_limit <= 100, "history_limit must be 0..100");
    let p = project::load(path)?;
    let selected = p.get_revision(revision.unwrap_or(p.revision))?;
    let page: Vec<_> = p
        .history
        .iter()
        .rev()
        .filter(|r| before_revision.is_none_or(|before| r.id < before))
        .take(history_limit)
        .collect();
    let next = page
        .last()
        .and_then(|last| p.history.iter().any(|r| r.id < last.id).then_some(last.id));
    let mut data = json!({"project":path,"head":p.revision,"revision":selected.id,"branch":p.current_branch,"recipe_hash":selected.recipe_hash,"label":selected.label,"source":p.source,"output_node":selected.recipe.output,"node_count":selected.recipe.nodes.len(),"mask_count":selected.recipe.masks.len(),"history_count":p.history.len(),"next_before_revision":next,"build_version":env!("CARGO_PKG_VERSION"),"history":page.iter().map(|r| json!({"revision":r.id,"parent":r.parent,"label":r.label,"timestamp":r.timestamp})).collect::<Vec<_>>()});
    if include_recipe {
        data["recipe"] = serde_json::to_value(&selected.recipe)?;
        data["color_pipeline"] = serde_json::to_value(&selected.color_pipeline)?;
        data["raw_develop"] = serde_json::to_value(&selected.raw_develop)?;
    }
    data["final_revision"] = json!(crate::workfiles::final_revision_for(path, &p));
    Ok(data)
}
impl Session {
    pub fn clear_cached_data(&mut self) {
        self.engine.clear_cache();
        self.analyses.clear();
    }
    fn analysis(&mut self, frame: &Frame, p: &project::Project, revision: u64) -> Result<Value> {
        let key = format!(
            "{}:{}:{}",
            p.source.hash,
            serde_json::to_string(&p.source.color_space)?,
            p.get_revision(revision)?.recipe_hash
        );
        if let Some(data) = self.analyses.get(&key) {
            return Ok(data.clone());
        }
        let data = serde_json::to_value(tinge_core::scopes::analyze(frame))?;
        if self.analyses.len() >= 8 {
            self.analyses.clear();
        }
        self.analyses.insert(key, data.clone());
        Ok(data)
    }
    fn project_frame(
        &mut self,
        path: &Path,
        revision: Option<u64>,
    ) -> Result<(Arc<Frame>, project::Project)> {
        let p = project::load(path)?;
        let show = self.progress;
        let observer = self.observer.clone();
        let f =
            self.engine
                .project_render(path, &p, revision, self.cancel.as_ref(), &mut |v| {
                    if let Some(observer) = &observer { observer(json!({"phase":"render","node":v.node,"completed":v.completed,"total":v.total,"cached":v.cached})); }
                    if show {
                        eprintln!("{}", json!({"event":"progress","data":v}));
                    }
                })?;
        Ok((f, p))
    }
    fn preview_resource(&mut self, path: &Path) -> Result<String> {
        let path = std::fs::canonicalize(path)?;
        let hash = io::hash_file(&path)?;
        let uri = format!("tinge://preview/{hash}");
        self.previews.retain(|id, old| old != &path || id == &uri);
        self.previews.insert(uri.clone(), path);
        while self.previews.len() > 64 {
            let old = self
                .previews
                .keys()
                .find(|key| *key != &uri)
                .cloned()
                .unwrap();
            self.previews.remove(&old);
        }
        Ok(uri)
    }
    pub fn run(&mut self, request: Request) -> Result<Value> {
        self.run_depth(request, 0)
    }
    fn run_depth(&mut self, request: Request, depth: usize) -> Result<Value> {
        ensure!(depth <= 4, "batch nesting exceeds 4");
        ensure!(
            !self.cancel.load(std::sync::atomic::Ordering::Relaxed),
            "operation cancelled"
        );
        match request {
            Request::Selections { project } => {
                Ok(serde_json::to_value(project::selections::load(&project)?)?)
            }
            Request::SelectionSave {
                project,
                expect_revision,
                revision,
                mask,
                note,
            } => {
                project::selections::validate_mask(&mask)?;
                ensure!(note.len() <= 4096, "selection note exceeds 4096 bytes");
                let (frame, p) = self.project_frame(&project, revision)?;
                let rev = p.get_revision(revision.unwrap_or(p.revision))?;
                let item = project::selections::Selection {
                    id: crate::web::random_token()?,
                    revision: rev.id,
                    source_hash: p.source.hash.clone(),
                    recipe_hash: rev.recipe_hash.clone(),
                    output_node: rev.recipe.output.clone(),
                    width: frame.width,
                    height: frame.height,
                    timestamp: 0,
                    mask,
                    note,
                };
                Ok(serde_json::to_value(project::selections::save(
                    &project,
                    expect_revision,
                    item,
                )?)?)
            }
            Request::Cutout {
                input,
                output,
                options,
                matte,
                input_space,
                color_pipeline,
                raw_develop,
                bit_depth,
                output_space,
                linear_unit_nits,
                overwrite,
                asset_base,
            } => {
                ensure!(
                    options.output == tinge_core::cutout::CutoutOutput::Cutout,
                    "cutout command requires options.output=cutout; use --matte for scalar data or a recipe matte node"
                );
                options.validate()?;
                ensure_distinct(&input, &output)?;
                let ext = output
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                ensure!(
                    ["png", "tif", "tiff", "exr"].contains(&ext.as_str()),
                    "cutout output must support alpha: PNG, TIFF, or EXR"
                );
                ensure!(
                    !(["tif", "tiff"].contains(&ext.as_str()) && bit_depth == Some(32)),
                    "transparent TIFF supports 8/16 bits; use EXR for float alpha"
                );
                let origin = asset_base.unwrap_or(std::env::current_dir()?);
                let color_pipeline = color_pipeline.map(|p| p.resolved_at(&origin));
                let mut protected = options
                    .assets()
                    .into_iter()
                    .map(|p| project::resolve(&origin, &p))
                    .collect::<Vec<_>>();
                protected.push(input.clone());
                if let Some(p) = &color_pipeline {
                    if let tinge_ocio::ConfigSource::File { path }
                    | tinge_ocio::ConfigSource::Frozen { path, .. } = p.source()
                    {
                        protected.push(path);
                    }
                    let mut probe = [[0.18, 0.18, 0.18, 1.0]];
                    if let Some(report) = p.to_working(&mut probe)? {
                        protected.extend(report.files.into_iter().map(PathBuf::from));
                    }
                    protected.extend(
                        p.to_display(&mut probe)?
                            .files
                            .into_iter()
                            .map(PathBuf::from),
                    );
                }
                for dest in std::iter::once(&output).chain(matte.iter()) {
                    for source in &protected {
                        ensure_distinct(source, dest)?;
                    }
                    ensure!(
                        overwrite || !dest.exists(),
                        "output exists; use overwrite: {}",
                        dest.display()
                    );
                }
                if let Some(matte) = &matte {
                    ensure_distinct(&output, matte)?;
                    ensure!(
                        matte
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
                        "matte output must be PNG"
                    );
                }
                let input = tinge_engine::load_source_with_options(
                    &input,
                    input_space,
                    color_pipeline.as_ref(),
                    raw_develop.as_ref(),
                )?;
                let (frame, report) = tinge_core::cutout::run(
                    &input,
                    &options,
                    &|p, w, h| io::load_matte(&project::resolve(&origin, p), w, h),
                    self.cancel.as_ref(),
                )?;
                let (export, transform) = tinge_engine::export_frame(
                    &frame,
                    &output,
                    export_options(
                        &output,
                        bit_depth,
                        overwrite,
                        output_space,
                        linear_unit_nits,
                        color_pipeline.as_ref(),
                    ),
                    color_pipeline.as_ref(),
                )?;
                if let Some(path) = &matte {
                    io::export_matte(&frame, path, 16, overwrite)?;
                }
                Ok(
                    json!({"export":export,"matte":matte,"matte_bit_depth":if matte.is_some(){Some(16)}else{None},"cutout":report,"options":options,"color_pipeline":color_pipeline,"ocio_transform":transform,"raw_develop":raw_develop}),
                )
            }
            Request::CdlInspect { input } => {
                let (document, hash) = tinge_ocio::cdl::read(&input)?;
                Ok(
                    json!({"input":input,"hash":hash,"document":document,"engine_version":tinge_ocio::version()?,"scope":"native ASC SOP/saturation and standard descriptions; strict UTF-8 XML; unsupported XML extensions and ColorCorrectionRef are rejected"}),
                )
            }
            Request::CdlImport {
                input,
                selector,
                color_space,
                style,
                inverse,
            } => {
                let (document, hash) = tinge_ocio::cdl::read(&input)?;
                let (index, grade) =
                    document.import(&hash, selector.as_ref(), color_space, style, inverse)?;
                Ok(
                    json!({"input":input,"hash":hash,"correction_index":index,"grade":grade,"op":{"type":"ocio_grade","grade":grade},"color_space_validated_against_config":false,"scope":"apply the returned op using a project color_pipeline; config compilation is required before commit"}),
                )
            }
            Request::CdlExport {
                source,
                output,
                overwrite,
            } => {
                use tinge_ocio::cdl::{CdlDocument, CdlFormat, CdlMetadata, correction_from_grade};
                let format = CdlFormat::from_path(&output)?;
                let (document, context) = match source {
                    CdlExportSource::Document { document } => {
                        ensure!(
                            document.format == format,
                            "CDL output extension must match document format"
                        );
                        (*document, json!(null))
                    }
                    CdlExportSource::Project {
                        project: path,
                        revision,
                        nodes,
                    } => {
                        ensure!(
                            !nodes.is_empty() && nodes.len() <= 4096,
                            "select 1..4096 CDL nodes"
                        );
                        let p = project::load(&path)?;
                        protect_project(&path, &p, &output)?;
                        let rev = p.get_revision(revision.unwrap_or(p.revision))?;
                        let mut corrections = Vec::new();
                        let mut seen = std::collections::BTreeSet::new();
                        let mut metadata: Option<CdlMetadata> = None;
                        let mut selected = Vec::new();
                        for id in &nodes {
                            ensure!(seen.insert(id), "duplicate CDL node selection: {id}");
                            let node = rev
                                .recipe
                                .nodes
                                .iter()
                                .find(|n| &n.id == id)
                                .ok_or_else(|| anyhow::anyhow!("CDL node not found: {id}"))?;
                            let tinge_core::Operation::OcioGrade { grade } = &node.op else {
                                anyhow::bail!("selected node {id} is not an OCIO CDL");
                            };
                            let (correction, parent) = correction_from_grade(grade, id)?;
                            if let Some(parent) = parent {
                                ensure!(
                                    metadata.as_ref().is_none_or(|m| m == &parent),
                                    "selected CDLs have conflicting collection descriptions; export separately"
                                );
                                metadata = Some(parent);
                            }
                            corrections.push(correction);
                            selected.push(node);
                        }
                        (
                            CdlDocument {
                                format,
                                metadata: metadata.unwrap_or_default(),
                                corrections,
                            },
                            json!({"project":path,"revision":rev.id,"recipe_hash":rev.recipe_hash,"nodes":selected,"color_pipeline":rev.color_pipeline,"raw_develop":rev.raw_develop}),
                        )
                    }
                };
                let xml = tinge_ocio::cdl::write(&document)?;
                io::atomic_bytes(&output, xml.as_bytes(), overwrite)?;
                Ok(
                    json!({"output":output,"hash":blake3::hash(xml.as_bytes()).to_hex().to_string(),"bytes":xml.len(),"document":document,"source_context":context,"scope":"stored forward SOP/saturation parameters and standard descriptions; XML does not encode processing space, style, direction, graph inputs, masks or node mix; not an evaluated graph export"}),
                )
            }
            Request::LutInspect { input } => {
                let hash = io::hash_file(&input)?;
                let text = std::fs::read_to_string(&input)?;
                ensure!(
                    blake3::hash(text.as_bytes()).to_hex().as_str() == hash,
                    "LUT changed during inspection"
                );
                let lut = tinge_core::lut::CubeLut::parse(&text)?;
                Ok(
                    json!({"input":input,"hash":hash,"lut":lut.info(),"interpolation":["trilinear","tetrahedral"],"outside_domain":"clamp"}),
                )
            }
            Request::LutBake {
                source,
                output,
                options,
                overwrite,
                asset_base,
            } => {
                ensure!(
                    output
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("cube")),
                    "LUT bake output must use .cube"
                );
                let (recipe, pipeline, origin, revision) = match source {
                    LutBakeSource::Recipe {
                        recipe,
                        color_pipeline,
                    } => {
                        let origin = asset_base.unwrap_or(std::env::current_dir()?);
                        for n in &recipe.nodes {
                            if let tinge_core::Operation::Lut { path, .. } = &n.op {
                                ensure_distinct(&project::resolve(&origin, path), &output)?;
                            }
                        }
                        if let Some(p) = &color_pipeline
                            && let tinge_ocio::PipelineConfig::Source(
                                tinge_ocio::ConfigSource::File { path }
                                | tinge_ocio::ConfigSource::Frozen { path, .. },
                            ) = &p.config
                        {
                            let config = if path.is_absolute() {
                                path.clone()
                            } else {
                                origin.join(path)
                            };
                            ensure_distinct(&config, &output)?;
                        }
                        for file in tinge_engine::bake::editable_dependencies(
                            &recipe,
                            color_pipeline.as_deref(),
                            &origin,
                            &options,
                        )? {
                            ensure_distinct(Path::new(&file), &output)?;
                        }
                        (recipe, color_pipeline.map(|p| *p), origin, None)
                    }
                    LutBakeSource::Project {
                        project: path,
                        revision,
                    } => {
                        ensure!(
                            asset_base.is_none(),
                            "project LUT baking uses the project's asset base"
                        );
                        let p = project::load(&path)?;
                        protect_project(&path, &p, &output)?;
                        let rev = p.get_revision(revision.unwrap_or(p.revision))?;
                        tinge_engine::verify_recipe_assets(&rev.recipe, project::base(&path))?;
                        (
                            rev.recipe.clone(),
                            rev.color_pipeline.clone(),
                            project::base(&path).to_path_buf(),
                            Some(rev.id),
                        )
                    }
                };
                let (text, report) = tinge_engine::bake::bake(recipe, pipeline, &origin, options)?;
                io::atomic_bytes(&output, text.as_bytes(), overwrite)?;
                Ok(
                    json!({"output":output,"hash":blake3::hash(text.as_bytes()).to_hex().to_string(),"bytes":text.len(),"revision":revision,"bake":report,"source_development_included":false,"input_scope":"independent RGB samples; RAW decoding/development and source image not baked"}),
                )
            }
            Request::Capabilities {} => Ok(capabilities()),
            Request::OcioConfigs {} => Ok(
                json!({"engine_version":tinge_ocio::version()?,"configs":tinge_ocio::builtin_configs()?}),
            ),
            Request::OcioInspect { config, context } => Ok(serde_json::to_value(
                tinge_ocio::inspect_with_context(&config, &context)?,
            )?),
            Request::OcioTransform {
                config,
                transform,
                mut pixels,
                context,
            } => {
                ensure!(
                    pixels.len() <= 1_000_000,
                    "numeric OCIO request exceeds 1M pixels; use image grading for images"
                );
                let report =
                    tinge_ocio::apply_with_context(&config, &transform, &mut pixels, &context)?;
                Ok(json!({"pixels":pixels,"transform":transform,"report":report}))
            }
            Request::Schema { kind, target } => {
                if let Some(target) = target {
                    crate::protocol::target_schema(&target)
                } else {
                    Ok(match kind {
                        SchemaKind::Request => {
                            serde_json::to_value(schemars::schema_for!(Request))?
                        }
                        SchemaKind::Recipe => serde_json::to_value(schemars::schema_for!(Recipe))?,
                        SchemaKind::Edits => {
                            serde_json::to_value(schemars::schema_for!(Vec<Edit>))?
                        }
                    })
                }
            }
            Request::Inspect { input } => Ok(serde_json::to_value(io::inspect(&input)?)?),
            Request::RawPlan {
                input,
                options,
                sensor_points,
            } => Ok(serde_json::to_value(io::raw::plan_with_probes(
                &input,
                &options,
                &sensor_points,
            )?)?),
            Request::Init {
                input,
                project,
                input_space,
                color_pipeline,
                raw_develop,
            } => {
                let p = project::init_with_source_options(
                    &project,
                    &input,
                    input_space,
                    color_pipeline,
                    raw_develop,
                )?;
                Ok(revision_json(&project, &p))
            }
            Request::Show { project } => Ok(serde_json::to_value(project::load(&project)?)?),
            Request::ProjectInfo {
                project,
                revision,
                include_recipe,
                history_limit,
                before_revision,
            } => project_info(
                &project,
                revision,
                include_recipe,
                history_limit,
                before_revision,
            ),
            Request::JobSubmit {
                request,
                idempotency_key,
            } => {
                ensure!(
                    self.persistent,
                    "job_submit requires a persistent MCP or serve session"
                );
                if self.jobs.is_none() {
                    self.jobs = Some(crate::jobs::Jobs::new(
                        self.engine.cache_budget_bytes,
                        self.idle_seconds,
                    )?);
                }
                let jobs = self.jobs.as_ref().unwrap();
                jobs.submit(*request, idempotency_key)
            }
            Request::JobStatus { job } => {
                let jobs = self
                    .jobs
                    .as_ref()
                    .context("no background jobs in this session")?;
                let data = jobs.status(job)?;
                let resources = jobs.resources(job);
                let protected: std::collections::BTreeSet<_> = resources.keys().cloned().collect();
                self.previews.extend(resources);
                while self.previews.len() > 64 {
                    let key = self
                        .previews
                        .keys()
                        .find(|key| !protected.contains(*key))
                        .cloned()
                        .unwrap();
                    self.previews.remove(&key);
                }
                Ok(data)
            }
            Request::JobCancel { job } => self
                .jobs
                .as_ref()
                .context("no background jobs in this session")?
                .cancel(job),
            Request::Configure {
                cache_budget_mib,
                idle_seconds,
            } => {
                ensure!(
                    cache_budget_mib <= 4096 && (1..=3600).contains(&idle_seconds),
                    "cache budget must be 0..4096 MiB; idle_seconds must be 1..3600"
                );
                self.clear_cached_data();
                self.engine.cache_budget_bytes = cache_budget_mib * 1024 * 1024;
                self.idle_seconds = idle_seconds;
                if let Some(jobs) = &self.jobs {
                    jobs.configure(self.engine.cache_budget_bytes, idle_seconds);
                }
                Ok(
                    json!({"cache_budget_mib":cache_budget_mib,"idle_seconds":idle_seconds,"budget_scope":"each engine cache, not total process memory","build_version":env!("CARGO_PKG_VERSION")}),
                )
            }
            Request::CacheInfo {} => Ok(
                json!({"main_cached_bytes":self.engine.cache_usage_bytes(),"job_cached_bytes":self.jobs.as_ref().map_or(0,|jobs|jobs.cache_usage_bytes()),"cache_budget_mib":self.engine.cache_budget_bytes/(1024*1024),"idle_seconds":self.idle_seconds,"scope":"cached Frame pixels; excludes active rendering, PNG buffers, managed viewers and OS allocator overhead"}),
            ),
            Request::ViewerOpen { project, port } => {
                ensure!(
                    self.persistent,
                    "viewer_open requires a persistent MCP or serve session; CLI callers use view"
                );
                self.viewers.open(&project, port)
            }
            Request::ViewerStatus {} => self.viewers.status(),
            Request::ViewerClose { project } => self.viewers.close(&project),
            Request::CleanupPlan { project, revision } => {
                crate::workfiles::cleanup_plan(&project, revision)
            }
            Request::Finalize {
                project,
                expect_revision,
                revision,
            } => {
                let result = crate::workfiles::finalize(&project, expect_revision, revision)?;
                self.previews.retain(|_, file| file.exists());
                Ok(result)
            }
            Request::WorkfileRegister {
                project,
                file,
                revision,
                role,
            } => crate::workfiles::register(&project, &file, revision, role),
            Request::Adjust(options) => self.run_depth(options.prepare()?, depth + 1),
            Request::EditPreview {
                project,
                expect_revision,
                edits,
                output,
                label,
                asset_base,
                max_edge,
                overwrite,
            } => {
                let previous = project::load(&project)?;
                let output = match output {
                    Some(output) => output,
                    None => crate::workfiles::edit_path(&project)?,
                };
                protect_project(&project, &previous, &output)?;
                ensure_png(&output)?;
                ensure!(
                    max_edge > 0 && max_edge <= 16384,
                    "max_edge must be 1..16384"
                );
                ensure!(
                    overwrite || !output.exists(),
                    "output exists; choose another path or overwrite"
                );
                let origin = asset_base.unwrap_or(std::env::current_dir()?);
                let p = project::transaction(&project, expect_revision, edits, label, &origin)?;
                let mut receipt = revision_json(&project, &p);
                receipt["committed"] = json!(true);
                if let Some(observer) = &self.observer {
                    observer(receipt.clone());
                }
                match self.run_depth(
                    Request::Preview {
                        project,
                        output: Some(output),
                        revision: Some(p.revision),
                        max_edge,
                        overwrite,
                        include_analysis: false,
                    },
                    depth + 1,
                ) {
                    Ok(preview) => {
                        receipt["preview"] = preview;
                    }
                    Err(error) => {
                        receipt["preview_error"] = error_json(&error);
                    }
                }
                Ok(receipt)
            }
            Request::Apply {
                project,
                expect_revision,
                edits,
                label,
                asset_base,
            } => {
                let origin = asset_base.unwrap_or(std::env::current_dir()?);
                let p = project::transaction(&project, expect_revision, edits, label, &origin)?;
                Ok(revision_json(&project, &p))
            }
            Request::Restore {
                project,
                expect_revision,
                revision,
            } => {
                let p = project::restore(
                    &project,
                    expect_revision,
                    revision,
                    format!("restore revision {revision}"),
                )?;
                Ok(revision_json(&project, &p))
            }
            Request::Branch {
                project,
                expect_revision,
                name,
                checkout,
            } => {
                let p = project::branch(&project, expect_revision, name, checkout)?;
                Ok(revision_json(&project, &p))
            }
            Request::Tag {
                project,
                expect_revision,
                name,
            } => {
                let p = project::tag(&project, expect_revision, name)?;
                Ok(revision_json(&project, &p))
            }
            Request::Validate {
                recipe,
                color_pipeline,
            } => {
                let plan = recipe.validate()?;
                let origin = std::env::current_dir()?;
                let color_pipeline = color_pipeline.map(|p| p.resolved_at(&origin));
                let processors = project::compile_color_nodes(&recipe, color_pipeline.as_ref())?;
                let reports: BTreeMap<_, _> = processors
                    .iter()
                    .map(|(id, p)| (id, p.identity()))
                    .collect();
                Ok(
                    json!({"valid":true,"topological_order":plan,"output":recipe.output,"ocio_nodes":reports}),
                )
            }
            Request::Grade {
                input,
                recipe,
                output,
                input_space,
                color_pipeline,
                raw_develop,
                bit_depth,
                output_space,
                linear_unit_nits,
                overwrite,
                asset_base,
            } => {
                ensure_distinct(&input, &output)?;
                let origin = asset_base.unwrap_or(std::env::current_dir()?);
                let color_pipeline = color_pipeline.map(|p| p.resolved_at(&origin));
                let input = Arc::new(tinge_engine::load_source_with_options(
                    &input,
                    input_space,
                    color_pipeline.as_ref(),
                    raw_develop.as_ref(),
                )?);
                let show = self.progress;
                let f = self.engine.render_with_context(
                    input,
                    &recipe,
                    tinge_engine::RenderContext {
                        asset_base: &origin,
                        color_pipeline: color_pipeline.as_ref(),
                    },
                    self.cancel.as_ref(),
                    &mut |v| {
                        if show {
                            eprintln!("{}", json!({"event":"progress","data":v}));
                        }
                    },
                )?;
                let (mut report, transform) = tinge_engine::export_frame(
                    &f,
                    &output,
                    export_options(
                        &output,
                        bit_depth,
                        overwrite,
                        output_space,
                        linear_unit_nits,
                        color_pipeline.as_ref(),
                    ),
                    color_pipeline.as_ref(),
                )?;
                report.warnings.push("One-shot grade does not freeze external assets; use a project for reproducible history.".into());
                let mut result = serde_json::to_value(report)?;
                result["color_pipeline"] = serde_json::to_value(color_pipeline)?;
                result["ocio_transform"] = serde_json::to_value(transform)?;
                result["raw_develop"] = serde_json::to_value(raw_develop)?;
                Ok(result)
            }
            Request::Render {
                project,
                output,
                revision,
                bit_depth,
                output_space,
                linear_unit_nits,
                max_edge,
                overwrite,
                temporary,
            } => {
                let (f, p) = self.project_frame(&project, revision)?;
                protect_project(&project, &p, &output)?;
                let f = if let Some(edge) = max_edge {
                    Arc::new(f.resized(edge)?)
                } else {
                    f
                };
                let pipeline = p
                    .get_revision(revision.unwrap_or(p.revision))?
                    .color_pipeline
                    .as_ref()
                    .map(|p| p.resolved_at(project::base(&project)));
                let (report, transform) = tinge_engine::export_frame(
                    &f,
                    &output,
                    export_options(
                        &output,
                        bit_depth,
                        overwrite,
                        output_space,
                        linear_unit_nits,
                        pipeline.as_ref(),
                    ),
                    pipeline.as_ref(),
                )?;
                let mut result = json!({"project":project,"revision":revision.unwrap_or(p.revision),"export":report,"color_pipeline":pipeline,"raw_develop":p.get_revision(revision.unwrap_or(p.revision))?.raw_develop,"ocio_transform":transform});
                register_workfile(
                    &mut result,
                    &project,
                    &output,
                    revision.unwrap_or(p.revision),
                    if temporary {
                        crate::workfiles::Role::Draft
                    } else {
                        crate::workfiles::Role::Export
                    },
                );
                Ok(result)
            }
            Request::Analyze {
                input,
                input_space,
                color_pipeline,
                raw_develop,
                recipe,
                asset_base,
            } => {
                let origin = asset_base.unwrap_or(std::env::current_dir()?);
                let color_pipeline = color_pipeline.map(|p| p.resolved_at(&origin));
                let f = Arc::new(tinge_engine::load_source_with_options(
                    &input,
                    input_space,
                    color_pipeline.as_ref(),
                    raw_develop.as_ref(),
                )?);
                let f = if let Some(r) = recipe {
                    self.engine.render_with_context(
                        f,
                        &r,
                        tinge_engine::RenderContext {
                            asset_base: &origin,
                            color_pipeline: color_pipeline.as_ref(),
                        },
                        self.cancel.as_ref(),
                        &mut |_| {},
                    )?
                } else {
                    f
                };
                let mut analysis = serde_json::to_value(tinge_core::scopes::analyze(&f))?;
                analysis["raw_develop"] = serde_json::to_value(raw_develop)?;
                Ok(analysis)
            }
            Request::Stats {
                project,
                revision,
                scopes,
            } => {
                let (f, p) = self.project_frame(&project, revision)?;
                let analysis = self.analysis(&f, &p, revision.unwrap_or(p.revision))?;
                let mut result = json!({"project":project,"revision":revision.unwrap_or(p.revision),"analysis":analysis});
                if scopes {
                    result["scopes"] = serde_json::to_value(tinge_core::scopes::scopes(&f))?;
                }
                Ok(result)
            }
            Request::Preview {
                project,
                output,
                revision,
                max_edge,
                overwrite,
                include_analysis,
            } => {
                let (f, p) = self.project_frame(&project, revision)?;
                let managed = output.is_none();
                let output = match output {
                    Some(output) => output,
                    None => crate::workfiles::preview_path(
                        &project,
                        revision.unwrap_or(p.revision),
                        max_edge,
                        false,
                    )?,
                };
                protect_project(&project, &p, &output)?;
                ensure_png(&output)?;
                let preview = f.resized(max_edge)?;
                let pipeline = p
                    .get_revision(revision.unwrap_or(p.revision))?
                    .color_pipeline
                    .as_ref()
                    .map(|p| p.resolved_at(project::base(&project)));
                let preview_pipeline = pipeline.as_ref().map(|p| p.for_preview()).transpose()?;
                let (report, transform) = tinge_engine::export_frame(
                    &preview,
                    &output,
                    options(&output, Some(8), overwrite || managed),
                    preview_pipeline.as_ref(),
                )?;
                let uri = self.preview_resource(&output)?;
                let mut data = json!({"project":project,"revision":revision.unwrap_or(p.revision),"recipe_hash":p.get_revision(revision.unwrap_or(p.revision))?.recipe_hash,"display_transform":if pipeline.is_some() {"OCIO sRGB display/view"} else {"linear sRGB to sRGB, hard clip"},"color_pipeline":pipeline,"preview_pipeline":preview_pipeline,"ocio_transform":transform,"preview":report,"resource_uri":uri});
                if include_analysis {
                    data["analysis"] = self.analysis(&f, &p, revision.unwrap_or(p.revision))?;
                }
                register_workfile(
                    &mut data,
                    &project,
                    &output,
                    revision.unwrap_or(p.revision),
                    crate::workfiles::Role::Preview,
                );
                Ok(data)
            }
            Request::Compare {
                project,
                output,
                revision,
                max_edge,
                overwrite,
            } => {
                let (graded, p) = self.project_frame(&project, revision)?;
                let managed = output.is_none();
                let output = match output {
                    Some(output) => output,
                    None => crate::workfiles::preview_path(
                        &project,
                        revision.unwrap_or(p.revision),
                        max_edge,
                        true,
                    )?,
                };
                protect_project(&project, &p, &output)?;
                ensure_png(&output)?;
                let pipeline = p
                    .get_revision(revision.unwrap_or(p.revision))?
                    .color_pipeline
                    .as_ref()
                    .map(|p| p.resolved_at(project::base(&project)));
                let source = self
                    .engine
                    .project_source(&project, &p, revision, self.cancel.as_ref())?
                    .resized(max_edge)?;
                let graded = graded.resized(max_edge)?;
                let w = source.width + graded.width;
                let h = source.height.max(graded.height);
                let mut pixels = vec![[0.025, 0.025, 0.025, 1.0]; (w * h) as usize];
                for (f, offset) in [(&source, 0), (&graded, source.width)] {
                    for y in 0..f.height {
                        let row = (y * w + offset) as usize;
                        let src = (y * f.width) as usize;
                        pixels[row..row + f.width as usize]
                            .copy_from_slice(&f.pixels[src..src + f.width as usize]);
                    }
                }
                let compare = Frame::new(w, h, pixels)?;
                let preview_pipeline = pipeline.as_ref().map(|p| p.for_preview()).transpose()?;
                let (report, transform) = tinge_engine::export_frame(
                    &compare,
                    &output,
                    options(&output, Some(8), overwrite || managed),
                    preview_pipeline.as_ref(),
                )?;
                let uri = self.preview_resource(&output)?;
                let mut result = json!({"project":project,"revision":revision.unwrap_or(p.revision),"layout":"original left / graded right","color_pipeline":pipeline,"preview_pipeline":preview_pipeline,"ocio_transform":transform,"preview":report,"resource_uri":uri});
                register_workfile(
                    &mut result,
                    &project,
                    &output,
                    revision.unwrap_or(p.revision),
                    crate::workfiles::Role::Preview,
                );
                Ok(result)
            }
            Request::Batch {
                jobs,
                stop_on_error,
            } => {
                ensure!(jobs.len() <= 1000, "batch exceeds 1000 jobs");
                let mut results = Vec::new();
                let mut failures = 0;
                for (i, job) in jobs.into_iter().enumerate() {
                    match self.run_depth(job, depth + 1) {
                        Ok(data) => {
                            let failed = crate::outcome::failed(&data);
                            if failed {
                                failures += 1;
                            }
                            results.push(json!({"index":i,"ok":!failed,"data":data}));
                            if failed && stop_on_error {
                                break;
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            results.push(json!({"index":i,"ok":false,"error":error_json(&e)}));
                            if stop_on_error {
                                break;
                            }
                        }
                    }
                }
                Ok(json!({"results":results,"failures":failures,"ok":failures==0}))
            }
            Request::ClearCache {} => {
                self.clear_cached_data();
                if let Some(jobs) = &self.jobs {
                    jobs.clear_cache();
                }
                Ok(json!({"cleared":true}))
            }
        }
    }
}
fn register_workfile(
    result: &mut Value,
    project: &Path,
    file: &Path,
    revision: u64,
    role: crate::workfiles::Role,
) {
    match crate::workfiles::register(project, file, revision, role) {
        Ok(registered) => result["workfile"] = registered,
        Err(error) => result["workfile_warning"] = json!(error.to_string()),
    }
}
fn ensure_png(path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
        "previews must use .png"
    );
    Ok(())
}
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    if path.exists() {
        return Ok(std::fs::canonicalize(path)?);
    }
    let mut parent = path.as_path();
    let mut suffix = Vec::new();
    while !parent.exists() {
        if let Some(name) = parent.file_name() {
            suffix.push(name.to_os_string());
        }
        parent = parent
            .parent()
            .ok_or_else(|| anyhow::anyhow!("cannot resolve output path"))?;
    }
    let mut out = std::fs::canonicalize(parent)?;
    for name in suffix.into_iter().rev() {
        out.push(name);
    }
    Ok(out)
}
pub(crate) fn ensure_distinct(a: &Path, b: &Path) -> Result<()> {
    ensure!(
        absolute(a)? != absolute(b)?,
        "output must differ from input"
    );
    Ok(())
}
pub(crate) fn protect_project(path: &Path, p: &project::Project, output: &Path) -> Result<()> {
    ensure_distinct(path, output)?;
    ensure_distinct(
        &project::resolve(project::base(path), &p.source.path),
        output,
    )?;
    let assets = std::path::absolute(path)?;
    let mut asset_name = assets.as_os_str().to_os_string();
    asset_name.push(".assets");
    ensure!(
        !absolute(output)?.starts_with(absolute(&PathBuf::from(asset_name))?),
        "cannot export into immutable project assets"
    );
    Ok(())
}
pub fn capabilities() -> Value {
    let mut value: Value = serde_json::from_str(include_str!("capabilities.json"))
        .expect("embedded capabilities must be valid JSON");
    value["version"] = json!(env!("CARGO_PKG_VERSION"));
    value["color_transforms"]["ocio_runtime"] = match tinge_ocio::version() {
        Ok(version) => json!({"available":true,"version":version,"stub":false}),
        Err(error) => json!({"available":false,"error":error.to_string()}),
    };
    value
}
