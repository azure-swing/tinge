use std::collections::BTreeMap;
use tinge_ocio::{ConfigSource, NodeGrade, Transform, apply_with_context};

#[derive(serde::Deserialize)]
struct Fixture {
    engine_version: String,
    cases: Vec<Case>,
}
#[derive(serde::Deserialize)]
struct Case {
    label: String,
    config: ConfigSource,
    context: BTreeMap<String, String>,
    transform: Transform,
    input: Vec<[f32; 4]>,
    expected: Vec<[f32; 4]>,
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn cdl_matrix_and_looks_match_thirteen_official_native_reference_cases() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/grades-ocio-2.5.2.json")).unwrap();
    assert_eq!(fixture.engine_version, tinge_ocio::version().unwrap());
    assert_eq!(fixture.cases.len(), 13);
    for mut case in fixture.cases {
        if let ConfigSource::File { path } = &mut case.config {
            *path = root().join(&*path);
        }
        let mut pixels = case.input.clone();
        let report =
            apply_with_context(&case.config, &case.transform, &mut pixels, &case.context).unwrap();
        assert!(report.processor_cache_id.is_some());
        if matches!(
            &case.transform,
            Transform::Grade {
                grade: NodeGrade::Look { .. },
                ..
            }
        ) {
            assert!(!report.looks.is_empty(), "{}", case.label);
        }
        for (actual, (expected, original)) in
            pixels.iter().zip(case.expected.iter().zip(&case.input))
        {
            for c in 0..3 {
                assert!(
                    (actual[c] - expected[c]).abs() <= 3e-5 * expected[c].abs().max(1.0),
                    "{}: {actual:?} != {expected:?}",
                    case.label
                );
            }
            assert_eq!(actual[3].to_bits(), original[3].to_bits());
        }
    }
}

#[test]
fn frozen_context_looks_keep_order_and_fallback_without_original_luts() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.ocio");
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
        let path = dir.path().join(format!("luts/{name}"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("look.cube"), lut).unwrap();
    }
    let bytes = tinge_ocio::archive_file(&config).unwrap();
    let archive = dir.path().join("grade.ocioz");
    std::fs::write(&archive, &bytes).unwrap();
    let source = ConfigSource::Frozen {
        path: archive,
        hash: blake3::hash(&bytes).to_hex().to_string(),
    };
    std::fs::remove_file(config).unwrap();
    std::fs::remove_dir_all(dir.path().join("luts")).unwrap();
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/grades-ocio-2.5.2.json")).unwrap();
    for case in fixture
        .cases
        .into_iter()
        .filter(|c| matches!(c.config, ConfigSource::File { .. }))
    {
        let mut pixels = case.input;
        apply_with_context(&source, &case.transform, &mut pixels, &case.context).unwrap();
        for (a, b) in pixels.iter().zip(case.expected) {
            for c in 0..3 {
                assert!(
                    (a[c] - b[c]).abs() < 3e-5 * b[c].abs().max(1.0),
                    "{} {a:?} {b:?}",
                    case.label
                );
            }
        }
    }
}

#[test]
fn invalid_grades_fail_before_mutating_pixels() {
    let config = ConfigSource::Builtin {
        name: "studio-config-v4.0.0_aces-v2.0_ocio-v2.5".into(),
    };
    let mut pixels = [[0.18, 0.4, -0.1, 0.25]];
    let original = pixels;
    let grades = [
        serde_json::json!({"type":"look","looks":"absent"}),
        serde_json::json!({"type":"cdl","color_space":"ACEScct","slope":[0,1,1],"offset":[0,0,0],"power":[1,1,1],"saturation":1,"inverse":true}),
        serde_json::json!({"type":"cdl","color_space":"missing","slope":[1,1,1],"offset":[0,0,0],"power":[1,1,1],"saturation":1}),
        serde_json::json!({"type":"matrix","color_space":"ACEScg","matrix":[[0,0,0],[0,0,0],[0,0,0]],"offset":[0,0,0],"inverse":true}),
    ];
    for grade in grades {
        let grade = serde_json::from_value(grade).unwrap();
        let transform = Transform::Grade {
            source: tinge_ocio::WORKING_SPACE.into(),
            grade,
        };
        assert!(apply_with_context(&config, &transform, &mut pixels, &BTreeMap::new()).is_err());
        assert_eq!(pixels, original);
    }
}
