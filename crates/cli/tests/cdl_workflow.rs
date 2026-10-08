mod support;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn request(value: Value, success: bool) -> Value {
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
        .write_all(value.to_string().as_bytes())
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
fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/cdl-exchange-ocio-2.5.2.json"
    ))
    .unwrap()
}

#[test]
fn imported_cdl_values_and_metadata_survive_source_removal_and_project_export() {
    let temp = tempfile::tempdir().unwrap();
    let ccc = temp.path().join("source.ccc");
    std::fs::write(&ccc, fixture()["files"][1]["xml"].as_str().unwrap()).unwrap();
    let imported = request(
        json!({"command":"cdl_import","input":ccc,"selector":{"type":"id","id":"1"},"color_space":"ACEScct","style":"no_clamp"}),
        true,
    );
    assert_eq!(imported["correction_index"], 0);
    let pipeline: Value =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let input = temp.path().join("source.exr");
    let project = temp.path().join("cdl.vcolor");
    let frame = vibecolor_core::Frame::new(2, 1, vec![[0.18, 0.4, 0.06, 0.25], [16., 4., 0.1, 1.]])
        .unwrap();
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
        json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}),
        true,
    );
    request(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"upsert_node","node":{"id":"imported","op":imported["op"],"mix":0.65}},{"type":"set_output","id":"imported"}]}),
        true,
    );
    let output = temp.path().join("graded.exr");
    request(
        json!({"command":"render","project":project,"output":output}),
        true,
    );
    let original = std::fs::read(&output).unwrap();
    let decoded = vibecolor_io::load(&output, None).unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
    ))
    .unwrap();
    for (pixel, (input, index)) in decoded.pixels.iter().zip(frame.pixels.iter().zip([1, 3])) {
        for c in 0..3 {
            let grade = oracle["cases"][0]["expected"][index][c].as_f64().unwrap() as f32;
            let expected = input[c] * (1. - 0.65) + grade * 0.65;
            assert!((pixel[c] - expected).abs() <= 3e-5 * expected.abs().max(1.));
        }
        assert_eq!(pixel[3], input[3]);
    }
    std::fs::remove_file(ccc).unwrap();
    std::fs::remove_file(input).unwrap();
    request(
        json!({"command":"render","project":project,"output":output,"overwrite":true}),
        true,
    );
    assert_eq!(std::fs::read(output).unwrap(), original);
    let before = std::fs::read(&project).unwrap();
    let mut invalid_op = imported["op"].clone();
    invalid_op["grade"]["exchange"]["source_hash"] = json!("bad");
    request(
        json!({"command":"apply","project":project,"expect_revision":1,"edits":[{"type":"upsert_node","node":{"id":"invalid","op":invalid_op}}]}),
        false,
    );
    assert_eq!(std::fs::read(&project).unwrap(), before);
    let exported = temp.path().join("saved.cdl");
    let report = request(
        json!({"command":"cdl_export","source":{"type":"project","project":project,"revision":1,"nodes":["imported"]},"output":exported}),
        true,
    );
    assert_eq!(std::fs::read(&project).unwrap(), before);
    assert_eq!(
        report["source_context"]["nodes"][0]["mix"]
            .as_f64()
            .unwrap() as f32,
        0.65f32
    );
    assert_eq!(
        report["document"]["corrections"][0]["metadata"]["descriptions"][0],
        "Warm & 雪"
    );
    let reimport = request(
        json!({"command":"cdl_import","input":exported,"color_space":"ACEScct","style":"no_clamp"}),
        true,
    );
    for field in [
        "slope",
        "offset",
        "power",
        "saturation",
        "color_space",
        "style",
        "inverse",
    ] {
        assert_eq!(reimport["grade"][field], imported["grade"][field]);
    }
    assert_eq!(
        reimport["grade"]["exchange"]["collection_metadata"],
        imported["grade"]["exchange"]["collection_metadata"]
    );
}

#[test]
fn mcp_cdl_inspect_select_export_and_reread_keep_descriptions() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input.ccc");
    let output = temp.path().join("exported.ccc");
    std::fs::write(&input, fixture()["files"][1]["xml"].as_str().unwrap()).unwrap();
    let document = vibecolor_ocio::cdl::read(&input).unwrap().0;
    let call = support::call;
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        call(2, json!({"command":"cdl_inspect","input":input})),
        call(
            3,
            json!({"command":"cdl_import","input":input,"selector":{"type":"index","index":1},"color_space":"ACEScct","style":"asc","inverse":true}),
        ),
        call(
            4,
            json!({"command":"cdl_export","source":{"type":"document","document":document},"output":output}),
        ),
        call(5, json!({"command":"cdl_inspect","input":output})),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_vibecolor"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for m in messages {
        writeln!(stdin, "{m}").unwrap();
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
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    for r in &responses[1..] {
        assert_eq!(r["result"]["isError"], false, "{r}");
    }
    assert_eq!(
        responses[2]["result"]["structuredContent"]["grade"]["exchange"]["correction_id"],
        "cool&\"雪"
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["grade"]["inverse"],
        true
    );
    assert_eq!(
        responses[1]["result"]["structuredContent"]["document"],
        responses[4]["result"]["structuredContent"]["document"]
    );
}

#[test]
fn malformed_cdl_and_missing_interpretation_fail_without_overwriting_or_committing() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input.ccc");
    std::fs::write(&input, fixture()["files"][1]["xml"].as_str().unwrap()).unwrap();
    request(
        json!({"command":"cdl_import","input":input,"color_space":"ACEScct","style":"asc"}),
        false,
    );
    request(
        json!({"command":"cdl_import","input":input,"selector":{"type":"id","id":"missing"},"color_space":"ACEScct","style":"asc"}),
        false,
    );
    request(
        json!({"command":"cdl_import","input":input,"selector":{"type":"index","index":0},"color_space":"ACEScct"}),
        false,
    );
    let output = temp.path().join("keep.cc");
    std::fs::write(&output, b"keep output").unwrap();
    let invalid = json!({"format":"cc","corrections":[{"slope":[1,1,1],"offset":[0,0,0],"power":[-1,1,1],"saturation":1}]});
    request(
        json!({"command":"cdl_export","source":{"type":"document","document":invalid},"output":output,"overwrite":true}),
        false,
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"keep output");
    let document = vibecolor_ocio::cdl::read(&input).unwrap().0;
    request(
        json!({"command":"cdl_export","source":{"type":"document","document":document},"output":output,"overwrite":true}),
        false,
    );
    assert_eq!(std::fs::read(output).unwrap(), b"keep output");
    let cli = Command::new(env!("CARGO_BIN_EXE_vibecolor"))
        .args([
            "cdl-import",
            input.to_str().unwrap(),
            "--color-space",
            "ACEScct",
        ])
        .output()
        .unwrap();
    assert!(!cli.status.success());
    assert!(String::from_utf8_lossy(&cli.stderr).contains("--style"));
}
