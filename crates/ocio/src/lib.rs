//! Real, statically linked OpenColorIO. Stub builds are rejected at runtime.
pub mod cdl;
mod grade;
use anyhow::{Context, Result, ensure};
pub use grade::{CdlStyle, NodeGrade};
use ocio_rs::{
    BuiltinConfigRegistry, Config, Context as OcioContext, EnvironmentMode, TransformDirection,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Versioned names make built-in configs reproducible across OCIO upgrades.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigSource {
    Builtin {
        name: String,
    },
    File {
        path: PathBuf,
    },
    /// Immutable OCIOZ package, verified before any cache or processor is used.
    Frozen {
        path: PathBuf,
        hash: String,
    },
}

/// Legacy string configs remain byte-for-byte compatible in revision hashes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PipelineConfig {
    Builtin(String),
    Source(ConfigSource),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transform {
    ColorSpace {
        source: String,
        destination: String,
    },
    DisplayView {
        source: String,
        display: String,
        view: String,
        #[serde(default)]
        inverse: bool,
    },
    /// Input and output use the same source encoding; grade owns its process space.
    Grade {
        source: String,
        grade: NodeGrade,
    },
}

pub const WORKING_SPACE: &str = "Linear Rec.709 (sRGB)";
pub const SRGB_DISPLAY: &str = "sRGB - Display";
pub const ENGINE_VERSION: &str = "2.5.2";

fn engine_version() -> String {
    ENGINE_VERSION.into()
}
fn working_space() -> String {
    WORKING_SPACE.into()
}
fn is_working_space(name: &str) -> bool {
    name == WORKING_SPACE
}
fn working_encoding() -> tinge_color::ColorSpace {
    tinge_color::ColorSpace {
        primaries: tinge_color::Primaries::Srgb,
        transfer: tinge_color::Transfer::Linear,
    }
}
fn is_working_encoding(space: &tinge_color::ColorSpace) -> bool {
    *space == working_encoding()
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTarget {
    #[default]
    Srgb,
    DisplayP3,
    Rec2100Pq,
}
fn is_srgb(target: &DisplayTarget) -> bool {
    *target == DisplayTarget::Srgb
}
impl DisplayTarget {
    pub fn name(self) -> &'static str {
        match self {
            Self::Srgb => SRGB_DISPLAY,
            Self::DisplayP3 => "Display P3 - Display",
            Self::Rec2100Pq => "Rec.2100-PQ - Display",
        }
    }
    pub fn color_space(self) -> tinge_color::ColorSpace {
        use tinge_color::{ColorSpace, Primaries, Transfer};
        match self {
            Self::Srgb => ColorSpace::default(),
            Self::DisplayP3 => ColorSpace {
                primaries: Primaries::DisplayP3,
                transfer: Transfer::Srgb,
            },
            Self::Rec2100Pq => ColorSpace {
                primaries: Primaries::Rec2020,
                transfer: Transfer::Pq,
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEncoding {
    /// Existing ICC / RAW / explicitly declared input colorimetry to working RGB.
    #[default]
    Managed,
    /// Stored rendered RGB values. ICC and transfer decoding are bypassed.
    Encoded { color_space: String },
}

/// An input/display chain around scene-linear grading.
/// Display outputs are tagged with the corresponding file colorimetry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    pub config: PipelineConfig,
    #[serde(default = "engine_version")]
    pub engine_version: String,
    #[serde(default)]
    pub input: InputEncoding,
    pub view: String,
    #[serde(default, skip_serializing_if = "is_srgb")]
    pub display: DisplayTarget,
    /// Required for non-sRGB displays; agent previews remain SDR sRGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_view: Option<String>,
    /// Explicit overrides; absent values use authored defaults, never ambient env.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
    /// OCIO color-space name corresponding to the declared linear encoding.
    #[serde(default = "working_space", skip_serializing_if = "is_working_space")]
    pub working_space: String,
    #[serde(
        default = "working_encoding",
        skip_serializing_if = "is_working_encoding"
    )]
    pub working_encoding: tinge_color::ColorSpace,
    /// Custom config display names; encoding remains owned by `display`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_display_name: Option<String>,
}

impl Pipeline {
    pub fn compile_grade(&self, grade: &NodeGrade) -> Result<CompiledTransform> {
        ensure!(
            self.engine_version == version()?,
            "OCIO engine version mismatch"
        );
        ensure!(
            self.working_encoding.transfer == tinge_color::Transfer::Linear,
            "OCIO working_encoding must be linear"
        );
        compile_with_context(
            &self.source(),
            &Transform::Grade {
                source: self.working_space.clone(),
                grade: grade.clone(),
            },
            &self.context,
        )
    }
    /// Canonical Frame -> declared linear anchor -> native grade -> canonical Frame.
    pub fn apply_grade(
        &self,
        processor: &CompiledTransform,
        pixels: &mut [[f32; 4]],
    ) -> Result<TransformReport> {
        convert_primaries(
            pixels,
            tinge_color::Primaries::Srgb,
            self.working_encoding.primaries,
        );
        let report = processor.apply(pixels)?;
        convert_primaries(
            pixels,
            self.working_encoding.primaries,
            tinge_color::Primaries::Srgb,
        );
        ensure!(
            pixels.iter().flatten().all(|v| v.is_finite()),
            "OCIO grade produced non-finite canonical RGB"
        );
        Ok(report)
    }
    fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(self.display.name())
    }
    fn preview_display_name(&self) -> &str {
        self.preview_display_name.as_deref().unwrap_or_else(|| {
            if self.display == DisplayTarget::Srgb {
                self.display_name()
            } else {
                SRGB_DISPLAY
            }
        })
    }
    pub fn source(&self) -> ConfigSource {
        match &self.config {
            PipelineConfig::Builtin(name) => ConfigSource::Builtin { name: name.clone() },
            PipelineConfig::Source(source) => source.clone(),
        }
    }
    /// Resolve external package/config paths without mutating stored project data.
    pub fn resolved_at(&self, base: &Path) -> Self {
        let mut pipeline = self.clone();
        if let PipelineConfig::Source(
            ConfigSource::File { path } | ConfigSource::Frozen { path, .. },
        ) = &mut pipeline.config
            && !path.is_absolute()
        {
            *path = base.join(&*path);
        }
        pipeline
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.engine_version == version()?,
            "OCIO engine version mismatch: pipeline requires {}, runtime is {}",
            self.engine_version,
            version()?
        );
        let info = inspect_with_context(&self.source(), &self.context)?;
        ensure!(
            self.working_encoding.transfer == tinge_color::Transfer::Linear,
            "OCIO working_encoding must be linear; grading Frame remains scene-linear sRGB"
        );
        ensure!(
            info.color_spaces.iter().any(|s| s == &self.working_space),
            "config lacks declared working space"
        );
        let display = info
            .displays
            .iter()
            .find(|d| d.name == self.display_name())
            .context("config lacks requested display")?;
        ensure!(
            self.view != "Raw" && display.views.contains(&self.view),
            "view must be a color-managed view for the requested display; Raw is untagged data"
        );
        if self.display != DisplayTarget::Srgb {
            ensure!(
                self.preview_view.is_some(),
                "non-sRGB display requires an explicit preview_view for SDR sRGB agent previews"
            );
        }
        if let Some(view) = &self.preview_view {
            let srgb = info
                .displays
                .iter()
                .find(|d| d.name == self.preview_display_name())
                .context("config lacks sRGB preview display")?;
            ensure!(
                view != "Raw" && srgb.views.contains(view),
                "preview_view must be a color-managed sRGB view"
            );
        }
        if let InputEncoding::Encoded { color_space } = &self.input {
            ensure!(
                info.color_spaces.contains(color_space),
                "input color space is absent from config"
            );
            ensure!(color_space != "Raw", "Raw is not a defined RGB encoding");
        }
        // Resolve the actual selected dependencies now, before committing a
        // project revision; config.validate alone does not compile LUTs/views.
        let config = load(&self.source())?;
        let (context, _, _) = explicit_context(&config, &self.context)?;
        config.processor_display_with_context(
            &self.working_space,
            self.display_name(),
            &self.view,
            TransformDirection::Forward,
            &context,
        )?;
        if let Some(view) = &self.preview_view {
            config.processor_display_with_context(
                &self.working_space,
                self.preview_display_name(),
                view,
                TransformDirection::Forward,
                &context,
            )?;
        }
        if let InputEncoding::Encoded { color_space } = &self.input {
            config.processor_with_context(color_space, &self.working_space, &context)?;
        }
        Ok(())
    }
    pub fn to_working(&self, pixels: &mut [[f32; 4]]) -> Result<Option<TransformReport>> {
        self.validate()?;
        match &self.input {
            InputEncoding::Managed => Ok(None),
            InputEncoding::Encoded { color_space } => {
                let report = apply_with_context(
                    &self.source(),
                    &Transform::ColorSpace {
                        source: color_space.clone(),
                        destination: self.working_space.clone(),
                    },
                    pixels,
                    &self.context,
                )?;
                convert_primaries(
                    pixels,
                    self.working_encoding.primaries,
                    tinge_color::Primaries::Srgb,
                );
                Ok(Some(report))
            }
        }
    }
    pub fn to_display(&self, pixels: &mut [[f32; 4]]) -> Result<TransformReport> {
        self.validate()?;
        convert_primaries(
            pixels,
            tinge_color::Primaries::Srgb,
            self.working_encoding.primaries,
        );
        apply_with_context(
            &self.source(),
            &Transform::DisplayView {
                source: self.working_space.clone(),
                display: self.display_name().into(),
                view: self.view.clone(),
                inverse: false,
            },
            pixels,
            &self.context,
        )
    }
    pub fn for_preview(&self) -> Result<Self> {
        self.validate()?;
        let mut pipeline = self.clone();
        pipeline.display_name = Some(self.preview_display_name().into());
        pipeline.display = DisplayTarget::Srgb;
        pipeline.view = self
            .preview_view
            .clone()
            .unwrap_or_else(|| self.view.clone());
        Ok(pipeline)
    }
}

