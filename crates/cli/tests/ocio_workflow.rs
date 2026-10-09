mod support;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use tinge_core::Frame;

fn run(request: Value) -> Value {
    let mut process = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .args(["run", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let result = process.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice::<Value>(&result.stdout).unwrap()["data"].clone()
}

fn rejected(request: Value) -> Value {
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
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let response: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(response["ok"], false);
    response
}

#[test]
fn native_grade_mcp_schema_numeric_oracle_and_persistent_node_cache() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scene.exr");
    let output = dir.path().join("grade.exr");
    let frame = Frame::new(1, 1, vec![[0.18, 0.4, 0.06, 0.25]]).unwrap();
    tinge_io::export(
        &frame,
        &input,
        tinge_io::ExportOptions {
            bit_depth: 32,
            space: tinge_color::ColorSpace {
                primaries: tinge_color::Primaries::Srgb,
                transfer: tinge_color::Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
    ))
    .unwrap();
    let case = &fixture["cases"][0];
    let recipe = json!({"nodes":[{"id":"cdl","op":{"type":"ocio_grade","grade":case["transform"]["grade"]}}],"output":"cdl"});
    let pipeline: Value =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let call = support::call;
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"ocio-grade-test","version":"1"}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        call(
            3,
            json!({"command":"validate","recipe":recipe,"color_pipeline":pipeline}),
        ),
        call(
            4,
            json!({"command":"ocio_transform","config":case["config"],"transform":case["transform"],"pixels":case["input"]}),
        ),
        call(
            5,
            json!({"command":"grade","input":input,"output":output,"recipe":recipe,"color_pipeline":pipeline}),
        ),
        call(
            6,
            json!({"command":"grade","input":input,"output":output,"recipe":recipe,"color_pipeline":pipeline,"overwrite":true}),
        ),
        call(7, json!({"command":"schema","target":"op:ocio_grade"})),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .args(["--progress", "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
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
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 7);
    assert!(
        responses[6]["result"]["structuredContent"]
            .to_string()
            .contains("no_clamp")
    );
    for r in &responses[2..] {
        assert_ne!(r["result"]["isError"], true, "{r}");
    }
    let numeric = &responses[3]["result"]["structuredContent"]["pixels"];
    let actual: Vec<[f32; 4]> = serde_json::from_value(numeric.clone()).unwrap();
    let expected: Vec<[f32; 4]> = serde_json::from_value(case["expected"].clone()).unwrap();
    for (a, b) in actual.iter().zip(expected) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 3e-5 * b[c].abs().max(1.0));
        }
        assert_eq!(a[3], b[3]);
    }
    let events: Vec<Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["data"]["cached"], false);
    assert_eq!(events[1]["data"]["cached"], true);
    assert!(events[1]["data"]["ocio_transform"]["processor_cache_id"].is_string());
    let pixel = tinge_io::load(&output, None).unwrap().pixels[0];
    let expected: [f32; 4] = serde_json::from_value(case["expected"][1].clone()).unwrap();
    for c in 0..4 {
        assert!((pixel[c] - expected[c]).abs() < 3e-5);
    }
}

