use crate::api::{Request, Session};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

fn request_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| serde_json::to_value(schemars::schema_for!(Request)).unwrap())
}
#[cfg(test)]
pub fn command_names() -> Vec<String> {
    request_schema()["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|variant| {
            variant["properties"]["command"]["const"]
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}
fn references(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                found.insert(reference.into());
            }
            for value in map.values() {
                references(value, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                references(value, found);
            }
        }
        _ => {}
    }
}
pub fn target_schema(target: &str) -> Result<Value> {
    let full = request_schema();
    let (variants, tag, name) = if let Some(name) = target.strip_prefix("op:") {
        (&full["$defs"]["Operation"]["oneOf"], "type", name)
    } else if let Some(name) = target.strip_prefix("mask:") {
        (&full["$defs"]["Mask"]["oneOf"], "type", name)
    } else if let Some(name) = target.strip_prefix("edit:") {
        (&full["$defs"]["Edit"]["oneOf"], "type", name)
    } else {
        (&full["oneOf"], "command", target)
    };
    let mut schema = variants
        .as_array()
        .context("unknown schema group")?
        .iter()
        .find(|variant| variant["properties"][tag]["const"] == name)
        .cloned()
        .context(
            "unknown schema target; use a command name, op:<type>, mask:<type>, or edit:<type>",
        )?;
    let mut pending = BTreeSet::new();
    references(&schema, &mut pending);
    let mut definitions = BTreeMap::new();
    while let Some(reference) = pending.pop_first() {
        if reference == "#" {
            return Ok(full.clone());
        }
        let name = reference
            .strip_prefix("#/$defs/")
            .context("unsupported schema reference")?;
        if definitions.contains_key(name) {
            continue;
        }
        let value = full["$defs"]
            .get(name)
            .context("schema definition missing")?
            .clone();
        references(&value, &mut pending);
        definitions.insert(name.to_owned(), value);
    }
    if !definitions.is_empty() {
        schema["$defs"] = serde_json::to_value(definitions)?;
    }
    Ok(schema)
}
fn compact(mut data: Value, command: &str) -> Value {
    if command == "capabilities" {
        return json!({"name":data["name"],"version":env!("CARGO_PKG_VERSION"),"status":data["status"],"processing":data["processing"],"tools":crate::tools::list()["tools"].as_array().unwrap().iter().map(|tool| &tool["name"]).collect::<Vec<_>>(),"workflow":["project_info","adjust","render"],"advanced_edit":"edit_preview for explicit graphs/masks; read include_recipe:true only when needed","schema_target":"command name, op:<type>, mask:<type>, edit:<type>","workfiles":"omit preview output for managed path; temporary=true marks draft exports; finalize recycles registered work after explicit final-version selection","jobs":"background:true on the same tool; session-local; mutation idempotency keys retained for this session; cancellation never rolls back commits"});
    }
    if let Some(analysis) = data.get_mut("analysis").and_then(Value::as_object_mut) {
        analysis.remove("histogram");
    }
    // analyze returns its statistics at the root, including inside job results.
    if data.get("working_space").is_some()
        && let Some(object) = data.as_object_mut()
    {
        object.remove("histogram");
    }
    if let Some(preview) = data.get_mut("preview")
        && preview.get("resource_uri").is_some()
    {
        *preview = compact(preview.take(), "preview");
    }
    if let Some(result) = data.get_mut("result") {
        *result = compact(result.take(), "result");
    }
    data
}

