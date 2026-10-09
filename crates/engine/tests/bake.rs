use std::path::Path;
use tinge_color::{ColorSpace, Primaries, Transfer};
use tinge_core::{
    Recipe,
    lut::{CubeLut, LutInterpolation},
};
use tinge_engine::bake::{BakeOptions, LutEncoding, bake, eligible_nodes};
fn linear() -> LutEncoding {
    LutEncoding::Standard {
        space: ColorSpace {
            primaries: Primaries::Srgb,
            transfer: Transfer::Linear,
        },
    }
}
#[test]
fn identity_bake_preserves_hdr_negatives_and_quantifies_serialized_error() {
    let options = BakeOptions {
        size: 9,
        input_encoding: linear(),
        output_encoding: linear(),
        domain_min: [-1.0; 3],
        domain_max: [4.0; 3],
        validation_samples: 512,
        ..Default::default()
    };
    let (text, report) = bake(Recipe::default(), None, Path::new("."), options.clone()).unwrap();
    let (again, _) = bake(Recipe::default(), None, Path::new("."), options).unwrap();
    assert_eq!(text, again);
    assert!(
        report
            .sampled_error
            .max_absolute_rgb
            .iter()
            .all(|v| *v < 1e-6)
    );
    assert!(report.output_above_one_channels > 0 && report.output_below_zero_channels > 0);
    let lut = CubeLut::parse(&text).unwrap();
    assert_eq!(
        lut.sample_with([-1.0; 3], LutInterpolation::Tetrahedral),
        [-1.0; 3]
    );
    assert_eq!(
        lut.sample_with([4.0; 3], LutInterpolation::Tetrahedral),
        [4.0; 3]
    );
    assert_eq!(
        lut.sample_with([8.0; 3], LutInterpolation::Tetrahedral),
        [4.0; 3]
    );
}
#[test]
fn rgb_qualifiers_and_branches_bake_with_measured_resolution_gain() {
    let recipe:Recipe=serde_json::from_value(serde_json::json!({"nodes":[{"id":"e","mask":"range","mix":0.7,"op":{"type":"exposure","stops":1}},{"id":"b","inputs":["source","e"],"op":{"type":"blend","mode":"normal"}}],"output":"b","masks":{"range":{"type":"luma_range","min":0.1,"max":0.5,"softness":0.1}}})).unwrap();
    let coarse = BakeOptions {
        size: 5,
        input_encoding: linear(),
        output_encoding: linear(),
        validation_samples: 1024,
        ..Default::default()
    };
    let fine = BakeOptions {
        size: 33,
        ..coarse.clone()
    };
    let (_, a) = bake(recipe.clone(), None, Path::new("."), coarse).unwrap();
    let (_, b) = bake(recipe, None, Path::new("."), fine).unwrap();
    for c in 0..3 {
        assert!(
            b.sampled_error.max_absolute_rgb[c] < a.sampled_error.max_absolute_rgb[c] * 0.3,
            "{a:?} {b:?}"
        );
    }
    assert_eq!(b.reachable_nodes, ["e", "b"]);
}
#[test]
fn native_cdl_bake_grid_matches_independent_hdr_reference_and_display_path() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../ocio/tests/fixtures/grades-ocio-2.5.2.json"
    ))
    .unwrap();
    let case = &fixture["cases"][0];
    let recipe:Recipe=serde_json::from_value(serde_json::json!({"nodes":[{"id":"cdl","op":{"type":"ocio_grade","grade":case["transform"]["grade"]}}],"output":"cdl"})).unwrap();
    let pipeline: tinge_ocio::Pipeline =
        serde_json::from_str(include_str!("../../../examples/aces2-srgb.json")).unwrap();
    let options = BakeOptions {
        size: 9,
        input_encoding: linear(),
        output_encoding: linear(),
        domain_min: [0.0; 3],
        domain_max: [16.0, 4.0, 0.4],
        validation_samples: 128,
        ..Default::default()
    };
    let (text, report) = bake(
        recipe.clone(),
        Some(pipeline.clone()),
        Path::new("."),
        options.clone(),
    )
    .unwrap();
    assert!(report.node_transforms[0].ocio_transform.is_some());
    let expected: [f32; 4] = serde_json::from_value(case["expected"][3].clone()).unwrap();
    let actual = CubeLut::parse(&text).unwrap().sample([16.0, 4.0, 0.1]);
    for c in 0..3 {
        assert!((actual[c] - expected[c]).abs() < 3e-5 * expected[c].abs().max(1.0));
    }
    let (text, report) = bake(
        recipe,
        Some(pipeline.clone()),
        Path::new("."),
        BakeOptions {
            output_encoding: LutEncoding::Display,
            ..options
        },
    )
    .unwrap();
    let mut display = [expected];
    pipeline.to_display(&mut display).unwrap();
    let actual = CubeLut::parse(&text).unwrap().sample([16.0, 4.0, 0.1]);
    for c in 0..3 {
        assert!((actual[c] - display[0][c]).abs() < 5e-5);
    }
    assert!(
        report
            .output_transform
            .unwrap()
            .processor_cache_id
            .is_some()
    );
}
#[test]
fn spatial_effects_and_masks_reject_and_standard_pq_requires_luminance_units() {
    let spatial:Recipe=serde_json::from_value(serde_json::json!({"nodes":[{"id":"g","op":{"type":"grain","amount":0.1,"seed":7}}],"output":"g"})).unwrap();
    assert!(eligible_nodes(&spatial).is_err());
    let mut unused = spatial;
    unused.output = "source".into();
    assert!(eligible_nodes(&unused).unwrap().is_empty());
    unused.output = "g".into();
    unused.nodes[0].enabled = false;
    assert!(eligible_nodes(&unused).is_ok());
    let spatial:Recipe=serde_json::from_value(serde_json::json!({"nodes":[{"id":"e","mask":"g","op":{"type":"exposure","stops":1}}],"output":"e","masks":{"g":{"type":"linear_gradient","start":[0,0.5],"end":[1,0.5]}}})).unwrap();
    assert!(eligible_nodes(&spatial).is_err());
    let pq = LutEncoding::Standard {
        space: ColorSpace {
            primaries: Primaries::Rec2020,
            transfer: Transfer::Pq,
        },
    };
    let options = BakeOptions {
        size: 9,
        input_encoding: pq.clone(),
        output_encoding: pq,
        validation_samples: 128,
        ..Default::default()
    };
    assert!(bake(Recipe::default(), None, Path::new("."), options.clone()).is_err());
    let (_, report) = bake(
        Recipe::default(),
        None,
        Path::new("."),
        BakeOptions {
            linear_unit_nits: Some(203.0),
            ..options
        },
    )
    .unwrap();
    assert!(
        report
            .sampled_error
            .max_absolute_rgb
            .iter()
            .all(|v| v.is_finite() && *v < 2e-3),
        "{:?}",
        report.sampled_error
    );
}
