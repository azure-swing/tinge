mod support;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use vibecolor_core::Frame;
fn request(request: Value, success: bool) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vibecolor"))
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
    let value: Value = serde_json::from_slice(if success {
        &output.stdout
    } else {
        &output.stderr
    })
    .unwrap();
    if success {
        value["data"].clone()
    } else {
        value
    }
}
fn linear_options() -> Value {
    json!({"size":17,"input_encoding":{"type":"standard","space":{"transfer":"linear"}},"output_encoding":{"type":"standard","space":{"transfer":"linear"}},"domain_min":[-1,-1,-1],"domain_max":[4,4,4],"validation_samples":256})
}
#[test]
fn cube_bake_import_freeze_project_revision_and_tamper_protection() {
    let dir = tempfile::tempdir().unwrap();
    let cube = dir.path().join("grade.cube");
    let options = linear_options();
    let recipe = json!({"nodes":[{"id":"balance","op":{"type":"white_balance","gains":[1.1,1,0.9]}}],"output":"balance"});
    let report = request(
        json!({"command":"lut_bake","source":{"type":"recipe","recipe":recipe},"options":options,"output":cube}),
        true,
    );
    assert!(
        report["bake"]["sampled_error"]["max_absolute_rgb"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_f64().unwrap() < 2e-6)
    );
    let original = std::fs::read(&cube).unwrap();
    let inspected = request(json!({"command":"lut_inspect","input":cube}), true);
    assert_eq!(inspected["lut"]["size_3d"], 17);
    let input = dir.path().join("source.exr");
    let project = dir.path().join("grade.vcolor");
    let frame = Frame::new(2, 1, vec![[1.2, -0.05, 0.4, 0.25], [-0.25, 0.3, 1.6, 1.0]]).unwrap();
    vibecolor_io::export(
        &frame,
        &input,
        vibecolor_io::ExportOptions {
            bit_depth: 32,
            space: vibecolor_color::ColorSpace {
                transfer: vibecolor_color::Transfer::Linear,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    request(
        json!({"command":"init","input":input,"project":project}),
        true,
    );
    request(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"upsert_node","node":{"id":"lut","op":{"type":"lut","path":cube,"domain":"linear","interpolation":"tetrahedral"}}},{"type":"set_output","id":"lut"}]}),
        true,
    );
    std::fs::remove_file(&cube).unwrap();
    std::fs::remove_file(input).unwrap();
    let output = dir.path().join("graded.exr");
    request(
        json!({"command":"render","project":project,"output":output}),
        true,
    );
    let rendered = vibecolor_io::load(&output, None).unwrap();
    for (a, b) in rendered.pixels.iter().zip(&frame.pixels) {
        for (c, gain) in [1.1, 1.0, 0.9].iter().enumerate() {
            assert!((a[c] - b[c] * gain).abs() < 3e-6);
        }
        assert_eq!(a[3], b[3]);
    }
    let rebaked = dir.path().join("rebaked.cube");
    let before = std::fs::read(&project).unwrap();
    let report = request(
        json!({"command":"lut_bake","source":{"type":"project","project":project,"revision":1},"options":options,"output":rebaked}),
        true,
    );
    assert_eq!(report["revision"], 1);
    assert_eq!(std::fs::read(&project).unwrap(), before);
    assert_eq!(std::fs::read(rebaked).unwrap(), original);
    let p = vibecolor_project::load(&project).unwrap();
    let vibecolor_core::Operation::Lut { path, .. } = &p.head().unwrap().recipe.nodes[0].op else {
        unreachable!()
    };
    std::fs::write(vibecolor_project::resolve(dir.path(), path), b"tampered").unwrap();
    let rejected = dir.path().join("tamper.cube");
    request(
        json!({"command":"lut_bake","source":{"type":"project","project":project},"output":rejected}),
        false,
    );
    assert!(!rejected.exists());
}
#[test]
fn mcp_bakes_native_log_cdl_and_inspects_strict_cube_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("log.cube");
    let fixture: Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
    ))
    .unwrap();
    let grade = &fixture["cases"][0]["transform"]["grade"];
    let pipeline: Value =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let recipe =
        json!({"nodes":[{"id":"cdl","op":{"type":"ocio_grade","grade":grade}}],"output":"cdl"});
    let call = support::call;
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        call(
            2,
            json!({"command":"lut_bake","source":{"type":"recipe","recipe":recipe,"color_pipeline":pipeline},"output":output,"options":{"size":17,"validation_samples":256,"input_encoding":{"type":"ocio","color_space":"ACEScct"},"output_encoding":{"type":"ocio","color_space":"ACEScct"}}}),
        ),
        call(3, json!({"command":"lut_inspect","input":output})),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_vibecolor"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(stdin);
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let responses: Vec<Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for r in &responses[1..] {
        assert_eq!(r["result"]["isError"], false, "{r}");
    }
    let baked = &responses[1]["result"]["structuredContent"]["bake"];
    assert!(baked["input_transform"]["processor_cache_id"].is_string());
    assert!(baked["output_transform"]["processor_cache_id"].is_string());
    assert!(baked["node_transforms"][0]["ocio_transform"]["processor_cache_id"].is_string());
    assert_eq!(
        responses[2]["result"]["structuredContent"]["lut"]["row_count"],
        4913
    );
    assert_eq!(baked["sampled_error"]["samples"], 256);
}
#[test]
fn invalid_baking_leaves_existing_outputs_and_input_luts_intact() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("existing.cube");
    std::fs::write(&output, b"keep existing output").unwrap();
    let spatial = json!({"nodes":[{"id":"blur","op":{"type":"blur","radius":2}}],"output":"blur"});
    request(
        json!({"command":"lut_bake","source":{"type":"recipe","recipe":spatial},"output":output,"overwrite":true}),
        false,
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"keep existing output");
    let recipe = json!({"nodes":[{"id":"lut","op":{"type":"lut","path":output}}],"output":"lut"});
    request(
        json!({"command":"lut_bake","source":{"type":"recipe","recipe":recipe},"output":output,"overwrite":true}),
        false,
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"keep existing output");
    request(
        json!({"command":"lut_bake","source":{"type":"recipe","recipe":{}},"output":output,"overwrite":true,"options":{"validation_samples":0}}),
        false,
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"keep existing output");
}

#[test]
fn bake_cannot_replace_a_transitive_ocio_display_lut() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.ocio");
    std::fs::write(
        &config,
        include_str!("../../../examples/custom-ocio/config.ocio"),
    )
    .unwrap();
    let lut_dir = dir.path().join("luts/warm");
    std::fs::create_dir_all(&lut_dir).unwrap();
    let lut = lut_dir.join("look.cube");
    let original = include_str!("../../../examples/custom-ocio/luts/warm/look.cube");
    std::fs::write(&lut, original).unwrap();
    let mut pipeline: Value =
        serde_json::from_str(include_str!("../../../examples/custom-ocio/pipeline.json")).unwrap();
    pipeline["config"]["path"] = json!(config);
    let error = request(
        json!({"command":"lut_bake","source":{"type":"recipe","recipe":{},"color_pipeline":pipeline},"options":{"output_encoding":{"type":"display"}},"output":lut,"overwrite":true}),
        false,
    );
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("output must differ from input"),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(lut).unwrap(), original);
}