#[test]
fn log_domain_cdl_cli_grade_project_float_and_preview_match_independent_reference() {
    let dir = tempfile::tempdir().unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
    ))
    .unwrap();
    let case = &fixture["cases"][0];
    let mut pixels: Vec<[f32; 4]> = serde_json::from_value(case["input"].clone()).unwrap();
    // EXR has associated alpha: use nonzero alpha for the HDR reference pixel.
    pixels[3][3] = 1.0;
    let frame = Frame::new(pixels.len() as u32, 1, pixels.clone()).unwrap();
    let input = dir.path().join("source.exr");
    tinge_io::export(
        &frame,
        &input,
        tinge_io::ExportOptions {
            bit_depth: 32,
            space: tinge_color::ColorSpace {
                primaries: tinge_color::Primaries::Srgb,
                transfer: tinge_color::Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let pipeline: Value =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let recipe = json!({"nodes":[{"id":"cdl","op":{"type":"ocio_grade","grade":case["transform"]["grade"]}}],"output":"cdl"});
    let validated = run(json!({"command":"validate","recipe":recipe,"color_pipeline":pipeline}));
    assert!(validated["ocio_nodes"]["cdl"]["processor_cache_id"].is_string());
    rejected(json!({"command":"validate","recipe":recipe}));
    let grade = dir.path().join("grade.exr");
    run(
        json!({"command":"grade","input":input,"recipe":recipe,"output":grade,"color_pipeline":pipeline}),
    );
    let project = dir.path().join("grade.tinge");
    run(json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}));
    run(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"replace_recipe","recipe":recipe}]}),
    );
    std::fs::remove_file(input).unwrap();
    let render = dir.path().join("render.exr");
    run(json!({"command":"render","project":project,"output":render}));
    let output = tinge_io::load(&render, None).unwrap();
    assert_eq!(output.pixels, tinge_io::load(&grade, None).unwrap().pixels);
    let expected: Vec<[f32; 4]> = serde_json::from_value(case["expected"].clone()).unwrap();
    for (i, (a, b)) in output.pixels.iter().zip(expected).enumerate() {
        for c in 0..3 {
            assert!(
                (a[c] - b[c]).abs() < 3e-5 * b[c].abs().max(1.0),
                "{a:?} {b:?}"
            );
        }
        assert_eq!(a[3], pixels[i][3]);
    }
    let output = dir.path().join("preview.png");
    run(json!({"command":"preview","project":project,"output":output}));
    let pipeline: tinge_ocio::Pipeline = serde_json::from_value(pipeline).unwrap();
    let mut expected = tinge_io::load(&render, None).unwrap().pixels;
    pipeline.to_display(&mut expected).unwrap();
    let actual = tinge_io::load_signal(&output).unwrap();
    for (a, b) in actual.pixels.iter().zip(expected) {
        for c in 0..4 {
            assert!((a[c] - b[c].clamp(0.0, 1.0)).abs() <= 0.5 / 255.0 + 3e-5);
        }
    }
}

