//! Reviewed, static MCP operation surface. CLI/JSONL dispatch remains in api.
use crate::{api::Request, protocol};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::sync::OnceLock;

struct Tool {
    command: &'static str,
    description: &'static str,
    read_only: bool,
    destructive: bool,
    open_world: bool,
}

// Arbitrary local paths are open-ended entities: this server does not enforce
// a workspace allowlist. Session-only controls are bounded, hence closed-world.
const TOOLS: &[Tool] = &[
    Tool {
        command: "adjust",
        description: "Basic photo grading in one commit and preview: exposure, contrast, saturation, vibrance, white balance and tones. Supply at least one non-null control. Absolute values; omitted/null controls stay unchanged. New adjustment_id appends a WB/primary/tone group; reuse it to update without stacking, keeping downstream edits. Requires observed revision. Preview failure may follow a commit: inspect committed/revision before retrying. Advanced graphs/masks use edit_preview.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "capabilities",
        description: "Get implemented image-processing capabilities and limitations. Does not change files or start work.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "project_info",
        description: "Read a local Tinge project's current or selected revision, optional recipe and paginated history. Does not modify the project.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "show",
        description: "Read the complete local project including all historical recipes. Use for explicit full-history inspection; project_info provides smaller paginated summaries.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "inspect",
        description: "Inspect a local image's dimensions, format and color metadata without exporting or editing it.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "raw_plan",
        description: "Inspect the effective RAW development settings and optional sensor samples for a local image. Camera support depends on rawler; does not create a project or output.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "init",
        description: "Create a new non-destructive local image project and freeze source and color assets. Writes a project and asset directory; rejects an existing project. Re-read project state before retrying an uncertain result.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "edit_preview",
        description: "Commit revision-checked image edits and save a PNG preview of that exact revision. Writes project/assets and preview; overwrite requires explicit true. Preview failure may follow a successful commit: inspect committed/revision before retrying. Use submit_edit_preview for background execution with a session idempotency key.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "apply",
        description: "Commit edits to a local project without generating a preview. Requires expect_revision; changes the active recipe and preserves history. Inspect project state before retrying an uncertain commit.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "restore",
        description: "Restore a historical recipe as a new revision of a local project. Requires expect_revision, changes the active recipe and preserves history. Does not undo filesystem exports or job cancellation.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "branch",
        description: "Create a named branch in a revision-checked local project; checkout optionally changes the active branch. Writes project metadata; existing branch references are retained unless checkout selects that branch.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "tag",
        description: "Add a new named tag on the current revision of a local project. Requires expect_revision and creates a metadata revision while preserving the recipe/history; existing tag names are rejected.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "preview",
        description: "Render and persist an SDR sRGB PNG preview of a local project revision. Omit output to reuse a managed cache path, which may replace an earlier preview. Explicit paths reject existing files unless overwrite=true. Analysis is opt-in; returns a preview resource.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "compare",
        description: "Save a PNG comparison of the original and selected project revision. Omit output for a reusable managed path; cached images may be replaced. Explicit output overwrite requires true. Does not change the recipe.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "render",
        description: "Export a selected local project revision to an image file with explicit color and bit-depth options. Writes output and workfile registry; refuses overwrite unless true. temporary=true registers a draft for later cleanup. Does not finalize or clean the project.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "grade",
        description: "Process a local image with an explicit node recipe and export the result without creating a project. Writes output; refuses existing output unless overwrite=true. RAW/color support is limited to reported capabilities.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "cutout",
        description: "Extract a foreground from a local image using color, GrabCut or trimap settings; export transparent output and optional matte. Writes files; overwrite requires true. Complex backgrounds need user selection marks; no semantic AI model is used.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "stats",
        description: "Compute statistics and optional scopes for a local project revision without saving images or modifying the project. Scopes are opt-in; full histogram requires _response=full.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "analyze",
        description: "Compute image statistics for a local source and optional recipe without saving output or modifying files. Full histogram requires _response=full.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "validate",
        description: "Validate a node recipe and optional color pipeline without rendering or saving files. Referenced local color assets may be read.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "selections",
        description: "Read saved viewer selections and notes from a local project for mask-guided edits. Does not change selections or the recipe.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "selection_save",
        description: "Append a geometry mask and note to the local project's selection sidecar for a specified revision. Requires expect_revision; repeated calls can append duplicate selections. Does not apply the mask to the recipe.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "cleanup_plan",
        description: "Read the cleanup plan for an explicitly selected final project revision. Lists registered workfiles to recycle and retained files; does not finalize, delete or recycle anything.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "finalize",
        description: "Mark an explicitly selected project revision final and recycle eligible registered temporary files. Requires expect_revision and user-authorized cleanup. Preserves source/assets/history/durable exports. Windows local fixed drives only; partial failures do not undo finalization. Inspect failed/skipped/retained before retrying.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "workfile_register",
        description: "Register a local project workfile and its revision/role for retention or cleanup. Updates registry metadata; assigning a temporary role can make the file eligible for later recycling. Does not immediately recycle the file.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "cdl_inspect",
        description: "Read ASC CDL corrections and metadata from a local CDL file without editing or exporting it.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "cdl_import",
        description: "Read a local ASC CDL correction and return an edit-ready operation in an explicitly chosen color space/style. Does not apply edits or write the project.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "cdl_export",
        description: "Export an explicit CDL document or selected project nodes to a local CDL file. Writes output; existing output requires overwrite=true. Reports color-chain context that CDL cannot encode.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "lut_inspect",
        description: "Inspect a local 1D/3D cube LUT's dimensions and encoding ranges without changing it.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "lut_bake",
        description: "Bake a color-only recipe or project revision to a local 3D cube LUT and report sampled error. Writes output; overwrite requires true. Rejects spatial effects and unsupported alpha processing.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "ocio_configs",
        description: "List bundled OpenColorIO configurations without modifying files or accessing arbitrary paths.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "ocio_inspect",
        description: "Inspect a built-in or local OpenColorIO configuration and its spaces/views. Reads referenced local assets; does not save or modify the configuration.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "ocio_transform",
        description: "Transform supplied RGBA pixel values through a built-in or local OpenColorIO configuration. Reads color assets; returns values without exporting or changing files.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "schema",
        description: "Read supplementary recipe/operator/mask/edit or CLI request JSON Schema. Every callable MCP operation already has its own complete input schema; this tool does not discover hidden executable operations.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "job_status",
        description: "Read a background job's progress, commit receipt and result using its returned job ID. Jobs are session-local; failed/cancelled preview does not imply edits were rolled back. Does not queue or cancel work.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "job_cancel",
        description: "Request cooperative cancellation of a background job in this session. Can stop queued/running work; cannot roll back committed revisions or completed exports. Read job_status and project_info to determine committed state.",
        read_only: false,
        destructive: true,
        open_world: false,
    },
    Tool {
        command: "viewer_open",
        description: "Start or reuse a loopback viewer for a local project. Starts a stateful child service; does not launch a browser or edit the project. Service ends with this MCP session and permits interactive project edits.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "viewer_status",
        description: "Inspect viewer services owned by this MCP session without starting or stopping them.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "viewer_close",
        description: "Stop this session's managed viewer for a local project. Interrupts that viewer service; does not terminate externally started services or remove project history.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "configure",
        description: "Change this session's per-engine pixel-cache budget and idle timeout, clearing cached data. Budget is 0..4096 MiB and timeout 1..3600 seconds; does not limit total process memory or configure managed viewers.",
        read_only: false,
        destructive: true,
        open_world: false,
    },
    Tool {
        command: "cache_info",
        description: "Read this session's engine cache usage and budget. Does not clear caches; figures exclude active rendering, viewer caches and total process memory.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "clear_cache",
        description: "Clear this session's cached pixels and statistics; request worker cache release after current work. Removes reusable cache data without cancelling jobs or deleting exported files.",
        read_only: false,
        destructive: true,
        open_world: false,
    },
];

