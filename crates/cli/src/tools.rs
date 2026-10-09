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
        description: "Adjust basic photo controls in one commit and PNG preview. Absolute values; omitted/null controls stay unchanged. Supply at least one control. Preview failure can follow a commit: check committed/revision before retrying. For graphs/masks use edit_preview.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "capabilities",
        description: "List implemented image-processing capabilities and limitations.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "project_info",
        description: "Read a project's current or selected revision, optional recipe and paginated history.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "show",
        description: "Read all historical recipes. For a revision or paginated summary use project_info.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "inspect",
        description: "Read image dimensions, format and color metadata.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "raw_plan",
        description: "Inspect effective RAW development settings and optional sensor samples. Camera support depends on rawler.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "init",
        description: "Create a revisioned project with frozen source and color assets. Rejects an existing project; check project state after an uncertain result.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "edit_preview",
        description: "Commit graph/mask edits and save a PNG preview of that revision. For basic controls use adjust. Preview failure can follow a commit: check committed/revision before retrying.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "apply",
        description: "Commit edits without a preview, preserving history. Check project state after an uncertain commit.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "restore",
        description: "Restore a historical recipe as a new revision, preserving history.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "branch",
        description: "Create a branch; checkout optionally activates it. Existing branch references are retained.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "tag",
        description: "Tag the current revision, creating a metadata revision with unchanged recipe. Rejects existing tag names.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "preview",
        description: "Save an SDR sRGB PNG preview. Omit output to reuse a managed cache path, replacing its earlier preview. Analysis is opt-in.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "compare",
        description: "Save a PNG comparison of the original and selected revision. Omit output to reuse a managed path, replacing its earlier image.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "render",
        description: "Export a project revision with color and bit-depth options. Registers the output; temporary=true marks a draft. Cleanup is separate.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "grade",
        description: "Process an image with an explicit node recipe and export without creating a project.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "cutout",
        description: "Extract foreground using color, GrabCut or trimap; export transparent output and optional matte. Complex backgrounds need selection marks; no semantic AI model.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "stats",
        description: "Compute project revision statistics and optional scopes. Full histogram requires _response=full.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "analyze",
        description: "Compute source image statistics. For a project recipe use stats. Full histogram requires _response=full.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "validate",
        description: "Validate a node recipe and optional color pipeline, reading referenced color assets.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "selections",
        description: "Read saved viewer selections and notes for mask-guided edits.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "selection_save",
        description: "Append a geometry mask and note for a revision; does not apply it to the recipe. Repeated calls can append duplicates.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "cleanup_plan",
        description: "Preview files to recycle or retain for a selected final revision.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "finalize",
        description: "Finalize a selected revision and recycle eligible temporary files with user authorization. Preserves source/assets/history/durable exports. Windows fixed drives only. Partial failures keep finalization; check failed/skipped/retained before retrying.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "workfile_register",
        description: "Register a workfile's revision and role. A temporary role makes it eligible for later recycling.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "cdl_inspect",
        description: "Read ASC CDL corrections and metadata.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "cdl_import",
        description: "Convert an ASC CDL correction to an edit-ready operation in the chosen color space/style.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "cdl_export",
        description: "Export a CDL document or selected project nodes. Reports color-chain context that CDL cannot encode.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "lut_inspect",
        description: "Read a 1D/3D cube LUT's dimensions and encoding ranges.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "lut_bake",
        description: "Bake a project's color-only revision to a 3D cube LUT with sampled error. Rejects spatial effects and unsupported alpha processing.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "ocio_configs",
        description: "List bundled OpenColorIO configurations.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "ocio_inspect",
        description: "Read an OpenColorIO configuration's spaces/views and referenced assets.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "ocio_transform",
        description: "Transform supplied RGBA values through an OpenColorIO configuration.",
        read_only: true,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "schema",
        description: "Read recipe/operator/mask/edit or CLI request JSON Schema. MCP input schemas are already supplied with each tool.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "job_status",
        description: "Read a session job's progress, commit receipt and result. Failed/cancelled preview may still have committed edits.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "job_cancel",
        description: "Request cancellation of a session job. Commits and exports are not rolled back; check job_status for the result.",
        read_only: false,
        destructive: true,
        open_world: false,
    },
    Tool {
        command: "viewer_open",
        description: "Start or reuse a loopback viewer supporting interactive project edits. Returns a URL; service ends with this MCP session.",
        read_only: false,
        destructive: false,
        open_world: true,
    },
    Tool {
        command: "viewer_status",
        description: "List viewer services owned by this MCP session.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "viewer_close",
        description: "Stop this session's managed viewer for a project.",
        read_only: false,
        destructive: true,
        open_world: true,
    },
    Tool {
        command: "configure",
        description: "Set this session's per-engine pixel-cache budget and idle timeout, clearing caches. Excludes viewer caches; budget does not bound total memory.",
        read_only: false,
        destructive: true,
        open_world: false,
    },
    Tool {
        command: "cache_info",
        description: "Read engine cache usage and budget. Excludes active rendering, viewer caches and other process memory.",
        read_only: true,
        destructive: false,
        open_world: false,
    },
    Tool {
        command: "clear_cache",
        description: "Clear session pixels/statistics caches; release worker cache after current work.",
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

fn descriptor(tool: &Tool) -> Value {
    let background = BACKGROUND.contains(&tool.command);
    let mut schema = protocol::target_schema(tool.command).expect("reviewed command schema");
    if tool.command == "analyze" {
        schema["properties"]
            .as_object_mut()
            .unwrap()
            .remove("recipe");
    }
    if tool.command == "lut_bake" {
        let source = schema["properties"]
            .as_object_mut()
            .unwrap()
            .remove("source")
            .unwrap();
        let source = &schema["$defs"][source["$ref"]
            .as_str()
            .unwrap()
            .strip_prefix("#/$defs/")
            .unwrap()];
        let project = source["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["properties"]["type"]["const"] == "project")
            .unwrap()
            .clone();
        for field in ["project", "revision"] {
            schema["properties"][field] = project["properties"][field].clone();
        }
        schema["properties"]
            .as_object_mut()
            .unwrap()
            .remove("asset_base");
        let required = schema["required"].as_array_mut().unwrap();
        required.retain(|v| v != "source");
        required.push(json!("project"));
    }
    crate::schema_compact::prune_definitions(&mut schema);
    let properties = schema["properties"].as_object_mut().unwrap();
    properties.remove("command");
    properties.insert("_response".into(), json!({"type":"string","enum":["compact","full"],"default":"compact","description":"Result detail; full retains histograms."}));
    properties.insert("_inline_image".into(), json!({"type":"boolean","default":false,"description":"Return preview pixels instead of a link."}));
    if tool.command == "preview" {
        properties.get_mut("include_analysis").unwrap()["default"] = json!(false);
    }
    if background {
        properties.insert("background".into(), json!({"type":"boolean","default":false,"description":"Queue a job instead of waiting; returns job ID for job_status."}));
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
            "Observed project head; rejects stale revisions.",
        ),
        ("revision", "Revision; omission selects current head."),
        ("overwrite", "Allow replacing existing output."),
        ("temporary", "Register a draft for later cleanup."),
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
    if background {
        schema["if"] = json!({"properties":{"idempotency_key":{"type":"string"}},"required":["idempotency_key"]});
        schema["then"] =
            json!({"properties":{"background":{"const":true}},"required":["background"]});
    }
    crate::schema_compact::simplify_descriptions(&mut schema);
    crate::schema_compact::compact(&mut schema);
    let description = if background {
        format!("{} Supports background execution.", tool.description)
    } else {
        tool.description.to_owned()
    };
    json!({"name":format!("tinge_{}", tool.command),"description":description,"inputSchema":schema,"annotations":{"readOnlyHint":tool.read_only && !background,"destructiveHint":tool.destructive,"openWorldHint":tool.open_world}})
}

pub fn list() -> &'static Value {
    static LIST: OnceLock<Value> = OnceLock::new();
    LIST.get_or_init(|| {
        let tools: Vec<Value> = TOOLS.iter().map(descriptor).collect();
        json!({"tools":tools})
    })
}

pub fn request(name: &str, mut arguments: Value) -> Result<(Request, String)> {
    let name = name.strip_prefix("tinge_").context("unknown tool")?;
    let command = name;
    ensure!(TOOLS.iter().any(|t| t.command == command), "unknown tool");
    let map = arguments
        .as_object_mut()
        .context("arguments must be an object")?;
    ensure!(
        !map.contains_key("command"),
        "command is not an MCP input; select the named tool"
    );
    let supports_background = BACKGROUND.contains(&command);
    let background = if supports_background {
        map.remove("background")
            .map(serde_json::from_value::<bool>)
            .transpose()
            .context("background must be boolean")?
            .unwrap_or(false)
    } else {
        false
    };
    let key = if supports_background {
        map.remove("idempotency_key")
    } else {
        None
    };
    let key: Option<String> = key
        .map(serde_json::from_value)
        .transpose()
        .context("invalid idempotency_key")?
        .flatten();
    ensure!(
        background || key.is_none(),
        "idempotency_key requires background:true"
    );
    if command == "analyze" {
        ensure!(
            !map.contains_key("recipe"),
            "analyze accepts source controls; use stats for a project recipe"
        );
    }
    if command == "lut_bake" {
        ensure!(
            !map.contains_key("source") && !map.contains_key("asset_base"),
            "lut_bake accepts project/revision"
        );
        let project = map.remove("project").context("missing project")?;
        let revision = map.remove("revision").unwrap_or(Value::Null);
        map.insert(
            "source".into(),
            json!({"type":"project","project":project,"revision":revision}),
        );
    }
    map.insert("command".into(), json!(command));
    // Compact preview is the MCP default, independent of result verbosity.
    if command == "preview" {
        map.entry("include_analysis").or_insert(json!(false));
    }
    let request = serde_json::from_value(arguments).context("invalid tool arguments")?;
    let request = if background {
        Request::JobSubmit {
            request: Box::new(request),
            idempotency_key: key,
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
        assert_eq!(tools.len(), 42);
        // Bound the actual advertised payload, including transitive definitions.
        assert!(serde_json::to_vec(tools).unwrap().len() < 180_000);
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
            assert_eq!(direct["annotations"]["readOnlyHint"], false);
            assert_eq!(
                direct["inputSchema"]["properties"]["background"]["default"],
                false
            );
            assert!(!names.contains(format!("tinge_submit_{command}").as_str()));
        }
    }

    #[test]
    fn execution_mode_preserves_each_operation_and_rejects_unused_controls() {
        for (command, arguments) in [
            (
                "adjust",
                json!({"project":"p.tinge","expect_revision":0,"exposure":0.2}),
            ),
            (
                "edit_preview",
                json!({"project":"p.tinge","expect_revision":0,"edits":[]}),
            ),
            ("preview", json!({"project":"p.tinge"})),
            ("compare", json!({"project":"p.tinge"})),
            ("render", json!({"project":"p.tinge","output":"out.png"})),
            (
                "grade",
                json!({"input":"in.png","recipe":{},"output":"out.png"}),
            ),
            ("stats", json!({"project":"p.tinge"})),
            ("analyze", json!({"input":"in.png"})),
            (
                "finalize",
                json!({"project":"p.tinge","expect_revision":0,"revision":0}),
            ),
        ] {
            let name = format!("tinge_{command}");
            let (direct, _) = request(&name, arguments.clone()).unwrap();
            let mut background = arguments.clone();
            background["background"] = json!(true);
            background["idempotency_key"] = json!("once");
            let (
                Request::JobSubmit {
                    request: queued,
                    idempotency_key,
                },
                _,
            ) = request(&name, background).unwrap()
            else {
                panic!("not queued");
            };
            assert_eq!(
                serde_json::to_value(&direct).unwrap(),
                serde_json::to_value(queued).unwrap()
            );
            assert_eq!(idempotency_key.as_deref(), Some("once"));
            let mut invalid = arguments.clone();
            invalid["background"] = json!("true");
            assert!(request(&name, invalid).is_err());
            let mut invalid = arguments.clone();
            invalid["idempotency_key"] = json!("unused");
            assert!(request(&name, invalid).is_err());
            assert!(request(&format!("tinge_submit_{command}"), arguments).is_err());
        }
        assert!(request("tinge_capabilities", json!({"background":false})).is_err());
    }

    #[test]
    fn analysis_and_lut_contracts_are_narrow_and_match_dispatch() {
        assert!(request("tinge_analyze", json!({"input":"x.png","recipe":{}})).is_err());
        let (bake, _) = request(
            "tinge_lut_bake",
            json!({"project":"p.tinge","revision":2,"output":"x.cube"}),
        )
        .unwrap();
        assert!(matches!(
            bake,
            Request::LutBake {
                source: crate::api::LutBakeSource::Project {
                    revision: Some(2),
                    ..
                },
                asset_base: None,
                ..
            }
        ));
        assert!(
            request(
                "tinge_lut_bake",
                json!({"source":{"type":"recipe","recipe":{}},"output":"x.cube"})
            )
            .is_err()
        );
        for name in ["tinge_analyze", "tinge_lut_bake"] {
            let tools = list()["tools"].as_array().unwrap();
            let tool = tools.iter().find(|t| t["name"] == name).unwrap();
            assert!(serde_json::to_vec(tool).unwrap().len() < 8_000);
            assert!(tool["inputSchema"]["$defs"].get("Node").is_none());
            assert!(tool["inputSchema"]["$defs"].get("Mask").is_none());
            assert!(tool["inputSchema"]["properties"].get("recipe").is_none());
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
            "tinge_preview",
            json!({"project":"p.tinge","background":true,"idempotency_key":"same"}),
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
