use serde_json::json;
use std::sync::atomic::AtomicBool;
use tinge_color::from_display;
use tinge_core::{
    Frame,
    cutout::{self, CutoutOptions},
    recipe::Recipe,
};

fn run(frame: &Frame, options: serde_json::Value) -> (Frame, cutout::CutoutReport) {
    let options: CutoutOptions = serde_json::from_value(options).unwrap();
    cutout::run(
        frame,
        &options,
        &|_, _, _| panic!("unexpected external asset"),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn pixel(rgb: [f32; 3], alpha: f32) -> [f32; 4] {
    let c = from_display(rgb);
    [c[0], c[1], c[2], alpha]
}
#[test]
fn encoded_color_key_samples_preserve_existing_alpha_and_matte_values() {
    let frame = Frame::new(
        3,
        1,
        vec![
            pixel([0.0, 1.0, 0.0], 1.0),
            pixel([1.0, 0.0, 0.0], 0.4),
            pixel([0.0, 1.0, 0.0], 0.2),
        ],
    )
    .unwrap();
    let (out, report) = run(
        &frame,
        json!({"selection":{"type":"color","samples":[[0.0,0.0]],"tolerance":0.01,"softness":0.1}}),
    );
    assert_eq!(out.pixels, vec![[0.0; 4], frame.pixels[1], [0.0; 4]]);
    assert_eq!((report.transparent_pixels, report.partial_pixels), (2, 1));
    assert!(!report.model_weights_required);
    let (matte, _) = run(
        &frame,
        json!({"selection":{"type":"color","color":[0,1,0]},"output":"matte"}),
    );
    assert_eq!(
        matte.pixels,
        vec![[0., 0., 0., 1.], [0.4, 0.4, 0.4, 1.], [0., 0., 0., 1.]]
    );
}
#[test]
fn grabcut_finds_two_color_subject_and_strokes_are_hard_constraints() {
    let (w, h) = (32, 32);
    let pixels = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if (8..24).contains(&x) && (8..24).contains(&y) {
                pixel(
                    if x < 16 {
                        [0.85, 0.1, 0.15]
                    } else {
                        [0.8, 0.3, 0.1]
                    },
                    1.0,
                )
            } else {
                pixel(
                    if y < 16 {
                        [0.1, 0.5, 0.8]
                    } else {
                        [0.05, 0.35, 0.6]
                    },
                    1.0,
                )
            }
        })
        .collect();
    let frame = Frame::new(w, h, pixels).unwrap();
    for with_strokes in [false, true] {
        let mut selection = json!({"type":"grab_cut","rect":[0.15,0.15,0.85,0.85],"max_edge":64});
        if with_strokes {
            selection["foreground"] = json!([{"points":[[0.4,0.4],[0.6,0.6]],"radius":0.02}]);
            selection["background"] = json!([{"points":[[0.2,0.2]],"radius":0.01}]);
        }
        let (out, report) = run(&frame, json!({"selection":selection}));
        assert_eq!(report.segmentation_size, Some([32, 32]));
        for (i, p) in out.pixels.iter().enumerate() {
            let expected = if (8..24).contains(&(i % 32)) && (8..24).contains(&(i / 32)) {
                1.0
            } else {
                0.0
            };
            assert_eq!(p[3], expected, "pixel {i}, strokes={with_strokes}");
        }
    }
}
#[test]
fn grabcut_seed_only_and_reduced_resolution_restore_authoritative_labels() {
    let frame = Frame::new(
        64,
        64,
        (0..4096)
            .map(|i| {
                pixel(
                    if i % 64 < 32 {
                        [1., 0., 0.]
                    } else {
                        [0., 0., 1.]
                    },
                    1.0,
                )
            })
            .collect(),
    )
    .unwrap();
    let (out, report) = run(
        &frame,
        json!({"selection":{"type":"grab_cut","max_edge":16,"foreground":[{"points":[[0.2,0.5]],"radius":0.01}],"background":[{"points":[[0.8,0.5]],"radius":0.01}]},"refinement":{"method":"closed_form","max_edge":32},"feather":2.0}),
    );
    assert_eq!(report.segmentation_size, Some([16, 16]));
    assert_eq!(out.pixels[32 * 64 + 12][3], 1.0);
    assert_eq!(out.pixels[32 * 64 + 51][3], 0.0);
    assert!(out.pixels.iter().all(|p| (0.0..=1.0).contains(&p[3])));
}
#[test]
fn explicit_trimap_solves_transparency_and_mask_selection_scales_alpha() {
    let (w, h) = (15, 7);
    let frame = Frame::new(
        w,
        h,
        (0..w * h)
            .map(|i| {
                let a = (i % w) as f32 / (w - 1) as f32;
                [0.1 + 0.7 * a, 0.8 - 0.6 * a, 0.2 + 0.2 * a, 1.0]
            })
            .collect(),
    )
    .unwrap();
    let options:CutoutOptions=serde_json::from_value(json!({"selection":{"type":"trimap","path":"matte.png"},"refinement":{"iterations":500,"tolerance":1e-8}})).unwrap();
    let (out, report) = cutout::run(
        &frame,
        &options,
        &|_, _, _| {
            Ok((0..w * h)
                .map(|i| {
                    if i % w == 0 {
                        0.0
                    } else if i % w == w - 1 {
                        1.0
                    } else {
                        0.5
                    }
                })
                .collect())
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(report.solver.unwrap().converged);
    for (i, p) in out.pixels.iter().enumerate() {
        assert!((p[3] - (i % w as usize) as f32 / (w - 1) as f32).abs() < 2e-4);
    }
    let options: CutoutOptions = serde_json::from_value(
        json!({"selection":{"type":"mask","mask":{"type":"bitmap","path":"x.png"}}}),
    )
    .unwrap();
    let frame = Frame::new(3, 1, vec![[0.5, 0.2, 0.1, 0.4]; 3]).unwrap();
    let (out, _) = cutout::run(
        &frame,
        &options,
        &|_, _, _| Ok(vec![0.5; 3]),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(out.pixels, vec![[0.5, 0.2, 0.1, 0.2]; 3]);
}
#[test]
fn despill_removes_dominant_screen_color_without_changing_alpha() {
    let frame = Frame::new(1, 1, vec![pixel([0.4, 0.8, 0.3], 0.7)]).unwrap();
    let (out, _) = run(
        &frame,
        json!({"selection":{"type":"mask","mask":{"type":"rectangle","min":[0,0],"max":[1,1],"feather":0}},"despill":{"color":[0,1,0],"amount":1}}),
    );
    assert_eq!(out.pixels[0][3], 0.7);
    assert!((out.pixels[0][0] - out.pixels[0][1]).abs() < 1e-6);
}
#[test]
fn invalid_inputs_conflicting_seeds_and_cancellation_are_errors() {
    for options in [
        json!({"selection":{"type":"color"}}),
        json!({"selection":{"type":"grab_cut"}}),
        json!({"selection":{"type":"color","color":[0,1,0]},"decontaminate":2}),
        json!({"selection":{"type":"mask","mask":{"type":"bitmap","path":""}}}),
    ] {
        let opts: CutoutOptions = serde_json::from_value(options).unwrap();
        assert!(opts.validate().is_err());
    }
    assert!(
        serde_json::from_value::<CutoutOptions>(
            json!({"selection":{"type":"color","color":[0,1,0],"typo":1}})
        )
        .is_err()
    );
    let frame = Frame::new(16, 16, vec![[0.5, 0.2, 0.1, 1.0]; 256]).unwrap();
    let bad:CutoutOptions=serde_json::from_value(json!({"selection":{"type":"grab_cut","rect":[0.2,0.2,0.8,0.8],"foreground":[{"points":[[0.0,0.0]]}]}})).unwrap();
    assert!(
        cutout::run(
            &frame,
            &bad,
            &|_, _, _| unreachable!(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let good: CutoutOptions =
        serde_json::from_value(json!({"selection":{"type":"color","color":[0,1,0]}})).unwrap();
    assert!(
        cutout::run(
            &frame,
            &good,
            &|_, _, _| unreachable!(),
            &AtomicBool::new(true)
        )
        .is_err()
    );
    let recipe:Recipe=serde_json::from_value(json!({"nodes":[{"id":"key","op":{"type":"cutout","options":{"selection":{"type":"color","color":[0,1,0]}}}}],"output":"key"})).unwrap();
    assert!(recipe.validate().is_ok());
}

#[test]
fn alpha_matches_independent_pymatting_laplacian_and_scipy_direct_solver() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/matting-pymatting-1.1.16.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let w = case["width"].as_u64().unwrap() as u32;
        let h = case["height"].as_u64().unwrap() as u32;
        let pixels: Vec<[f32; 4]> = serde_json::from_value(case["pixels"].clone()).unwrap();
        let frame = Frame::new(w, h, pixels).unwrap();
        let trimap: Vec<f32> = serde_json::from_value(case["trimap"].clone()).unwrap();
        let opts:CutoutOptions=serde_json::from_value(json!({"selection":{"type":"trimap","path":"reference"},"refinement":{"space":"srgb","iterations":1000,"tolerance":1e-9}})).unwrap();
        let (out, report) = cutout::run(
            &frame,
            &opts,
            &|_, _, _| Ok(trimap.clone()),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(report.solver.unwrap().converged, "{}", case["name"]);
        for (i, p) in out.pixels.iter().enumerate() {
            let expected = case["expected"][i].as_f64().unwrap();
            let tol = case["absolute_alpha_tolerance"].as_f64().unwrap();
            assert!(
                (p[3] as f64 - expected).abs() < tol,
                "{} pixel {i}: {} != {expected}",
                case["name"],
                p[3]
            );
            if trimap[i] == 0.0 || trimap[i] == 1.0 {
                assert_eq!(p[3], trimap[i]);
            }
        }
    }
}

#[test]
fn linear_matting_keeps_negative_hdr_colors_and_physical_alpha() {
    let (w, h) = (15, 7);
    let frame = Frame::new(
        w,
        h,
        (0..w * h)
            .map(|i| {
                let a = (i % w) as f32 / (w - 1) as f32;
                [-0.2 + a, 3.0 - 1.5 * a, 0.6 + 0.9 * a, 1.0]
            })
            .collect(),
    )
    .unwrap();
    let trimap: Vec<_> = (0..w * h)
        .map(|i| {
            if i % w == 0 {
                0.0
            } else if i % w == w - 1 {
                1.0
            } else {
                0.5
            }
        })
        .collect();
    let opts:CutoutOptions=serde_json::from_value(json!({"selection":{"type":"trimap","path":"reference"},"refinement":{"iterations":1000,"tolerance":1e-9}})).unwrap();
    let (out, report) = cutout::run(
        &frame,
        &opts,
        &|_, _, _| Ok(trimap.clone()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(report.solver.unwrap().converged);
    for (i, p) in out.pixels.iter().enumerate() {
        assert!((p[3] - (i % w as usize) as f32 / (w - 1) as f32).abs() < 0.001);
    }
    assert!(out.pixels[1][0] < 0.0 && out.pixels[1][1] > 1.0);
}
