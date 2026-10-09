use serde::Deserialize;
use tinge_core::{
    Operation,
    lut::{CubeLut, LutInterpolation},
};
#[derive(Deserialize)]
struct Fixture {
    engine_version: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    label: String,
    cube: String,
    interpolation: LutInterpolation,
    input: Vec<[f32; 4]>,
    expected: Vec<[f32; 4]>,
}
#[test]
fn cube_formats_and_both_interpolations_match_official_ocio() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/cube-ocio-2.5.2.json")).unwrap();
    assert_eq!(fixture.engine_version, "2.5.2");
    assert_eq!(fixture.cases.len(), 8);
    for case in fixture.cases {
        let lut = CubeLut::parse(&case.cube).unwrap();
        for (input, expected) in case.input.iter().zip(&case.expected) {
            let actual = lut.sample_with([input[0], input[1], input[2]], case.interpolation);
            for c in 0..3 {
                assert!(
                    (actual[c] - expected[c]).abs() < 3e-6 * expected[c].abs().max(1.0),
                    "{} {:?} {input:?}: {actual:?} {expected:?}",
                    case.label,
                    case.interpolation
                );
            }
        }
    }
}
#[test]
fn malformed_ambiguous_and_nonfinite_cube_headers_are_rejected() {
    for text in [
        "LUT_1D_SIZE 2\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n",
        "LUT_1D_SIZE 2\n0 0 0\nDOMAIN_MAX 1 1 1\n1 1 1\n",
        "LUT_1D_SIZE 2\nLUT_3D_INPUT_RANGE 0 1\n0 0 0\n1 1 1\n",
        "LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0\nLUT_1D_INPUT_RANGE 0 1\n0 0 0\n1 1 1\n",
        "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 1 0\n0 0 0\n1 1 1\n",
        "LUT_1D_SIZE 2\n0 0 0\nNaN 1 1\n",
        "LUT_1D_SIZE 2\nDOMAIN_MAX inf 1 1\n0 0 0\n1 1 1\n",
        "TITLE \"unclosed\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n",
    ] {
        assert!(CubeLut::parse(text).is_err(), "{text}");
    }
}
#[test]
fn exported_cube_preserves_hdr_negative_tiny_values_and_legacy_operation_serialization() {
    let values = vec![[1e-12, -2.0, 16.0]; 8];
    let lut = CubeLut::from_3d(2, [-1.0; 3], [16.0; 3], values, "precision".into()).unwrap();
    let parsed = CubeLut::parse(&lut.write_3d().unwrap()).unwrap();
    assert_eq!(parsed.sample([-1.0; 3]), [1e-12, -2.0, 16.0]);
    assert_eq!(parsed.sample([16.0; 3]), [1e-12, -2.0, 16.0]);
    let old: Operation =
        serde_json::from_str(r#"{"type":"lut","path":"look.cube","domain":"srgb"}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"type":"lut","path":"look.cube","domain":"srgb"}"#
    );
    let new: Operation = serde_json::from_str(
        r#"{"type":"lut","path":"look.cube","domain":"srgb","interpolation":"tetrahedral"}"#,
    )
    .unwrap();
    assert!(serde_json::to_string(&new).unwrap().contains("tetrahedral"));
}
