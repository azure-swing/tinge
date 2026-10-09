use serde::Deserialize;
use tinge_ocio::{ConfigSource, Transform, apply};

#[derive(Deserialize)]
struct Fixture {
    engine_version: String,
    config: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    label: String,
    transform: Transform,
    input: Vec<[f32; 4]>,
    expected: Vec<[f32; 4]>,
}

#[test]
fn native_cpu_matches_official_python_ocio_reference() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/ocio-2.5.2.json")).unwrap();
    assert_eq!(tinge_ocio::version().unwrap(), fixture.engine_version);
    let config = ConfigSource::Builtin {
        name: fixture.config,
    };
    for case in fixture.cases {
        let mut output = case.input.clone();
        let report = apply(&config, &case.transform, &mut output).unwrap();
        assert!(report.processor_cache_id.is_some());
        for (i, (actual, expected)) in output.iter().zip(&case.expected).enumerate() {
            for channel in 0..3 {
                let tolerance = 3e-5 * expected[channel].abs().max(1.0);
                assert!(
                    (actual[channel] - expected[channel]).abs() <= tolerance,
                    "{} pixel {i} channel {channel}: {actual:?} != {expected:?}",
                    case.label
                );
            }
            assert_eq!(actual[3].to_bits(), case.input[i][3].to_bits());
        }
    }
}

#[test]
fn config_errors_are_errors_and_aliases_are_rejected() {
    assert!(
        tinge_ocio::inspect(&ConfigSource::Builtin {
            name: "default".into()
        })
        .is_err()
    );
    assert!(
        tinge_ocio::inspect(&ConfigSource::File {
            path: "missing-config.ocio".into()
        })
        .is_err()
    );
    let config = ConfigSource::Builtin {
        name: "studio-config-v4.0.0_aces-v2.0_ocio-v2.5".into(),
    };
    let mut pixels = [[0.18, 0.18, 0.18, 1.0]];
    assert!(
        apply(
            &config,
            &Transform::ColorSpace {
                source: "unknown".into(),
                destination: "ACEScg".into()
            },
            &mut pixels
        )
        .is_err()
    );
    assert_eq!(pixels[0][0], 0.18);
}

#[test]
fn external_config_resolves_relative_lut_and_reports_missing_asset() {
    let dir = tempfile::tempdir().unwrap();
    let lut_dir = dir.path().join("luts");
    std::fs::create_dir(&lut_dir).unwrap();
    let path = dir.path().join("配置.ocio");
    std::fs::write(
        &path,
        r#"ocio_profile_version: 2
search_path: luts
roles:
  scene_linear: linear
file_rules:
  - !<Rule> {name: Default, colorspace: linear}
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: encoded}
colorspaces:
  - !<ColorSpace>
    name: linear
    family: test
    bitdepth: 32f
    isdata: false
    allocation: uniform
  - !<ColorSpace>
    name: encoded
    family: test
    bitdepth: 32f
    isdata: false
    allocation: uniform
    from_scene_reference: !<FileTransform> {src: half.cube, interpolation: linear}
"#,
    )
    .unwrap();
    let lut = lut_dir.join("half.cube");
    std::fs::write(&lut, "LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n").unwrap();
    let source = ConfigSource::File { path };
    let transform = Transform::ColorSpace {
        source: "linear".into(),
        destination: "encoded".into(),
    };
    let mut pixel = [[0.6, 0.4, 0.2, 0.3]];
    apply(&source, &transform, &mut pixel).unwrap();
    assert!((pixel[0][0] - 0.3).abs() < 1e-6);
    assert_eq!(pixel[0][3], 0.3);
    std::fs::write(&lut, "LUT_1D_SIZE 2\n0 0 0\n0.25 0.25 0.25\n").unwrap();
    pixel[0] = [0.6, 0.4, 0.2, 0.3];
    apply(&source, &transform, &mut pixel).unwrap();
    assert!((pixel[0][0] - 0.15).abs() < 1e-6);
    std::fs::remove_file(lut).unwrap();
    assert!(apply(&source, &transform, &mut pixel).is_err());
}