#[test]
fn look_nodes_freeze_context_history_and_reject_invalid_transactions_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    std::fs::create_dir_all(&original).unwrap();
    let config = original.join("config.ocio");
    std::fs::write(
        &config,
        include_str!("../../../examples/node-ocio/config.ocio"),
    )
    .unwrap();
    for (name, lut) in [
        (
            "neutral",
            include_str!("../../../examples/node-ocio/luts/neutral/look.cube"),
        ),
        (
            "warm",
            include_str!("../../../examples/node-ocio/luts/warm/look.cube"),
        ),
    ] {
        let folder = original.join(format!("luts/{name}"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("look.cube"), lut).unwrap();
    }
    let input = original.join("source.exr");
    let frame = Frame::new(1, 1, vec![[0.18, 0.4, 0.06, 0.25]]).unwrap();
    tinge_io::export(
        &frame,
        &input,
        tinge_io::ExportOptions {
            bit_depth: 32,
            space: tinge_color::ColorSpace {
                primaries: tinge_color::Primaries::Srgb,
                transfer: tinge_color::Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let project = dir.path().join("look.tinge");
    let pipeline = json!({"config":{"type":"file","path":config},"working_space":"linear","display_name":"Photo sRGB","view":"Standard","context":{"GRADE":"warm"}});
    run(json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}));
    let recipe = json!({"nodes":[{"id":"look","op":{"type":"ocio_grade","grade":{"type":"look","looks":"+ContextGrade,+SoftContrast"}}}],"output":"look"});
    run(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"replace_recipe","recipe":recipe}]}),
    );
    let warm_project = tinge_project::load(&project).unwrap();
    let hash = warm_project.head().unwrap().recipe_hash.clone();
    std::fs::remove_dir_all(original).unwrap();
    let warm = dir.path().join("warm.png");
    run(json!({"command":"render","project":project,"output":warm}));
    let float = dir.path().join("warm.exr");
    run(json!({"command":"render","project":project,"output":float}));
    let reference: Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
    ))
    .unwrap();
    let expected: [f32; 4] =
        serde_json::from_value(reference["cases"][11]["expected"][1].clone()).unwrap();
    let actual = tinge_io::load(&float, None).unwrap().pixels[0];
    for c in 0..4 {
        assert!((actual[c] - expected[c]).abs() < 3e-5);
    }
    let before = std::fs::read(&project).unwrap();
    for edit in [
        json!({"type":"set_color_pipeline","pipeline":null}),
        json!({"type":"upsert_node","node":{"id":"invalid","enabled":false,"op":{"type":"ocio_grade","grade":{"type":"look","looks":"missing"}}}}),
    ] {
        rejected(json!({"command":"apply","project":project,"expect_revision":1,"edits":[edit]}));
        assert_eq!(std::fs::read(&project).unwrap(), before);
    }
    let mut neutral = warm_project.head().unwrap().color_pipeline.clone().unwrap();
    neutral.context.insert("GRADE".into(), "neutral".into());
    run(
        json!({"command":"apply","project":project,"expect_revision":1,"edits":[{"type":"set_color_pipeline","pipeline":neutral}]}),
    );
    let p = tinge_project::load(&project).unwrap();
    assert_ne!(p.head().unwrap().recipe_hash, hash);
    let output = dir.path().join("neutral.png");
    run(json!({"command":"render","project":project,"output":output}));
    assert_ne!(
        std::fs::read(&warm).unwrap(),
        std::fs::read(output).unwrap()
    );
    run(json!({"command":"restore","project":project,"expect_revision":2,"revision":1}));
    let output = dir.path().join("restored.png");
    run(json!({"command":"render","project":project,"output":output}));
    assert_eq!(std::fs::read(warm).unwrap(), std::fs::read(output).unwrap());
}

#[test]
fn ocio_grade_project_preview_and_float_export_use_correct_domains() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.exr");
    let project = dir.path().join("aces.tinge");
    let original = Frame::new(
        3,
        1,
        vec![
            [0.18, 0.18, 0.18, 0.25],
            [16.0, 4.0, 0.1, 1.0],
            [-0.05, 0.2, 0.4, 1.0],
        ],
    )
    .unwrap();
    let float_options = tinge_io::ExportOptions {
        bit_depth: 32,
        space: tinge_color::ColorSpace {
            primaries: tinge_color::Primaries::Srgb,
            transfer: tinge_color::Transfer::Linear,
        },
        ..Default::default()
    };
    tinge_io::export(&original, &input, float_options).unwrap();
    let pipeline: tinge_ocio::Pipeline =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let mut expected = original.pixels.clone();
    pipeline.to_display(&mut expected).unwrap();
    run(json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}));
    std::fs::remove_file(&input).unwrap(); // The frozen input must suffice.
    let preview = dir.path().join("preview.png");
    let report = run(json!({"command":"preview","project":project,"output":preview}));
    assert_eq!(report["ocio_transform"]["engine_version"], "2.5.2");
    let pixels = tinge_io::load_signal(&preview).unwrap().pixels;
    for (actual, expected) in pixels.iter().zip(expected) {
        for c in 0..4 {
            assert!((actual[c] - expected[c].clamp(0.0, 1.0)).abs() <= 0.5 / 255.0 + 1e-5);
        }
    }
    let scene = dir.path().join("scene.exr");
    let report = run(json!({"command":"render","project":project,"output":scene}));
    assert!(report["ocio_transform"].is_null());
    let scene = tinge_io::load(&scene, None).unwrap();
    assert_eq!(scene.pixels, original.pixels); // No ODT is baked into EXR.
    let comparison = dir.path().join("comparison.png");
    run(json!({"command":"compare","project":project,"output":comparison}));
    let comparison = tinge_io::load_signal(&comparison).unwrap();
    assert_eq!(&comparison.pixels[..3], &comparison.pixels[3..]);
}