fn convert_primaries(
    pixels: &mut [[f32; 4]],
    from: tinge_color::Primaries,
    to: tinge_color::Primaries,
) {
    for pixel in pixels {
        let rgb = tinge_color::convert_linear([pixel[0], pixel[1], pixel[2]], from, to);
        pixel[..3].copy_from_slice(&rgb);
    }
}

#[derive(Debug, Serialize)]
pub struct BuiltinConfig {
    pub name: String,
    pub label: String,
    pub recommended: bool,
}

#[derive(Debug, Serialize)]
pub struct Display {
    pub name: String,
    pub views: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfigInfo {
    pub engine_version: String,
    pub name: Option<String>,
    pub cache_id: Option<String>,
    pub color_spaces: Vec<String>,
    pub displays: Vec<Display>,
    pub context_defaults: BTreeMap<String, String>,
    pub resolved_context: BTreeMap<String, String>,
    pub looks: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransformReport {
    pub engine_version: String,
    pub config_cache_id: Option<String>,
    pub processor_cache_id: Option<String>,
    pub pixel_count: usize,
    pub context: BTreeMap<String, String>,
    pub files: Vec<String>,
    pub looks: Vec<String>,
}

pub fn version() -> Result<String> {
    ensure!(!ocio_rs::is_stub_build(), "OCIO stub is not a color engine");
    Ok(ocio_rs::version().unwrap_or_default())
}

pub fn builtin_configs() -> Result<Vec<BuiltinConfig>> {
    version()?;
    let registry = BuiltinConfigRegistry::get()?;
    (0..registry.num_builtin_configs())
        .map(|i| {
            Ok(BuiltinConfig {
                name: registry
                    .try_config_name(i)?
                    .context("missing builtin config name")?,
                label: registry.try_config_ui_name(i)?.unwrap_or_default(),
                recommended: registry.try_is_config_recommended(i)?,
            })
        })
        .collect()
}

fn load(source: &ConfigSource) -> Result<Config> {
    version()?;
    let config = match source {
        ConfigSource::Builtin { name } => {
            ensure!(
                builtin_configs()?.iter().any(|c| c.name == *name),
                "use an exact versioned builtin config name from ocio-configs; aliases are not reproducible"
            );
            Config::create_from_builtin_config(name)?
        }
        ConfigSource::File { path } | ConfigSource::Frozen { path, .. } => {
            if let ConfigSource::Frozen { hash, .. } = source {
                ensure!(
                    path.extension()
                        .is_some_and(|v| v.eq_ignore_ascii_case("ocioz")),
                    "frozen OCIO config must be an OCIOZ package"
                );
                ensure!(
                    hash.len() == 64 && file_hash(path)? == *hash,
                    "immutable OCIO package hash mismatch: {}",
                    path.display()
                );
            }
            // External configs/LUTs are editable. Observe their current bytes
            // on each synchronous request instead of OCIO's global file cache.
            ocio_rs::try_clear_all_caches()?;
            Config::from_file(path.to_str().context("OCIO config path must be UTF-8")?)?
        }
    };
    config.validate()?;
    Ok(config)
}

pub fn inspect(source: &ConfigSource) -> Result<ConfigInfo> {
    inspect_with_context(source, &BTreeMap::new())
}
pub fn validate_color_space(source: &ConfigSource, name: &str) -> Result<()> {
    let config = load(source)?;
    let space = config
        .try_color_space(name)?
        .context("OCIO color space is absent")?;
    ensure!(
        !space.is_data(),
        "OCIO encoding must be a color space, not data"
    );
    Ok(())
}

pub fn inspect_with_context(
    source: &ConfigSource,
    overrides: &BTreeMap<String, String>,
) -> Result<ConfigInfo> {
    let config = load(source)?;
    let (context, context_defaults, resolved_context) = explicit_context(&config, overrides)?;
    let color_spaces = (0..config.num_color_spaces())
        .map(|i| {
            config
                .try_color_space_name_by_index(i)?
                .context("missing color space")
        })
        .collect::<Result<Vec<_>>>()?;
    let displays = (0..config.num_displays())
        .map(|i| {
            let name = config.try_display(i)?.context("missing display")?;
            let views = (0..config.try_num_views(&name)?)
                .map(|j| config.try_view(&name, j)?.context("missing view"))
                .collect::<Result<Vec<_>>>()?;
            Ok(Display { name, views })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ConfigInfo {
        engine_version: version()?,
        name: config.try_name()?,
        cache_id: config.try_cache_id_for_context(&context)?,
        color_spaces,
        displays,
        context_defaults,
        resolved_context,
        looks: (0..config.num_looks())
            .map(|i| config.try_look_name_by_index(i)?.context("missing look"))
            .collect::<Result<_>>()?,
    })
}

/// RGB is transformed in-place; straight alpha is preserved exactly.
/// Caller owns the signal domain: the output is NOT necessarily scene-linear.
pub fn apply(
    source: &ConfigSource,
    transform: &Transform,
    pixels: &mut [[f32; 4]],
) -> Result<TransformReport> {
    apply_with_context(source, transform, pixels, &BTreeMap::new())
}

pub fn apply_with_context(
    source: &ConfigSource,
    transform: &Transform,
    pixels: &mut [[f32; 4]],
    overrides: &BTreeMap<String, String>,
) -> Result<TransformReport> {
    compile_with_context(source, transform, overrides)?.apply(pixels)
}

/// Compiled once per request, including dependency/cache identity for graph caching.
pub struct CompiledTransform {
    cpu: ocio_rs::CPUProcessor,
    report: TransformReport,
}
impl CompiledTransform {
    pub fn identity(&self) -> &TransformReport {
        &self.report
    }
    pub fn apply(&self, pixels: &mut [[f32; 4]]) -> Result<TransformReport> {
        ensure!(
            pixels.iter().flatten().all(|v| v.is_finite()),
            "OCIO input pixels must be finite"
        );
        for chunk in pixels.chunks_mut(16_384) {
            let mut packed: Vec<f32> = chunk.iter().flatten().copied().collect();
            self.cpu
                .try_apply_rgba_pixels(&mut packed, chunk.len() as i64, 4)?;
            for (pixel, result) in chunk.iter_mut().zip(packed.chunks_exact(4)) {
                ensure!(
                    result[..3].iter().all(|v| v.is_finite()),
                    "OCIO produced a non-finite RGB value"
                );
                pixel[..3].copy_from_slice(&result[..3]);
            }
        }
        let mut report = self.report.clone();
        report.pixel_count = pixels.len();
        Ok(report)
    }
}

pub fn compile_with_context(
    source: &ConfigSource,
    transform: &Transform,
    overrides: &BTreeMap<String, String>,
) -> Result<CompiledTransform> {
    let config = load(source)?;
    let (context, _, resolved_context) = explicit_context(&config, overrides)?;
    let processor = match transform {
        Transform::Grade { source, grade } => grade.processor(&config, &context, source)?,
        Transform::ColorSpace {
            source,
            destination,
        } => config.processor_with_context(source, destination, &context)?,
        Transform::DisplayView {
            source,
            display,
            view,
            inverse,
        } => config.processor_display_with_context(
            source,
            display,
            view,
            if *inverse {
                TransformDirection::Inverse
            } else {
                TransformDirection::Forward
            },
            &context,
        )?,
    };
    let metadata = processor.try_processor_metadata()?;
    let files = (0..metadata.num_files())
        .map(|i| metadata.file(i).context("missing processor file metadata"))
        .collect::<Result<_>>()?;
    let cpu = processor.default_cpu_processor()?;
    let looks = (0..metadata.num_looks())
        .map(|i| metadata.look(i).context("missing processor look metadata"))
        .collect::<Result<_>>()?;
    Ok(CompiledTransform {
        cpu,
        report: TransformReport {
            engine_version: version()?,
            config_cache_id: config.try_cache_id_for_context(&context)?,
            processor_cache_id: processor.try_cache_id()?,
            pixel_count: 0,
            context: resolved_context,
            files,
            looks,
        },
    })
}

fn file_hash(path: &Path) -> Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    use std::io::Read;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

type Variables = BTreeMap<String, String>;
fn explicit_context(
    config: &Config,
    overrides: &Variables,
) -> Result<(OcioContext, Variables, Variables)> {
    let current = config
        .try_current_context()?
        .context("config has no context")?;
    let context = OcioContext::create()?;
    context.set_environment_mode(EnvironmentMode::LoadPredefined)?;
    if let Some(dir) = current.try_working_dir()? {
        context.set_working_dir(dir)?;
    }
    for i in 0..current.try_num_search_paths()? {
        context.add_search_path(
            current
                .try_search_path_by_index(i)?
                .context("missing search path")?,
        )?;
    }
    if let Some(proxy) = current.try_config_io_proxy_object()? {
        context.set_config_io_proxy_object(&proxy)?;
    }
    let mut defaults = BTreeMap::new();
    for i in 0..config.num_environment_vars() {
        let name = config
            .environment_var_name_by_index(i)
            .context("missing context variable name")?;
        let value = config
            .environment_var_default(&name)
            .context("missing context variable default")?;
        defaults.insert(name, value);
    }
    let mut variables = defaults.clone();
    variables.extend(overrides.clone());
    for (name, value) in &variables {
        ensure!(
            !name.is_empty() && !name.contains(['$', '{', '}', '\0']),
            "invalid OCIO context variable name"
        );
        context.set_string_var(name, value)?;
    }
    Ok((context, defaults, variables))
}

/// Use OCIO's native binary package format, retaining all supported LUT files
/// under its working directory so future context/view changes remain usable.
pub fn archive_file(path: &Path) -> Result<Vec<u8>> {
    let config = load(&ConfigSource::File { path: path.into() })?;
    if path
        .extension()
        .is_some_and(|v| v.eq_ignore_ascii_case("ocioz"))
    {
        return Ok(std::fs::read(path)?);
    }
    ensure!(
        config.is_archivable(),
        "OCIO config cannot be archived: LUT sources and search paths must remain within its working directory; external dependency relocation is not implemented yet"
    );
    validate_archive_tree(&config)?;
    let bytes = config
        .archive_bytes()?
        .context("OCIO archive returned no binary data")?;
    ensure!(
        bytes.starts_with(b"PK\x03\x04"),
        "OCIO archive is not a complete ZIP package"
    );
    Ok(bytes)
}

fn validate_archive_tree(config: &Config) -> Result<()> {
    // The native archiver recurses through directories. Refuse links/junctions
    // and bound the import before entering that unbounded native traversal.
    let root = config
        .working_dir()
        .context("archive working directory is missing")?;
    let mut pending = vec![PathBuf::from(root)];
    let mut entries = 0usize;
    let mut bytes = 0u64;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            entries += 1;
            ensure!(
                entries <= 100_000,
                "OCIO archive directory exceeds 100000 entries"
            );
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "OCIO archive directory contains a link/junction: {}",
                entry.path().display()
            );
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|v| v.to_str())
                    .is_some_and(ocio_rs::transform::FileTransform::is_format_extension_supported)
            {
                bytes = bytes
                    .checked_add(entry.metadata()?.len())
                    .context("archive byte count overflow")?;
                ensure!(
                    bytes <= 512 * 1024 * 1024,
                    "OCIO archive LUT payload exceeds 512 MiB"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_engine_is_real() {
        assert!(super::version().unwrap().starts_with("2.5."));
    }
}
