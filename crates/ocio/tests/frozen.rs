use std::collections::BTreeMap;
use tinge_ocio::{ConfigSource, Pipeline, Transform, apply_with_context, archive_file};

#[derive(serde::Deserialize)]
struct CustomFixture {
    engine_version: String,
    cases: Vec<CustomCase>,
}
#[derive(serde::Deserialize)]
struct CustomCase {
    context: BTreeMap<String, String>,
    input: Vec<[f32; 4]>,
    expected: Vec<[f32; 4]>,
}
fn check_reference(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    for (actual, expected) in actual.iter().zip(expected) {
        for c in 0..3 {
            assert!(
                (actual[c] - expected[c]).abs() < 3e-5,
                "{actual:?} {expected:?}"
            );
        }
        assert_eq!(actual[3], expected[3]);
    }
}

#[test]
fn binary_ocioz_retains_all_context_choices_without_original_files() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("配置.ocio");
    std::fs::write(
        &config,
        include_str!("../../../examples/custom-ocio/config.ocio"),
    )
    .unwrap();
    for grade in ["neutral", "warm"] {
        std::fs::create_dir_all(dir.path().join(format!("luts/{grade}"))).unwrap();
        std::fs::write(
            dir.path().join(format!("luts/{grade}/look.cube")),
            if grade == "neutral" {
                "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n"
            } else {
                "LUT_1D_SIZE 2\n0 0 0\n1 0.95 0.85\n"
            },
        )
        .unwrap();
    }
    let bytes = archive_file(&config).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    assert!(bytes.contains(&0)); // C-string conversion must not truncate the ZIP.
    let archive = dir.path().join("frozen.ocioz");
    std::fs::write(&archive, &bytes).unwrap();
    let frozen = ConfigSource::Frozen {
        path: archive.clone(),
        hash: blake3::hash(&bytes).to_hex().to_string(),
    };
    let original = ConfigSource::File {
        path: config.clone(),
    };
    let transform = Transform::DisplayView {
        source: "linear".into(),
        display: "Photo sRGB".into(),
        view: "Standard".into(),
        inverse: false,
    };
    let input = [[0.18, 0.4, 0.6, 0.3], [1.0, 0.18, 0.001, 1.0]];
    let mut expected = BTreeMap::new();
    let fixture: CustomFixture =
        serde_json::from_str(include_str!("fixtures/custom-ocio-2.5.2.json")).unwrap();
    assert_eq!(fixture.engine_version, tinge_ocio::version().unwrap());
    for case in &fixture.cases {
        let mut pixels = case.input.clone();
        apply_with_context(&original, &transform, &mut pixels, &case.context).unwrap();
        check_reference(&pixels, &case.expected);
        expected.insert(case.context["GRADE"].clone(), pixels);
    }
    std::fs::remove_file(config).unwrap();
    for grade in ["neutral", "warm"] {
        std::fs::remove_file(dir.path().join(format!("luts/{grade}/look.cube"))).unwrap();
    }
    for grade in ["neutral", "warm"] {
        let mut pixels = input;
        let context = BTreeMap::from([("GRADE".into(), grade.into())]);
        let report = apply_with_context(&frozen, &transform, &mut pixels, &context).unwrap();
        assert_eq!(pixels.as_slice(), expected[grade].as_slice());
        assert!(!report.files.is_empty());
        assert_eq!(report.context["GRADE"], grade);
    }
    let mut default_pixels = input;
    apply_with_context(&frozen, &transform, &mut default_pixels, &BTreeMap::new()).unwrap();
    check_reference(&default_pixels, &expected["neutral"]);
    std::fs::write(archive, b"tampered").unwrap();
    assert!(
        apply_with_context(&frozen, &transform, &mut input.clone(), &BTreeMap::new())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
}

#[test]
fn acescg_config_anchor_converts_to_and_from_canonical_grading_frame() {
    use tinge_color::{Primaries, convert_linear};
    let mut pipeline: Pipeline =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let baseline = pipeline.clone();
    pipeline.working_space = "ACEScg".into();
    pipeline.working_encoding.primaries = Primaries::AcesCg;
    let scene = [[0.18, 0.4, 0.1, 0.2], [4.0, 2.0, -0.1, 1.0]];
    let mut canonical_view = scene;
    let mut aces_view = scene;
    baseline.to_display(&mut canonical_view).unwrap();
    pipeline.to_display(&mut aces_view).unwrap();
    for (a, b) in aces_view.iter().zip(canonical_view) {
        for c in 0..4 {
            assert!((a[c] - b[c]).abs() < 4e-5, "{a:?} {b:?}");
        }
    }
    pipeline.input = tinge_ocio::InputEncoding::Encoded {
        color_space: "ACEScg".into(),
    };
    let mut encoded = scene.map(|p| {
        let rgb = convert_linear([p[0], p[1], p[2]], Primaries::Srgb, Primaries::AcesCg);
        [rgb[0], rgb[1], rgb[2], p[3]]
    });
    pipeline.to_working(&mut encoded).unwrap();
    for (a, b) in encoded.iter().zip(scene) {
        for c in 0..4 {
            assert!((a[c] - b[c]).abs() < 3e-5);
        }
    }
    pipeline.working_encoding.transfer = tinge_color::Transfer::Srgb;
    assert!(pipeline.validate().is_err());
}

#[test]
fn non_archivable_external_dependency_is_rejected_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("external.cube");
    std::fs::write(&source, "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
    let child = dir.path().join("config");
    std::fs::create_dir(&child).unwrap();
    let path = child.join("config.ocio");
    let text = include_str!("../../../examples/custom-ocio/config.ocio").replace(
        "./$GRADE/look.cube",
        &source.to_string_lossy().replace('\\', "/"),
    );
    std::fs::write(&path, text).unwrap();
    let error = archive_file(&path).unwrap_err();
    assert!(error.to_string().contains("external dependency relocation"));
}