#[test]
fn encoded_input_is_decoded_once_and_grade_matches_project_render() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("encoded.png");
    let original = Frame::new(1, 1, vec![[0.18, 0.4, 0.05, 0.6]]).unwrap();
    tinge_io::export(&original, &input, Default::default()).unwrap();
    let mut pipeline: tinge_ocio::Pipeline =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    pipeline.input = tinge_ocio::InputEncoding::Encoded {
        color_space: "sRGB Encoded Rec.709 (sRGB)".into(),
    };
    let decoded = tinge_engine::load_source(&input, None, Some(&pipeline)).unwrap();
    for c in 0..4 {
        assert!((decoded.pixels[0][c] - original.pixels[0][c]).abs() < 3e-5);
    }
    let recipe = json!({"nodes":[{"id":"exposure","op":{"type":"exposure","stops":1.0}}],"output":"exposure"});
    let output = dir.path().join("grade.png");
    run(
        json!({"command":"grade","input":input,"output":output,"recipe":recipe,"color_pipeline":pipeline}),
    );
    let project = dir.path().join("encoded.tinge");
    run(json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}));
    run(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"replace_recipe","recipe":recipe}]}),
    );
    let rendered = dir.path().join("render.png");
    run(json!({"command":"render","project":project,"output":rendered}));
    assert_eq!(
        std::fs::read(output).unwrap(),
        std::fs::read(rendered).unwrap()
    );
}

#[test]
fn hdr_and_p3_projects_export_tagged_display_outputs_and_sdr_previews() {
    use tinge_color::{ColorSpace, Primaries, Transfer};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scene.exr");
    let frame = Frame::new(
        3,
        1,
        vec![[0.18; 4], [8.0, 2.0, 0.1, 1.0], [-0.1, 0.5, 1.0, 0.8]],
    )
    .unwrap();
    tinge_io::export(
        &frame,
        &input,
        tinge_io::ExportOptions {
            bit_depth: 32,
            space: ColorSpace {
                primaries: Primaries::Srgb,
                transfer: Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    for (name, fixture, primaries, transfer) in [
        (
            "hdr",
            include_str!("../../../examples/aces2-hdr1000.json"),
            Primaries::Rec2020,
            Transfer::Pq,
        ),
        (
            "p3",
            include_str!("../../../examples/aces2-display-p3.json"),
            Primaries::DisplayP3,
            Transfer::Srgb,
        ),
    ] {
        let pipeline: tinge_ocio::Pipeline = serde_json::from_str(fixture).unwrap();
        let project = dir.path().join(format!("{name}.tinge"));
        run(json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}));
        let output = dir.path().join(format!("{name}.png"));
        let report = run(json!({"command":"render","project":project,"output":output}));
        assert_eq!(
            report["export"]["color_space"],
            json!({"primaries":primaries,"transfer":transfer})
        );
        let metadata = tinge_io::inspect(&output).unwrap().color_metadata;
        if name == "hdr" {
            assert_eq!(metadata.cicp, Some([9, 16, 0, 1]));
        } else {
            assert!(metadata.icc_hash.is_some());
        }
        let mut expected = frame.pixels.clone();
        pipeline.to_display(&mut expected).unwrap();
        for (actual, expected) in tinge_io::load_signal(&output)
            .unwrap()
            .pixels
            .iter()
            .zip(expected)
        {
            for c in 0..4 {
                assert!((actual[c] - expected[c].clamp(0.0, 1.0)).abs() < 1.0 / 65535.0);
            }
        }
        let preview = dir.path().join(format!("{name}-preview.png"));
        let report = run(json!({"command":"preview","project":project,"output":preview}));
        assert_eq!(
            report["preview_pipeline"]["view"],
            pipeline.preview_view.as_ref().unwrap().as_str()
        );
        let mut expected = frame.pixels.clone();
        pipeline
            .for_preview()
            .unwrap()
            .to_display(&mut expected)
            .unwrap();
        for (actual, expected) in tinge_io::load_signal(&preview)
            .unwrap()
            .pixels
            .iter()
            .zip(expected)
        {
            for c in 0..4 {
                assert!((actual[c] - expected[c].clamp(0.0, 1.0)).abs() < 0.5 / 255.0 + 1e-5);
            }
        }
        let linear = dir.path().join(format!("{name}-acescg.exr"));
        run(
            json!({"command":"render","project":project,"output":linear,"output_space":{"primaries":"aces_cg","transfer":"linear"}}),
        );
        assert_eq!(
            tinge_io::inspect(&linear)
                .unwrap()
                .color_metadata
                .color_interop_id
                .as_deref(),
            Some("lin_ap1_scene")
        );
        let roundtrip = tinge_io::load(&linear, None).unwrap();
        for (a, b) in roundtrip.pixels.iter().zip(&frame.pixels) {
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 3e-5);
            }
        }
        let invalid = dir.path().join(format!("{name}-mismatch.png"));
        let result = Command::new(env!("CARGO_BIN_EXE_tinge"))
            .args([
                "render",
                project.to_str().unwrap(),
                "--output",
                invalid.to_str().unwrap(),
                "--output-space",
                r#"{"primaries":"srgb","transfer":"srgb"}"#,
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!invalid.exists());
    }
}

