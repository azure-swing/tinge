use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use tinge_core::Frame;

fn run(request: Value, success: bool) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
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
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value =
        serde_json::from_slice(if success { &out.stdout } else { &out.stderr }).unwrap();
    if success {
        value["data"].clone()
    } else {
        value
    }
}
fn input(path: &std::path::Path) -> Frame {
    let frame = Frame::new(
        8,
        8,
        (0..64)
            .map(|i| {
                if i % 8 < 4 {
                    [1., 0., 0., 0.5]
                } else {
                    [0., 1., 0., 1.]
                }
            })
            .collect(),
    )
    .unwrap();
    tinge_io::export(&frame, path, tinge_io::ExportOptions::default()).unwrap();
    frame
}
#[test]
fn cutout_exports_exact_scalar_alpha_and_protects_inputs_and_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.png");
    input(&source);
    let output = dir.path().join("cutout.png");
    let matte = dir.path().join("alpha.png");
    let opts = json!({"selection":{"type":"color","color":[0,1,0]}});
    let data = run(
        json!({"command":"cutout","input":source,"output":output,"matte":matte,"options":opts}),
        true,
    );
    assert_eq!(data["cutout"]["model_weights_required"], false);
    assert_eq!(data["matte_bit_depth"], 16);
    let result = tinge_io::load(&output, None).unwrap();
    let alpha = tinge_io::load_matte(&matte, 8, 8).unwrap();
    for (i, (p, a)) in result.pixels.iter().zip(&alpha).enumerate() {
        assert_eq!(p[3], *a);
        assert!((*a - if i % 8 < 4 { 0.5 } else { 0.0 }).abs() < 1.0 / 65535.0);
    }
    for ext in ["tiff", "exr"] {
        let dest = dir.path().join(format!("alpha.{ext}"));
        run(
            json!({"command":"cutout","input":source,"output":dest,"options":opts}),
            true,
        );
        let decoded = tinge_io::load(&dest, None).unwrap();
        for (p, a) in decoded.pixels.iter().zip(&alpha) {
            assert!((p[3] - a).abs() < 1e-6);
        }
    }
    let bad_float = dir.path().join("float-alpha.tiff");
    run(
        json!({"command":"cutout","input":source,"output":bad_float,"bit_depth":32,"options":opts}),
        false,
    );
    assert!(!bad_float.exists());
    let bytes = std::fs::read(&output).unwrap();
    run(
        json!({"command":"cutout","input":source,"output":output,"options":opts}),
        false,
    );
    assert_eq!(bytes, std::fs::read(&output).unwrap());
    run(
        json!({"command":"cutout","input":source,"output":source,"options":opts,"overwrite":true}),
        false,
    );
    let jpeg = dir.path().join("bad.jpg");
    run(
        json!({"command":"cutout","input":source,"output":jpeg,"options":opts}),
        false,
    );
    assert!(!jpeg.exists());
    let bad = dir.path().join("bad.png");
    run(
        json!({"command":"cutout","input":source,"output":bad,"options":{"selection":{"type":"color","color":[0,1,0],"typo":1}}}),
        false,
    );
    assert!(!bad.exists());
    let shared = dir.path().join("shared.png");
    run(
        json!({"command":"cutout","input":source,"output":shared,"matte":shared,"options":opts}),
        false,
    );
    assert!(!shared.exists());
    let extmask = json!({"selection":{"type":"mask","mask":{"type":"bitmap","path":matte}}});
    run(
        json!({"command":"cutout","input":source,"output":matte,"options":extmask,"overwrite":true}),
        false,
    );
}
#[test]
fn cutout_project_freezes_mattes_restores_revisions_and_rejects_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.png");
    input(&source);
    let path = dir.path().join("key.tinge");
    let external = dir.path().join("matte.png");
    let f = Frame::new(8, 8, vec![[0., 0., 0., 0.5]; 64]).unwrap();
    tinge_io::export_matte(&f, &external, 16, false).unwrap();
    run(
        json!({"command":"init","input":source,"project":path}),
        true,
    );
    run(
        json!({"command":"apply","project":path,"expect_revision":0,"edits":[{"type":"upsert_node","node":{"id":"key","op":{"type":"cutout","options":{"selection":{"type":"mask","mask":{"type":"bitmap","path":external}}}}}},{"type":"set_output","id":"key"}]}),
        true,
    );
    std::fs::remove_file(external).unwrap();
    std::fs::remove_file(source).unwrap();
    let p: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let asset = p["history"][1]["recipe"]["nodes"][0]["op"]["options"]["selection"]["mask"]["path"]
        .as_str()
        .unwrap();
    assert!(path.parent().unwrap().join(asset).exists());
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for (index, revision) in [0, 1, 0, 1].into_iter().enumerate() {
        writeln!(stdin,"{}",json!({"command":"render","project":path,"revision":revision,"output":dir.path().join(format!("r{index}.png"))})).unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(responses.iter().all(|r| r["ok"] == true));
    let frames: Vec<_> = (0..4)
        .map(|i| tinge_io::load(&dir.path().join(format!("r{i}.png")), None).unwrap())
        .collect();
    assert_eq!(frames[0].pixels, frames[2].pixels);
    assert_eq!(frames[1].pixels, frames[3].pixels);
    assert_ne!(frames[0].pixels[0][3], frames[1].pixels[0][3]);
    run(
        json!({"command":"restore","project":path,"expect_revision":1,"revision":0}),
        true,
    );
    let bytes = std::fs::read(&path).unwrap();
    run(
        json!({"command":"apply","project":path,"expect_revision":2,"edits":[{"type":"upsert_node","node":{"id":"bad","op":{"type":"cutout","options":{"selection":{"type":"grab_cut","max_edge":4096}}}}}]}),
        false,
    );
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    std::fs::write(path.parent().unwrap().join(asset), b"changed").unwrap();
    run(
        json!({"command":"render","project":path,"revision":1,"output":dir.path().join("tampered.png")}),
        false,
    );
    // Alpha-changing nodes mix in premultiplied space: a half key retains foreground RGB.
    let mix = dir.path().join("mix.tinge");
    let src = dir.path().join("again.png");
    input(&src);
    run(json!({"command":"init","input":src,"project":mix}), true);
    run(
        json!({"command":"apply","project":mix,"expect_revision":0,"edits":[{"type":"upsert_node","node":{"id":"key","mix":0.5,"op":{"type":"cutout","options":{"selection":{"type":"color","color":[0,1,0]}}}}},{"type":"set_output","id":"key"}]}),
        true,
    );
    let out = dir.path().join("mix.png");
    run(json!({"command":"render","project":mix,"output":out}), true);
    let f = tinge_io::load(&out, None).unwrap();
    assert!((f.pixels[7][1] - 1.0).abs() < 5e-5, "{:?}", f.pixels[7]);
    assert!((f.pixels[7][3] - 0.5).abs() < 1.0 / 65535.0);
}
#[test]
fn cutout_native_command_mcp_and_schema_are_available() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.png");
    input(&source);
    let opts = dir.path().join("options.json");
    std::fs::write(&opts, r#"{"selection":{"type":"color","color":[0,1,0]}}"#).unwrap();
    let output = dir.path().join("native.png");
    let out = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .arg("cutout")
        .arg(&source)
        .arg("--options")
        .arg(&opts)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(output.exists());
    let schema = run(json!({"command":"schema"}), true);
    assert!(schema.to_string().contains("\"cutout\""));
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin,"{}",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})).unwrap();
    let output = dir.path().join("mcp.png");
    writeln!(stdin,"{}",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tinge_cutout","arguments":{"input":source,"output":output,"options":{"selection":{"type":"color","color":[0,1,0]}}}}})).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses[1]["result"]["isError"], false);
    assert!(output.exists());
}
