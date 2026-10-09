mod support;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn exe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinge"))
}

#[test]
fn native_command_identifies_as_tinge() {
    let version = exe().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("tinge ", env!("CARGO_PKG_VERSION"))
    );
    let help = exe().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: tinge")
    );
}

#[test]
fn committed_edit_with_failed_preview_is_reported_by_cli_jsonl_and_batch() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.png");
    let project = dir.path().join("image.tinge");
    let frame = tinge_core::Frame::new(2, 1, vec![[0.1, 0.2, 0.3, 1.0]; 2]).unwrap();
    tinge_io::export(&frame, &source, Default::default()).unwrap();
    tinge_project::init(&project, &source, None).unwrap();
    for (index, args) in [vec!["run", "-"], vec!["serve"], vec!["run", "-"]]
        .iter()
        .enumerate()
    {
        let output = dir.path().join(format!("blocked-{index}.png"));
        std::fs::create_dir(&output).unwrap();
        let mut body = json!({"command":"edit_preview","project":project,"expect_revision":index,"edits":[{"type":"upsert_node","node":{"id":"exp","op":{"type":"exposure","stops":0.2}}},{"type":"set_output","id":"exp"}],"output":output,"overwrite":true});
        if index == 2 {
            body = json!({"command":"batch","stop_on_error":true,"jobs":[body,{"command":"capabilities"}]});
        }
        let mut child = exe()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{body}").unwrap();
        let result = child.wait_with_output().unwrap();
        // Persistent servers report failure per response and remain usable.
        assert_eq!(result.status.success(), index == 1);
        let response: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(response["ok"], false);
        let data = if index == 2 {
            assert_eq!(response["data"]["failures"], 1);
            assert_eq!(response["data"]["results"].as_array().unwrap().len(), 1);
            &response["data"]["results"][0]["data"]
        } else {
            &response["data"]
        };
        assert!(data["preview_error"].is_object());
        assert_eq!(data["committed"], true);
        assert_eq!(data["revision"], index + 1);
        assert_eq!(
            tinge_project::load(&project).unwrap().revision,
            index as u64 + 1
        );
        assert!(output.is_dir());
    }
}
#[test]
fn nested_batch_failure_propagates_and_stops() {
    let request = json!({"command":"batch","stop_on_error":true,"jobs":[{"command":"batch","jobs":[{"command":"inspect","input":"a-file-that-does-not-exist.png"}]},{"command":"capabilities"}]});
    let mut child = exe()
        .args(["run", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let r = child.wait_with_output().unwrap();
    assert!(!r.status.success());
    let data: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(data["ok"], false);
    assert_eq!(data["data"]["failures"], 1);
    assert_eq!(data["data"]["results"].as_array().unwrap().len(), 1);
}
#[test]
fn cli_project_lifecycle_and_json_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.png");
    let project = dir.path().join("image.tinge");
    let out = dir.path().join("preview.png");
    let frame = tinge_core::Frame::new(2, 1, vec![[0.1, 0.2, 0.3, 1.0]; 2]).unwrap();
    tinge_io::export(&frame, &source, Default::default()).unwrap();
    let p = exe()
        .arg("init")
        .arg(&source)
        .arg("--project")
        .arg(&project)
        .output()
        .unwrap();
    assert!(p.status.success(), "{}", String::from_utf8_lossy(&p.stderr));
    let response: Value = serde_json::from_slice(&p.stdout).unwrap();
    assert_eq!(response["data"]["revision"], 0);
    let request = json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"upsert_node","node":{"id":"exp","op":{"type":"exposure","stops":1}}},{"type":"set_output","id":"exp"}]});
    let mut child = exe()
        .args(["run", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let r = child.wait_with_output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let mut child = exe()
        .args(["run", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let r = child.wait_with_output().unwrap();
    assert!(!r.status.success());
    let error: Value = serde_json::from_slice(&r.stderr).unwrap();
    assert_eq!(error["error"]["code"], "revision_conflict");
    assert_eq!(error["error"]["actual_revision"], 1);
    let r = exe()
        .arg("preview")
        .arg(&project)
        .arg("--output")
        .arg(&out)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(out.exists());
    let r = exe()
        .arg("render")
        .arg(&project)
        .arg("--output")
        .arg(&project)
        .arg("--overwrite")
        .output()
        .unwrap();
    assert!(!r.status.success());
    assert_eq!(tinge_project::load(&project).unwrap().revision, 1);
    let r = exe()
        .arg("restore")
        .arg(&project)
        .arg("0")
        .args(["--expect-revision", "1"])
        .output()
        .unwrap();
    assert!(r.status.success());
    assert_eq!(tinge_project::load(&project).unwrap().revision, 2);
}
#[test]
fn mcp_stdio_roundtrip_and_preview_image_resource() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.png");
    let project = dir.path().join("image.tinge");
    let preview = dir.path().join("preview.png");
    let frame = tinge_core::Frame::new(1, 1, vec![[0.18, 0.18, 0.18, 1.0]]).unwrap();
    tinge_io::export(&frame, &source, Default::default()).unwrap();
    tinge_project::init(&project, &source, None).unwrap();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        support::call(
            3,
            json!({"command":"preview","project":project,"output":preview}),
        ),
        json!({"jsonrpc":"2.0","id":4,"method":"resources/list"}),
    ];
    let mut child = exe()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for msg in messages {
        writeln!(stdin, "{msg}").unwrap();
    }
    drop(stdin);
    let r = child.wait_with_output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let results: Vec<Value> = String::from_utf8(r.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(results.len(), 4);
    assert_eq!(results[2]["result"]["isError"], false);
    assert_eq!(results[2]["result"]["content"][1]["type"], "image");
    assert!(
        results[2]["result"]["content"][1]["data"]
            .as_str()
            .unwrap()
            .starts_with("iVBOR")
    );
    assert_eq!(
        results[3]["result"]["resources"][0]["mimeType"],
        "image/png"
    );
    assert_eq!(
        results[1]["result"]["tools"][0]["inputSchema"]["type"],
        "object"
    );
}