#[test]
fn custom_config_projects_move_switch_context_restore_and_reject_tampered_cache() {
    use tinge_color::{ColorSpace, Primaries, Transfer};
    use tinge_ocio::{ConfigSource, PipelineConfig};
    let dir = tempfile::tempdir().unwrap();
    let studio = dir.path().join("studio");
    std::fs::create_dir(&studio).unwrap();
    let config = studio.join("config.ocio");
    std::fs::write(
        &config,
        include_str!("../../../examples/custom-ocio/config.ocio"),
    )
    .unwrap();
    for (name, contents) in [
        (
            "neutral",
            include_str!("../../../examples/custom-ocio/luts/neutral/look.cube"),
        ),
        (
            "warm",
            include_str!("../../../examples/custom-ocio/luts/warm/look.cube"),
        ),
    ] {
        std::fs::create_dir_all(studio.join(format!("luts/{name}"))).unwrap();
        std::fs::write(studio.join(format!("luts/{name}/look.cube")), contents).unwrap();
    }
    let input = dir.path().join("scene.exr");
    let frame = Frame::new(2, 1, vec![[0.18, 0.4, 0.6, 1.0], [0.8, 0.1, 0.02, 1.0]]).unwrap();
    tinge_io::export(
        &frame,
        &input,
        tinge_io::ExportOptions {
            bit_depth: 32,
            space: ColorSpace {
                primaries: Primaries::Srgb,
                transfer: Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let mut pipeline: tinge_ocio::Pipeline =
        serde_json::from_str(include_str!("../../../examples/custom-ocio/pipeline.json")).unwrap();
    pipeline.config = PipelineConfig::Source(ConfigSource::File {
        path: config.clone(),
    });
    let parent = dir.path().join("original");
    std::fs::create_dir(&parent).unwrap();
    let project = parent.join("custom.tinge");
    run(json!({"command":"init","input":input,"project":project,"color_pipeline":pipeline}));
    let warm = dir.path().join("warm.png");
    run(json!({"command":"render","project":project,"output":warm}));
    let original = tinge_project::load(&project).unwrap();
    let hash = original.head().unwrap().recipe_hash.clone();
    let frozen = original.head().unwrap().color_pipeline.clone().unwrap();
    assert!(matches!(
        frozen.config,
        PipelineConfig::Source(ConfigSource::Frozen { .. })
    ));
    std::fs::remove_file(config).unwrap();
    std::fs::remove_file(input).unwrap();
    for name in ["neutral", "warm"] {
        std::fs::remove_file(studio.join(format!("luts/{name}/look.cube"))).unwrap();
    }
    let moved = dir.path().join("moved");
    std::fs::rename(parent, &moved).unwrap();
    let project = moved.join("custom.tinge");
    let output = dir.path().join("moved.png");
    // A poisoned process environment must neither override authored defaults
    // nor the pipeline's explicit context.
    let request = json!({"command":"render","project":project,"output":output});
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .args(["run", "-"])
        .env("GRADE", "missing-ambient-choice")
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
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read(&warm).unwrap(),
        std::fs::read(output).unwrap()
    );
    let request = json!({"command":"ocio_transform","config":frozen.resolved_at(&moved).source(),
        "transform":{"type":"display_view","source":"linear","display":"Photo sRGB","view":"Standard"},
        "pixels":[[0.18,0.4,0.6,0.3]]});
    let mut child = Command::new(env!("CARGO_BIN_EXE_tinge"))
        .args(["run", "-"])
        .env("GRADE", "missing-ambient-choice")
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
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let response: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(response["data"]["report"]["context"]["GRADE"], "neutral");
    let reference: Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/custom-ocio-2.5.2.json"
    ))
    .unwrap();
    for c in 0..4 {
        assert!(
            (response["data"]["pixels"][0][c].as_f64().unwrap()
                - reference["cases"][0]["expected"][0][c].as_f64().unwrap())
            .abs()
                < 3e-5
        );
    }
    let preview = dir.path().join("preview.png");
    run(json!({"command":"preview","project":project,"output":preview}));
    let mut neutral = frozen.clone();
    neutral.context.insert("GRADE".into(), "neutral".into());
    run(
        json!({"command":"apply","project":project,"expect_revision":0,"edits":[{"type":"set_color_pipeline","pipeline":neutral}]}),
    );
    let p = tinge_project::load(&project).unwrap();
    assert_ne!(p.head().unwrap().recipe_hash, hash);
    let output = dir.path().join("neutral.png");
    run(json!({"command":"render","project":project,"output":output}));
    assert_ne!(
        std::fs::read(&warm).unwrap(),
        std::fs::read(output).unwrap()
    );
    let before = std::fs::read(&project).unwrap();
    let mut missing = frozen.clone();
    missing.context.insert("GRADE".into(), "missing".into());
    assert!(
        tinge_project::transaction(
            &project,
            1,
            vec![tinge_project::Edit::SetColorPipeline {
                pipeline: Some(missing)
            }],
            "invalid".into(),
            &moved
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&project).unwrap(), before);
    run(json!({"command":"restore","project":project,"expect_revision":1,"revision":0}));
    let output = dir.path().join("restored.png");
    run(json!({"command":"render","project":project,"output":output}));
    assert_eq!(
        std::fs::read(&warm).unwrap(),
        std::fs::read(output).unwrap()
    );
    // Revalidate the package even when the engine already cached decoded pixels.
    let p = tinge_project::load(&project).unwrap();
    let mut engine = tinge_engine::Engine::new();
    engine
        .project_render(
            &project,
            &p,
            None,
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
    if let PipelineConfig::Source(ConfigSource::Frozen { path, .. }) =
        &p.head().unwrap().color_pipeline.as_ref().unwrap().config
    {
        std::fs::write(
            tinge_project::resolve(&moved, &path.to_string_lossy()),
            b"tampered",
        )
        .unwrap();
    } else {
        panic!("expected frozen package");
    }
    let error = engine
        .project_render(
            &project,
            &p,
            None,
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
    assert!(error.to_string().contains("hash mismatch"), "{error}");
}