pub const BACKGROUND: &[&str] = &[
    "adjust",
    "edit_preview",
    "preview",
    "compare",
    "render",
    "grade",
    "stats",
    "analyze",
    "finalize",
];

fn descriptor(tool: &Tool, background: bool) -> Value {
    let mut schema = protocol::target_schema(tool.command).expect("reviewed command schema");
    let properties = schema["properties"].as_object_mut().unwrap();
    properties.remove("command");
    properties.insert("_response".into(), json!({"type":"string","enum":["compact","full"],"default":"compact","description":"Structured result detail; full retains histograms."}));
    properties.insert("_inline_image".into(), json!({"type":"boolean","default":false,"description":"Return available preview pixels instead of a link."}));
    if tool.command == "preview" {
        properties.get_mut("include_analysis").unwrap()["default"] = json!(false);
    }
    if background {
        properties.insert("idempotency_key".into(), json!({"type":["string","null"],"minLength":1,"maxLength":128,"description":"Session-only retry key, <=128 UTF-8 bytes; reuse with identical arguments."}));
    }
    for (name, description) in [
        (
            "project",
            "Local .tinge path; relative to server working directory.",
        ),
        ("input", "Local source path; no URL fetching or upload."),
        (
            "output",
            "Local destination; source/project/assets remain protected.",
        ),
        (
            "expect_revision",
            "Observed project head; stale values reject the edit.",
        ),
        (
            "revision",
            "Revision to use; optional omission selects current head.",
        ),
        (
            "overwrite",
            "Allow replacing existing output; default false.",
        ),
        (
            "temporary",
            "Register a draft for later authorized cleanup; default false.",
        ),
        (
            "asset_base",
            "Relative recipe resources base; default server working directory.",
        ),
        ("job", "Job ID returned by this session's submit tool."),
    ] {
        if let Some(property) = properties.get_mut(name) {
            property["description"] = json!(description);
        }
    }
    for (name, minimum, maximum) in [
        ("max_edge", 1, 16384),
        ("history_limit", 0, 100),
        ("cache_budget_mib", 0, 4096),
        ("idle_seconds", 1, 3600),
    ] {
        if let Some(property) = properties.get_mut(name) {
            property["minimum"] = json!(minimum);
            property["maximum"] = json!(maximum);
        }
    }
    schema["required"]
        .as_array_mut()
        .unwrap()
        .retain(|v| v != "command");
    if tool.command == "adjust" {
        let fields = schema["properties"].as_object_mut().unwrap();
        fields.get_mut("adjustment_id").unwrap()["minLength"] = json!(1);
        fields.get_mut("adjustment_id").unwrap()["maxLength"] = json!(64);
        fields.get_mut("adjustment_id").unwrap()["pattern"] = json!("^[A-Za-z0-9_-]+$");
        fields.get_mut("white_balance").unwrap()["items"]["minimum"] = json!(0.001);
        fields.get_mut("white_balance").unwrap()["items"]["maximum"] = json!(100);
        let null_controls: serde_json::Map<_, _> = [
            "exposure",
            "contrast",
            "saturation",
            "vibrance",
            "white_balance",
            "shadows",
            "highlights",
            "whites",
            "blacks",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), json!({"type":"null"})))
        .collect();
        schema["not"] = json!({"properties":null_controls});
    }
    crate::schema_compact::compact(&mut schema);
    let description = if background {
        format!(
            "Queue this operation; returns job ID for job_status. Retry keys are session-only; after restart inspect state. Queuing changes session state. {}",
            tool.description
        )
    } else {
        tool.description.to_owned()
    };
    json!({"name":format!("tinge_{}{}", if background {"submit_"} else {""}, tool.command),"description":description,"inputSchema":schema,"annotations":{"readOnlyHint":tool.read_only && !background,"destructiveHint":tool.destructive,"openWorldHint":tool.open_world}})
}