fn write_line(out: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
fn input_channel() -> Receiver<io::Result<String>> {
    let (sender, receiver) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}
fn next_line(
    session: &mut Session,
    input: &Receiver<io::Result<String>>,
) -> Option<io::Result<String>> {
    loop {
        match input.recv_timeout(Duration::from_secs(session.idle_seconds)) {
            Ok(line) => return Some(line),
            Err(mpsc::RecvTimeoutError::Timeout) => session.clear_cached_data(),
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}
pub fn jsonl(session: &mut Session) -> Result<()> {
    session.persistent = true;
    let input = input_channel();
    let mut stdout = io::stdout().lock();
    while let Some(line) = next_line(session, &input) {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let result = serde_json::from_str::<Request>(&line)
            .context("invalid request JSON")
            .and_then(|r| session.run(r));
        let response = match result {
            Ok(data) => {
                let failed = crate::outcome::failed(&data);
                json!({"ok":!failed,"data":data})
            }
            Err(e) => json!({"ok":false,"error":crate::api::error_json(&e)}),
        };
        write_line(&mut stdout, &response)?;
    }
    Ok(())
}
fn rpc_error(id: Value, code: i32, message: String) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn resource(session: &Session, uri: &str) -> Result<Value> {
    let path = session
        .previews
        .get(uri)
        .context("unknown preview URI; generate a preview in this server session first")?;
    let bytes = std::fs::read(path)?;
    ensure!(
        bytes.len() <= 32 * 1024 * 1024,
        "preview exceeds 32MiB MCP transfer limit"
    );
    ensure!(
        uri.rsplit('/').next() == Some(blake3::hash(&bytes).to_hex().as_str()),
        "preview content changed; generate a new preview for the current revision"
    );
    Ok(json!({"uri":uri,"mimeType":"image/png","blob":STANDARD.encode(bytes)}))
}
pub fn handle(session: &mut Session, message: Value, initialized: &mut bool) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "invalid JSON-RPC request".into(),
        ));
    }
    let method = method.unwrap();
    let id = id?; // Notifications never receive responses.
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let result: Result<Value> = (|| match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-11-25");
            let protocol = if ["2025-11-25", "2025-06-18", "2024-11-05"].contains(&requested) {
                requested
            } else {
                "2025-11-25"
            };
            *initialized = true;
            session.persistent = true;
            Ok(
                json!({"protocolVersion":protocol,"capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"tinge","version":env!("CARGO_PKG_VERSION")},"instructions":"Use named tools and visible schemas. Basic grading: project_info (recipe omitted), adjust all controls in one call, render. Reuse adjustment_id and returned revision; omitted controls stay unchanged. Load edit_preview and include_recipe:true only for advanced graphs/masks. Re-read on conflict/reconnect. _inline_image:true when viewing pixels. Heavy work: background:true then job_status. Before retrying check committed/revision/preview_error; cancellation never undoes commits. Jobs and retry keys are session-local. Paths are local and unrestricted. Finalize only a selected revision with authorized cleanup."}),
            )
        }
        "ping" => Ok(json!({})),
        _ if !*initialized => anyhow::bail!("initialize the MCP session first"),
        "tools/list" => Ok(crate::tools::list().clone()),
        "tools/call" => {
            let tool = params
                .get("name")
                .and_then(Value::as_str)
                .context("missing tool name")?;
            let mut arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let map = arguments
                .as_object_mut()
                .context("arguments must be an object")?;
            let response = map.remove("_response");
            ensure!(
                response
                    .as_ref()
                    .is_none_or(|v| v == "compact" || v == "full"),
                "_response must be compact or full"
            );
            let lean = response.as_ref().is_none_or(|v| v == "compact");
            let inline = map.remove("_inline_image");
            ensure!(
                inline.as_ref().is_none_or(Value::is_boolean),
                "_inline_image must be boolean"
            );
            let inline = inline.and_then(|v| v.as_bool()).unwrap_or(false);
            let result = crate::tools::request(tool, arguments);
            let command = result
                .as_ref()
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|_| tool.to_owned());
            let result = result.and_then(|(request, _)| session.run(request));
            match result {
                Ok(data) => {
                    let data = if lean { compact(data, &command) } else { data };
                    let failed = crate::outcome::failed(&data);
                    let mut content = vec![
                        json!({"type":"text","text":format!("Tinge {command}: {}", if failed { "failed; inspect structuredContent for committed changes and recovery details" } else { data.get("status").and_then(Value::as_str).unwrap_or("completed") })}),
                    ];
                    for uri in crate::jobs::resource_uris(&data) {
                        if inline {
                            match resource(session, &uri) {
                                Ok(r) => content.push(json!({"type":"image","mimeType":"image/png","data":r["blob"]})),
                                Err(error) => content.push(json!({"type":"text","text":format!("Preview transfer unavailable: {error}; operation result and any committed revision remain valid.")})),
                            }
                        } else {
                            content.push(json!({"type":"resource_link","uri":uri,"name":"preview.png","mimeType":"image/png"}));
                        }
                    }
                    Ok(json!({"content":content,"structuredContent":data,"isError":failed}))
                }
                Err(e) => Ok(
                    json!({"content":[{"type":"text","text":serde_json::to_string(&crate::api::error_json(&e))?}],"isError":true}),
                ),
            }
        }
        "resources/list" => Ok(
            json!({"resources":session.previews.iter().map(|(uri,p)|json!({"uri":uri,"name":p.file_name().unwrap_or_default().to_string_lossy(),"mimeType":"image/png"})).collect::<Vec<_>>() }),
        ),
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .context("missing URI")?;
            Ok(json!({"contents":[resource(session,uri)?]}))
        }
        _ => anyhow::bail!("method not found: {method}"),
    })();
    Some(match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(e) => rpc_error(
            id,
            if ![
                "initialize",
                "ping",
                "tools/list",
                "tools/call",
                "resources/list",
                "resources/read",
            ]
            .contains(&method)
            {
                -32601
            } else {
                -32602
            },
            format!("{e:#}"),
        ),
    })
}
pub fn mcp(session: &mut Session) -> Result<()> {
    let input = input_channel();
    let mut stdout = io::stdout().lock();
    let mut initialized = false;
    while let Some(line) = next_line(session, &input) {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(v) => handle(session, v, &mut initialized),
            Err(e) => Some(rpc_error(Value::Null, -32700, e.to_string())),
        };
        if let Some(v) = response {
            write_line(&mut stdout, &v)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_rejects_changed_content() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("preview.png");
        let frame = tinge_core::Frame::new(1, 1, vec![[0.18, 0.18, 0.18, 1.0]]).unwrap();
        tinge_io::export(
            &frame,
            &path,
            tinge_io::ExportOptions {
                bit_depth: 8,
                ..Default::default()
            },
        )
        .unwrap();
        let uri = format!("tinge://preview/{}", tinge_io::hash_file(&path).unwrap());
        let mut s = Session::default();
        s.previews.insert(uri.clone(), path.clone());
        assert!(resource(&s, &uri).is_ok());
        std::fs::write(path, b"changed").unwrap();
        assert!(resource(&s, &uri).is_err());
    }
    #[test]
    fn mcp_handshake_tools_and_errors() {
        let mut s = Session::default();
        let mut init = false;
        let r=handle(&mut s,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),&mut init).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(r["result"]["serverInfo"]["name"], "tinge");
        assert!(
            handle(
                &mut s,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                &mut init
            )
            .is_none()
        );
        let r = handle(
            &mut s,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            &mut init,
        )
        .unwrap();
        assert!(
            r["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "tinge_capabilities")
        );
        let r=handle(&mut s,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tinge_capabilities","arguments":{}}}),&mut init).unwrap();
        assert_eq!(r["result"]["isError"], false);
        let r=handle(&mut s,json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tinge_capabilities","arguments":{"typo":1}}}),&mut init).unwrap();
        assert_eq!(r["result"]["isError"], true);
    }
}
