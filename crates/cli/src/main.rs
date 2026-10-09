#[cfg(test)]
mod agent_tests;
mod api;
mod jobs;
mod outcome;
mod protocol;
mod tools;
mod viewers;
mod web;
mod workfiles;
use anyhow::{Context, Result};
use api::{Request, SchemaKind, Session};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};
use tinge_color::ColorSpace;
use tinge_core::Recipe;
use tinge_project::Edit;

#[derive(Parser)]
#[command(
    name = "tinge",
    version,
    about = "Agent-native, non-destructive float32 image grading. JSON on stdout."
)]
struct Cli {
    /// Emit node progress as JSON lines to stderr.
    #[arg(long, global = true)]
    progress: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Open the local image viewer. Agent grading remains in CLI / MCP.
    View {
        project: PathBuf,
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Print the local URL without opening the system browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Read saved viewer annotations with revision and output-coordinate basis.
    Selections {
        project: PathBuf,
    },
    /// Weight-free cutout: options JSON selects color key, GrabCut, mask, or trimap.
    Cutout {
        input: PathBuf,
        #[arg(long)]
        options: PathBuf,
        /// Optional 16-bit grayscale PNG with unencoded alpha values.
        #[arg(long)]
        matte: Option<PathBuf>,
        #[arg(long)]
        input_space: Option<String>,
        #[arg(long)]
        color_pipeline: Option<PathBuf>,
        #[arg(long)]
        raw_develop: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
    },
    CdlInspect {
        input: PathBuf,
    },
    CdlImport {
        input: PathBuf,
        #[arg(long, conflicts_with = "index")]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        index: Option<u32>,
        #[arg(long)]
        color_space: String,
        #[arg(long, value_enum)]
        style: CdlStyleArg,
        #[arg(long)]
        inverse: bool,
    },
    CdlExport {
        #[arg(long, conflicts_with = "project", required_unless_present = "project")]
        document: Option<PathBuf>,
        #[arg(
            long,
            conflicts_with = "document",
            required_unless_present = "document"
        )]
        project: Option<PathBuf>,
        #[arg(long, requires = "project")]
        revision: Option<u64>,
        #[arg(
            long,
            requires = "project",
            required_unless_present = "document",
            value_delimiter = ','
        )]
        nodes: Vec<String>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    LutInspect {
        input: PathBuf,
    },
    LutBake {
        #[arg(long, conflicts_with = "project", required_unless_present = "project")]
        recipe: Option<PathBuf>,
        #[arg(long, conflicts_with = "recipe", required_unless_present = "recipe")]
        project: Option<PathBuf>,
        #[arg(long, requires = "project")]
        revision: Option<u64>,
        #[arg(long, requires = "recipe")]
        color_pipeline: Option<PathBuf>,
        /// JSON BakeOptions; default sRGB RGB in/out, 33-cube, held-out QA.
        #[arg(long)]
        options: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    Capabilities,
    OcioConfigs,
    OcioInspect {
        /// Exact builtin name from ocio-configs, or a config.ocio file path.
        #[arg(
            long,
            conflicts_with = "config_file",
            required_unless_present = "config_file"
        )]
        builtin: Option<String>,
        #[arg(long, conflicts_with = "builtin", required_unless_present = "builtin")]
        config_file: Option<PathBuf>,
        /// Explicit OCIO context overrides as a JSON object.
        #[arg(long)]
        context: Option<String>,
    },
    Schema {
        #[arg(value_enum, default_value = "request")]
        kind: SchemaArg,
    },
    Inspect {
        input: PathBuf,
    },
    RawPlan {
        input: PathBuf,
        #[arg(long)]
        raw_develop: Option<PathBuf>,
    },
    Init {
        input: PathBuf,
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        input_space: Option<String>,
        #[arg(long)]
        color_pipeline: Option<PathBuf>,
        #[arg(long)]
        raw_develop: Option<PathBuf>,
    },
    Show {
        project: PathBuf,
    },
    Apply {
        project: PathBuf,
        #[arg(long)]
        expect_revision: u64,
        /// JSON array of edits, or - for stdin.
        #[arg(long)]
        edits: PathBuf,
        #[arg(long, default_value = "agent edit")]
        label: String,
    },
    Restore {
        project: PathBuf,
        revision: u64,
        #[arg(long)]
        expect_revision: u64,
    },
    Branch {
        project: PathBuf,
        name: String,
        #[arg(long)]
        expect_revision: u64,
        #[arg(long)]
        checkout: bool,
    },
    Tag {
        project: PathBuf,
        name: String,
        #[arg(long)]
        expect_revision: u64,
    },
    Validate {
        recipe: PathBuf,
        #[arg(long)]
        color_pipeline: Option<PathBuf>,
    },
    Grade {
        input: PathBuf,
        #[arg(long)]
        recipe: PathBuf,
        #[arg(long)]
        input_space: Option<String>,
        #[arg(long)]
        color_pipeline: Option<PathBuf>,
        #[arg(long)]
        raw_develop: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
    },
    Render {
        project: PathBuf,
        #[arg(long)]
        revision: Option<u64>,
        #[arg(long)]
        max_edge: Option<u32>,
        /// Mark this export as a disposable draft; finalization preserves the selected revision.
        #[arg(long)]
        temporary: bool,
        #[command(flatten)]
        output: Output,
    },
    Analyze {
        input: PathBuf,
        #[arg(long)]
        input_space: Option<String>,
        #[arg(long)]
        color_pipeline: Option<PathBuf>,
        #[arg(long)]
        raw_develop: Option<PathBuf>,
        #[arg(long)]
        recipe: Option<PathBuf>,
    },
    Stats {
        project: PathBuf,
        #[arg(long)]
        revision: Option<u64>,
        #[arg(long)]
        scopes: bool,
    },
    Preview {
        project: PathBuf,
        #[arg(long)]
        revision: Option<u64>,
        #[arg(long, default_value_t = 1600)]
        max_edge: u32,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    Compare {
        project: PathBuf,
        #[arg(long)]
        revision: Option<u64>,
        #[arg(long, default_value_t = 1200)]
        max_edge: u32,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Execute a JSON request. Use - to read stdin.
    Run {
        request: PathBuf,
    },
    /// Execute a JSON array of requests, continuing after individual failures.
    Batch {
        manifest: PathBuf,
        #[arg(long)]
        stop_on_error: bool,
    },
    /// Persistent newline-delimited JSON requests/responses, preserving engine cache.
    Serve,
    /// Model Context Protocol server on stdin/stdout.
    Mcp,
}
#[derive(Args)]
struct Output {
    #[arg(short, long)]
    output: PathBuf,
    /// Default: JPEG 8, EXR 32, PNG/TIFF 16.
    #[arg(long)]
    bit_depth: Option<u8>,
    /// JSON primaries/transfer. Defaults to the selected display, or sRGB/linear sRGB.
    #[arg(long)]
    output_space: Option<String>,
    /// Luminance of linear RGB 1.0 for manual PQ encoding; omit with OCIO displays.
    #[arg(long)]
    linear_unit_nits: Option<f32>,
    #[arg(long)]
    overwrite: bool,
}
#[derive(Clone, Copy, ValueEnum)]
enum SchemaArg {
    Request,
    Recipe,
    Edits,
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = if path == Path::new("-") {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        bytes
    } else {
        std::fs::read(path).with_context(|| format!("read {}", path.display()))?
    };
    serde_json::from_slice(&bytes).context("invalid JSON or unknown field")
}
fn origin(path: &Path) -> Result<PathBuf> {
    if path == Path::new("-") {
        Ok(std::env::current_dir()?)
    } else {
        Ok(std::fs::canonicalize(path)?
            .parent()
            .context("missing parent")?
            .into())
    }
}
fn space(text: Option<String>) -> Result<Option<ColorSpace>> {
    text.map(|s| {
        serde_json::from_str(&s).context(
            "input-space must be JSON, e.g. {\"primaries\":\"srgb\",\"transfer\":\"srgb\"}",
        )
    })
    .transpose()
}
fn read_pipeline(path: &Path) -> Result<tinge_ocio::Pipeline> {
    let pipeline: tinge_ocio::Pipeline = read_json(path)?;
    Ok(pipeline.resolved_at(&origin(path)?))
}
#[derive(Clone, Copy, ValueEnum)]
enum CdlStyleArg {
    Asc,
    NoClamp,
}
fn command_request(cmd: Command) -> Result<Request> {
    Ok(match cmd {
        Command::CdlInspect { input } => Request::CdlInspect { input },
        Command::CdlImport {
            input,
            id,
            index,
            color_space,
            style,
            inverse,
        } => Request::CdlImport {
            input,
            selector: id
                .map(|id| tinge_ocio::cdl::CdlSelector::Id { id })
                .or_else(|| index.map(|index| tinge_ocio::cdl::CdlSelector::Index { index })),
            color_space,
            style: match style {
                CdlStyleArg::Asc => tinge_ocio::CdlStyle::Asc,
                CdlStyleArg::NoClamp => tinge_ocio::CdlStyle::NoClamp,
            },
            inverse,
        },
        Command::CdlExport {
            document,
            project,
            revision,
            nodes,
            output,
            overwrite,
        } => Request::CdlExport {
            source: if let Some(document) = document {
                api::ensure_distinct(&document, &output)?;
                api::CdlExportSource::Document {
                    document: Box::new(read_json(&document)?),
                }
            } else {
                api::CdlExportSource::Project {
                    project: project.context("missing project")?,
                    revision,
                    nodes,
                }
            },
            output,
            overwrite,
        },
        Command::LutInspect { input } => Request::LutInspect { input },
        Command::LutBake {
            recipe,
            project,
            revision,
            color_pipeline,
            options,
            output,
            overwrite,
        } => {
            let (source, asset_base) = if let Some(recipe) = recipe {
                (
                    api::LutBakeSource::Recipe {
                        recipe: read_json(&recipe)?,
                        color_pipeline: color_pipeline
                            .as_deref()
                            .map(read_pipeline)
                            .transpose()?
                            .map(Box::new),
                    },
                    Some(origin(&recipe)?),
                )
            } else {
                (
                    api::LutBakeSource::Project {
                        project: project.context("missing project")?,
                        revision,
                    },
                    None,
                )
            };
            Request::LutBake {
                source,
                asset_base,
                output,
                overwrite,
                options: options
                    .as_deref()
                    .map(read_json)
                    .transpose()?
                    .unwrap_or_default(),
            }
        }
        Command::Cutout {
            input,
            options,
            matte,
            input_space,
            color_pipeline,
            raw_develop,
            output,
        } => {
            api::ensure_distinct(&options, &output.output)?;
            if let Some(matte) = &matte {
                api::ensure_distinct(&options, matte)?;
            }
            Request::Cutout {
                input,
                options: read_json(&options)?,
                matte,
                asset_base: Some(origin(&options)?),
                input_space: space(input_space)?,
                color_pipeline: color_pipeline.as_deref().map(read_pipeline).transpose()?,
                raw_develop: raw_develop.as_deref().map(read_json).transpose()?,
                output: output.output,
                bit_depth: output.bit_depth,
                output_space: space(output.output_space)?,
                linear_unit_nits: output.linear_unit_nits,
                overwrite: output.overwrite,
            }
        }
        Command::Capabilities => Request::Capabilities {},
        Command::OcioConfigs => Request::OcioConfigs {},
        Command::OcioInspect {
            builtin,
            config_file,
            context,
        } => Request::OcioInspect {
            config: if let Some(name) = builtin {
                tinge_ocio::ConfigSource::Builtin { name }
            } else {
                tinge_ocio::ConfigSource::File {
                    path: config_file.context("missing OCIO config")?,
                }
            },
            context: context
                .map(|text| serde_json::from_str(&text))
                .transpose()?
                .unwrap_or_default(),
        },
        Command::Schema { kind } => Request::Schema {
            target: None,
            kind: match kind {
                SchemaArg::Request => SchemaKind::Request,
                SchemaArg::Recipe => SchemaKind::Recipe,
                SchemaArg::Edits => SchemaKind::Edits,
            },
        },
        Command::Inspect { input } => Request::Inspect { input },
        Command::RawPlan { input, raw_develop } => Request::RawPlan {
            input,
            options: raw_develop
                .as_deref()
                .map(read_json)
                .transpose()?
                .unwrap_or_default(),
            sensor_points: Vec::new(),
        },
        Command::Init {
            input,
            project,
            input_space,
            color_pipeline,
            raw_develop,
        } => Request::Init {
            input,
            project,
            input_space: space(input_space)?,
            color_pipeline: color_pipeline.map(|p| read_pipeline(&p)).transpose()?,
            raw_develop: raw_develop.as_deref().map(read_json).transpose()?,
        },
        Command::Show { project } => Request::Show { project },
        Command::Apply {
            project,
            expect_revision,
            edits,
            label,
        } => {
            let ops: Vec<Edit> = read_json(&edits)?;
            Request::Apply {
                project,
                expect_revision,
                edits: ops,
                label,
                asset_base: Some(origin(&edits)?),
            }
        }
        Command::Restore {
            project,
            revision,
            expect_revision,
        } => Request::Restore {
            project,
            revision,
            expect_revision,
        },
        Command::Branch {
            project,
            name,
            expect_revision,
            checkout,
        } => Request::Branch {
            project,
            name,
            expect_revision,
            checkout,
        },
        Command::Tag {
            project,
            name,
            expect_revision,
        } => Request::Tag {
            project,
            name,
            expect_revision,
        },
        Command::Validate {
            recipe,
            color_pipeline,
        } => Request::Validate {
            recipe: read_json(&recipe)?,
            color_pipeline: color_pipeline.as_deref().map(read_pipeline).transpose()?,
        },
        Command::Grade {
            input,
            recipe,
            input_space,
            color_pipeline,
            raw_develop,
            output,
        } => Request::Grade {
            input,
            recipe: read_json(&recipe)?,
            input_space: space(input_space)?,
            color_pipeline: color_pipeline.map(|p| read_pipeline(&p)).transpose()?,
            raw_develop: raw_develop.as_deref().map(read_json).transpose()?,
            output: output.output,
            bit_depth: output.bit_depth,
            output_space: space(output.output_space)?,
            linear_unit_nits: output.linear_unit_nits,
            overwrite: output.overwrite,
            asset_base: Some(origin(&recipe)?),
        },
        Command::Render {
            project,
            revision,
            max_edge,
            temporary,
            output,
        } => Request::Render {
            project,
            revision,
            max_edge,
            temporary,
            output: output.output,
            bit_depth: output.bit_depth,
            output_space: space(output.output_space)?,
            linear_unit_nits: output.linear_unit_nits,
            overwrite: output.overwrite,
        },
        Command::Analyze {
            input,
            input_space,
            color_pipeline,
            raw_develop,
            recipe,
        } => {
            let asset_base = recipe.as_ref().map(|p| origin(p)).transpose()?;
            let recipe = recipe
                .as_ref()
                .map(|p| read_json::<Recipe>(p))
                .transpose()?;
            Request::Analyze {
                input,
                input_space: space(input_space)?,
                color_pipeline: color_pipeline.map(|p| read_pipeline(&p)).transpose()?,
                raw_develop: raw_develop.as_deref().map(read_json).transpose()?,
                recipe,
                asset_base,
            }
        }
        Command::Stats {
            project,
            revision,
            scopes,
        } => Request::Stats {
            project,
            revision,
            scopes,
        },
        Command::Preview {
            project,
            revision,
            max_edge,
            output,
            overwrite,
        } => Request::Preview {
            include_analysis: true,
            project,
            revision,
            max_edge,
            output: Some(output),
            overwrite,
        },
        Command::Compare {
            project,
            revision,
            max_edge,
            output,
            overwrite,
        } => Request::Compare {
            project,
            revision,
            max_edge,
            output: Some(output),
            overwrite,
        },
        Command::Run { request } => read_json(&request)?,
        Command::Batch {
            manifest,
            stop_on_error,
        } => Request::Batch {
            jobs: read_json(&manifest)?,
            stop_on_error,
        },
        Command::Selections { project } => Request::Selections { project },
        Command::Serve | Command::Mcp | Command::View { .. } => unreachable!(),
    })
}
fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                e.exit();
            }
            eprintln!(
                "{}",
                json!({"ok":false,"error":{"code":"arguments","message":e.to_string()}})
            );
            std::process::exit(2)
        }
    };
    let mut session = Session {
        progress: cli.progress,
        ..Default::default()
    };
    let result: Result<Option<Value>> = match cli.command {
        Command::View {
            project,
            port,
            no_open,
        } => web::serve(project, port, !no_open).map(|_| None),
        Command::Serve => protocol::jsonl(&mut session).map(|_| None),
        Command::Mcp => protocol::mcp(&mut session).map(|_| None),
        cmd => command_request(cmd).and_then(|r| session.run(r)).map(Some),
    };
    match result {
        Ok(Some(data)) => {
            let failed = outcome::failed(&data);
            println!("{}", json!({"ok":!failed,"data":data}));
            if failed {
                std::process::exit(1);
            }
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("{}", json!({"ok":false,"error":api::error_json(&e)}));
            std::process::exit(1);
        }
    }
}