pub fn list() -> &'static Value {
    static LIST: OnceLock<Value> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut tools: Vec<Value> = TOOLS.iter().map(|t| descriptor(t, false)).collect();
        tools.extend(
            TOOLS
                .iter()
                .filter(|t| BACKGROUND.contains(&t.command))
                .map(|t| descriptor(t, true)),
        );
        json!({"tools":tools})
    })
}

pub fn request(name: &str, mut arguments: Value) -> Result<(Request, String)> {
    let name = name.strip_prefix("tinge_").context("unknown tool")?;
    let (command, background) = name
        .strip_prefix("submit_")
        .map_or((name, false), |c| (c, true));
    ensure!(
        TOOLS.iter().any(|t| t.command == command)
            && (!background || BACKGROUND.contains(&command)),
        "unknown tool"
    );
    let map = arguments
        .as_object_mut()
        .context("arguments must be an object")?;
    ensure!(
        !map.contains_key("command"),
        "command is not an MCP input; select the named tool"
    );
    let key = if background {
        map.remove("idempotency_key")
    } else {
        None
    };
    map.insert("command".into(), json!(command));
    // Compact preview is the MCP default, independent of result verbosity.
    if command == "preview" {
        map.entry("include_analysis").or_insert(json!(false));
    }
    let request = serde_json::from_value(arguments).context("invalid tool arguments")?;
    let request = if background {
        Request::JobSubmit {
            request: Box::new(request),
            idempotency_key: key
                .map(serde_json::from_value)
                .transpose()
                .context("invalid idempotency_key")?
                .flatten(),
        }
    } else {
        request
    };
    Ok((request, name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn check_refs(value: &Value, root: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    let pointer = reference.strip_prefix('#').unwrap();
                    assert!(root.pointer(pointer).is_some(), "unresolved {reference}");
                }
                for value in map.values() {
                    check_refs(value, root);
                }
            }
            Value::Array(values) => {
                for value in values {
                    check_refs(value, root);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn public_surface_has_complete_schemas_annotations_and_no_dispatcher() {
        let tools = list()["tools"].as_array().unwrap();
        // Bound the actual advertised payload, including transitive definitions.
        assert!(serde_json::to_vec(tools).unwrap().len() < 290_000);
        let basic = tools.iter().find(|t| t["name"] == "tinge_adjust").unwrap();
        assert!(serde_json::to_vec(basic).unwrap().len() < 5_000);
        assert!(basic["inputSchema"].get("$defs").is_none());
        let names: BTreeSet<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names.len(), tools.len());
        let commands: BTreeSet<_> = protocol::command_names().into_iter().collect();
        let reviewed: BTreeSet<_> = TOOLS.iter().map(|t| t.command.to_owned()).collect();
        assert_eq!(
            commands
                .difference(&reviewed)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["batch", "job_submit"]
        );
        for name in [
            "tinge_run",
            "tinge_agent",
            "tinge_batch",
            "tinge_job_submit",
        ] {
            assert!(!names.contains(name));
        }
        for tool in tools {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object");
            assert_eq!(schema["additionalProperties"], false);
            assert!(schema["properties"].get("command").is_none());
            assert!(schema["properties"].get("request").is_none());
            assert!(schema["properties"].get("jobs").is_none());
            assert!(!tool["description"].as_str().unwrap().is_empty());
            for hint in ["readOnlyHint", "destructiveHint", "openWorldHint"] {
                assert!(tool["annotations"][hint].is_boolean());
            }
            if tool["annotations"]["readOnlyHint"] == true {
                assert_eq!(tool["annotations"]["destructiveHint"], false);
            }
            check_refs(schema, schema);
        }
        for command in BACKGROUND {
            let direct = tools
                .iter()
                .find(|t| t["name"] == format!("tinge_{command}"))
                .unwrap();
            let queued = tools
                .iter()
                .find(|t| t["name"] == format!("tinge_submit_{command}"))
                .unwrap();
            assert_eq!(queued["annotations"]["readOnlyHint"], false);
            assert_eq!(
                queued["annotations"]["destructiveHint"],
                direct["annotations"]["destructiveHint"]
            );
            assert_eq!(
                queued["inputSchema"]["$defs"],
                direct["inputSchema"]["$defs"]
            );
        }
    }

    #[test]
    fn named_calls_reject_operation_injection_and_validate_before_execution() {
        for (tool, arguments) in [
            ("tinge_run", json!({"command":"capabilities"})),
            ("tinge_agent", json!({"command":"capabilities"})),
            ("tinge_capabilities", json!({"command":"finalize"})),
            (
                "tinge_submit_preview",
                json!({"request":{"command":"finalize"}}),
            ),
            ("tinge_submit_apply", json!({})),
            ("tinge_batch", json!({"jobs":[]})),
            ("tinge_render", json!({"project":"p.tinge"})),
            ("tinge_job_status", json!({"job":"invented"})),
            ("tinge_capabilities", json!({"idempotency_key":"unused"})),
        ] {
            assert!(request(tool, arguments).is_err(), "accepted {tool}");
        }
        let (queued, _) = request(
            "tinge_submit_preview",
            json!({"project":"p.tinge","idempotency_key":"same"}),
        )
        .unwrap();
        let Request::JobSubmit {
            request,
            idempotency_key,
        } = queued
        else {
            panic!("not queued")
        };
        assert_eq!(idempotency_key.as_deref(), Some("same"));
        assert!(matches!(
            *request,
            Request::Preview {
                include_analysis: false,
                ..
            }
        ));
    }
}
