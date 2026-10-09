use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
#[path = "../../io/tests/support/mod.rs"]
mod support;
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
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(if success {
        &output.stdout
    } else {
        &output.stderr
    })
    .unwrap();
    if success {
        response["data"].clone()
    } else {
        response
    }
}
#[test]
fn raw_revision_history_cache_restore_and_atomic_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sensor.dng");
    let project = dir.path().join("raw.tinge");
    std::fs::write(&input, support::dng()).unwrap();
    run(
        json!({"command":"init","input":input,"project":project}),
        true,
    );
    let original: Value = serde_json::from_slice(&std::fs::read(&project).unwrap()).unwrap();
    assert!(original["history"][0].get("raw_develop").is_none());
    let options =
        json!({"exposure_ev":1.0,"white_balance":{"type":"camera_gains","gains":[2.0,1.0,0.5]}});
    run(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"set_raw_develop","options":options}]}),
        true,
    );
    std::fs::remove_file(&input).unwrap();
    let updated: Value = serde_json::from_slice(&std::fs::read(&project).unwrap()).unwrap();
    assert_ne!(
        original["history"][0]["recipe_hash"],
        updated["history"][1]["recipe_hash"]
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for revision in [0, 1, 0, 1] {
        writeln!(
            stdin,
            "{}",
            json!({"command":"stats","project":project,"revision":revision})
        )
        .unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().all(|s| s["ok"] == true));
    assert_eq!(lines[0]["data"]["analysis"], lines[2]["data"]["analysis"]);
    assert_eq!(lines[1]["data"]["analysis"], lines[3]["data"]["analysis"]);
    let a = lines[0]["data"]["analysis"]["rgb_mean"][0]
        .as_f64()
        .unwrap();
    let b = lines[1]["data"]["analysis"]["rgb_mean"][0]
        .as_f64()
        .unwrap();
    assert!(b > a * 3.9);
    let bytes = std::fs::read(&project).unwrap();
    run(
        json!({"command":"apply","project":project,"expect_revision":1,"edits":[{"type":"set_raw_develop","options":{"white_levels":{"repeat":[1,1],"values":[64]}}}]}),
        false,
    );
    assert_eq!(bytes, std::fs::read(&project).unwrap());
    run(
        json!({"command":"apply","project":project,"expect_revision":1,"edits":[{"type":"set_raw_develop","options":{"white_balance":{"type":"temperature","kelvin":1200}}}]}),
        false,
    );
    assert_eq!(bytes, std::fs::read(&project).unwrap());
    run(
        json!({"command":"apply","project":project,"expect_revision":1,"edits":[{"type":"set_raw_develop","options":{"black_levels":{"repeat":[1,1],"values":[0]},"white_levels":{"repeat":[1,1],"values":[1e-38]}}}]}),
        false,
    );
    assert_eq!(bytes, std::fs::read(&project).unwrap());
    run(
        json!({"command":"restore","project":project,"expect_revision":1,"revision":0}),
        true,
    );
    let restored = run(json!({"command":"show","project":project}), true);
    assert_eq!(
        restored["history"][2]["recipe_hash"],
        original["history"][0]["recipe_hash"]
    );
    assert!(restored["history"][2].get("raw_develop").is_none());
    run(
        json!({"command":"branch","project":project,"expect_revision":2,"name":"adjusted","checkout":true}),
        true,
    );
    run(
        json!({"command":"apply","project":project,"expect_revision":3,"edits":[{"type":"set_raw_develop","options":{"white_balance":{"type":"temperature","kelvin":4500,"tint_duv":0.002}}}]}),
        true,
    );
    run(
        json!({"command":"tag","project":project,"expect_revision":4,"name":"raw-wb"}),
        true,
    );
    let tagged = run(json!({"command":"show","project":project}), true);
    assert_eq!(
        tagged["history"][4]["raw_develop"]["white_balance"]["kelvin"],
        4500.0
    );
    assert_eq!(
        tagged["history"][5]["raw_develop"],
        tagged["history"][4]["raw_develop"]
    );
    // Hash tampering is rejected before rendering.
    let mut tampered = tagged;
    tampered["history"][5]["raw_develop"]["exposure_ev"] = json!(2.0);
    std::fs::write(&project, serde_json::to_vec(&tampered).unwrap()).unwrap();
    run(json!({"command":"show","project":project}), false);
}
#[test]
fn raw_cli_plan_one_shot_mcp_and_non_raw_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sensor.dng");
    std::fs::write(&input, support::dng()).unwrap();
    let options = dir.path().join("raw.json");
    std::fs::write(&options, r#"{"exposure_ev":1,"white_balance":{"type":"temperature","kelvin":4500,"tint_duv":0.002}}"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .arg("raw-plan")
        .arg(&input)
        .arg("--raw-develop")
        .arg(&options)
        .output()
        .unwrap();
    assert!(out.status.success());
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(response["data"]["options"]["exposure_ev"], 1.0);
    assert_eq!(
        response["data"]["white_balance_source"],
        "temperature_kang2002_duv"
    );
    let output = dir.path().join("scene.exr");
    run(
        json!({"command":"grade","input":input,"recipe":{"nodes":[],"output":"source"},"raw_develop":{"exposure_ev":1},"output":output}),
        true,
    );
    let analysis = run(
        json!({"command":"analyze","input":input,"raw_develop":{"exposure_ev":1}}),
        true,
    );
    assert_eq!(analysis["raw_develop"]["exposure_ev"], 1.0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tinge_raw_plan","arguments":{"input":input,"options":{"white_balance":{"type":"temperature","kelvin":4500,"tint_duv":0.002}},"sensor_points":[[16,16]]}}}),
    ] {
        writeln!(stdin, "{msg}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses[1]["result"]["isError"], false);
    assert_eq!(
        responses[1]["result"]["structuredContent"]["white_balance_source"],
        "temperature_kang2002_duv"
    );
    assert_eq!(
        responses[1]["result"]["structuredContent"]["sensor_probes"][0]["native_values"][0],
        1664.0
    );
    let png = dir.path().join("input.png");
    tinge_io::export(
        &tinge_core::Frame::new(1, 1, vec![[0.2; 4]]).unwrap(),
        &png,
        Default::default(),
    )
    .unwrap();
    let project = dir.path().join("not-raw.tinge");
    run(
        json!({"command":"init","input":png,"project":project,"raw_develop":{}}),
        false,
    );
    assert!(!project.exists());
    run(
        json!({"command":"raw_plan","input":input,"options":{"surprise":1}}),
        false,
    );
}
