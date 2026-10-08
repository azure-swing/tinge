#![cfg(feature = "raw")]
use vibecolor_io::{load, raw};

/// A real, uncompressed 12-bit Bayer DNG container generated from known sensor values.
/// Tests decoding and linear development without depending on a camera/photo license.
mod support;
use support::{dng, dng_with};

#[test]
fn raw_controls_sensor_levels_wb_exposure_and_hdr() {
    use vibecolor_io::{RawDevelopOptions, RawWhiteBalance, SensorLevels};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sensor.dng");
    std::fs::write(&input, dng()).unwrap();
    let options = RawDevelopOptions {
        white_balance: RawWhiteBalance::CameraGains {
            gains: vec![2.0, 2.0, 1.0],
        },
        exposure_ev: 2.0,
        black_levels: Some(SensorLevels {
            repeat: [2, 2],
            values: vec![64.0, 64.0, 64.0, 64.0],
        }),
        white_levels: Some(SensorLevels {
            repeat: [1, 1],
            values: vec![4095.0],
        }),
        ..Default::default()
    };
    let frame = raw::load_with_options(&input, &options).unwrap();
    let p = frame.pixels[16 * 32 + 16];
    let expected = [6400.0 / 4031.0, 4800.0 / 4031.0, 1600.0 / 4031.0];
    for c in 0..3 {
        assert!((p[c] - expected[c]).abs() < 0.003, "{p:?}");
    }
    assert!(p[0] > 1.0);
    assert_eq!(p[3], 1.0);
    let plan = raw::plan_with_probes(&input, &options, &[[16, 16], [17, 16], [17, 17]]).unwrap();
    assert_eq!(plan.white_balance_gains, vec![1.0, 1.0, 0.5]);
    assert_eq!(plan.sensor_probes[0].native_values, vec![1664.0]);
    assert!((plan.sensor_probes[0].normalized_before_wb[0] - 1600.0 / 4031.0).abs() < 1e-7);
    let xy = RawDevelopOptions {
        white_balance: RawWhiteBalance::Chromaticity {
            xy: [0.3127, 0.3290],
        },
        ..Default::default()
    };
    let plan = raw::plan(&input, &xy).unwrap();
    assert!(
        plan.white_balance_gains
            .iter()
            .all(|v| (*v - 1.0).abs() < 0.003)
    );
}
#[test]
fn raw_controls_monochrome_negative_crop_and_orientation() {
    use vibecolor_io::{BelowBlack, RawCrop, RawDevelopOptions, SensorLevels};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("mono.dng");
    let long = |v: u32| v.to_le_bytes().to_vec();
    let overrides = vec![
        (262, 3, 1, 1u16.to_le_bytes().to_vec()),
        (274, 3, 1, 6u16.to_le_bytes().to_vec()),
        (50719, 4, 2, [long(4), long(6)].concat()),
        (50720, 4, 2, [long(24), long(20)].concat()),
        (50829, 4, 4, [long(2), long(2), long(30), long(30)].concat()),
    ];
    std::fs::write(&input, dng_with(overrides, Some(vec![32; 1024]))).unwrap();
    let mut options = RawDevelopOptions {
        below_black: BelowBlack::Preserve,
        black_levels: Some(SensorLevels {
            repeat: [2, 2],
            values: vec![64.0, 48.0, 64.0, 48.0],
        }),
        ..Default::default()
    };
    let f = raw::load_with_options(&input, &options).unwrap();
    assert_eq!((f.width, f.height), (20, 24));
    assert!(
        f.pixels
            .iter()
            .all(|p| p[0] < 0.0 && p[0] == p[1] && p[1] == p[2])
    );
    options.below_black = BelowBlack::Clip;
    assert!(
        raw::load_with_options(&input, &options)
            .unwrap()
            .pixels
            .iter()
            .all(|p| *p == [0.0, 0.0, 0.0, 1.0])
    );
    options.crop = RawCrop::Sensor;
    options.apply_orientation = false;
    let f = raw::load_with_options(&input, &options).unwrap();
    assert_eq!((f.width, f.height), (32, 32));
    options.crop = RawCrop::Active;
    let f = raw::load_with_options(&input, &options).unwrap();
    assert_eq!((f.width, f.height), (28, 28));
}
#[test]
fn raw_controls_reject_invalid_sensor_interpretations() {
    use vibecolor_io::{RawDevelopOptions, RawWhiteBalance, SensorLevels};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sensor.dng");
    std::fs::write(&input, dng()).unwrap();
    let bad = vec![
        RawDevelopOptions {
            algorithm_version: 2,
            ..Default::default()
        },
        RawDevelopOptions {
            exposure_ev: 21.0,
            ..Default::default()
        },
        RawDevelopOptions {
            white_balance: RawWhiteBalance::CameraGains {
                gains: vec![1.0; 4],
            },
            ..Default::default()
        },
        RawDevelopOptions {
            white_balance: RawWhiteBalance::Chromaticity { xy: [0.5, 0.5] },
            ..Default::default()
        },
        RawDevelopOptions {
            black_levels: Some(SensorLevels {
                repeat: [2, 2],
                values: vec![0.0; 3],
            }),
            ..Default::default()
        },
        RawDevelopOptions {
            white_levels: Some(SensorLevels {
                repeat: [1, 1],
                values: vec![64.0],
            }),
            ..Default::default()
        },
        RawDevelopOptions {
            calibration_illuminant: Some(17),
            ..Default::default()
        },
        RawDevelopOptions {
            calibration_illuminant: Some(2),
            ..Default::default()
        },
    ];
    for options in bad {
        assert!(raw::plan(&input, &options).is_err(), "{options:?}");
    }
    assert!(raw::plan_with_probes(&input, &Default::default(), &[[32, 0]]).is_err());
    assert!(raw::plan_with_probes(&input, &Default::default(), &[[0, 0]; 257]).is_err());
    assert!(raw::plan(&dir.path().join("sensor.png"), &Default::default()).is_err());
}

#[test]
fn raw_temperature_resolves_xy_and_develops_the_same_camera_gains() {
    use vibecolor_io::{RawDevelopOptions, RawWhiteBalance};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("temperature.dng");
    std::fs::write(&input, dng()).unwrap();
    let options = RawDevelopOptions {
        white_balance: RawWhiteBalance::Temperature {
            kelvin: 4500.0,
            tint_duv: 0.002,
        },
        ..Default::default()
    };
    let plan = raw::plan(&input, &options).unwrap();
    assert_eq!(plan.white_balance_source, "temperature_kang2002_duv");
    let xy = plan.white_balance_white_xy.unwrap();
    let explicit = RawDevelopOptions {
        white_balance: RawWhiteBalance::Chromaticity { xy },
        ..Default::default()
    };
    let a = raw::load_with_options(&input, &options).unwrap();
    let b = raw::load_with_options(&input, &explicit).unwrap();
    assert_eq!(a.pixels, b.pixels);
    let same = raw::plan(&input, &explicit).unwrap();
    assert_eq!(plan.white_balance_gains, same.white_balance_gains);
}

#[test]
fn illuminant_a_camera_matrix_consumes_adapted_d65_xyz() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.dng");
    // Independently computed in f64 using published Bradford cone matrix and
    // A=[1.09850,1,.35585], D65=[.95047,1,1.08883]. M_A=M_sRGB*CAT(A->D65).
    let rational = |x: i32| [x.to_le_bytes(), 1_000_000i32.to_le_bytes()].concat();
    let matrix = [
        2_907_413, -2_012_061, -510_700, -1_071_737, 2_180_039, -7_685, 159_296, -374_451,
        3_370_700,
    ];
    std::fs::write(
        &input,
        dng_with(
            vec![
                (
                    50721,
                    10,
                    9,
                    matrix.into_iter().flat_map(rational).collect(),
                ),
                (50778, 3, 1, 17u16.to_le_bytes().to_vec()),
            ],
            None,
        ),
    )
    .unwrap();
    let options = vibecolor_io::RawDevelopOptions {
        calibration_illuminant: Some(17),
        ..Default::default()
    };
    let plan = raw::plan(&input, &options).unwrap();
    let expected = [
        [3.240454, -1.537139, -0.498531],
        [-0.969266, 1.876011, 0.041556],
        [0.055643, -0.204026, 1.057225],
    ];
    for (row, target) in plan.xyz_to_camera_d65.iter().zip(expected) {
        for c in 0..3 {
            assert!(
                (row[c] - target[c]).abs() < 3e-5,
                "{row:?} versus {target:?}"
            );
        }
    }
    let f = raw::load_with_options(&input, &options).unwrap();
    let p = f.pixels[16 * 32 + 16];
    let expected = [1600.0 / 4031.0, 1200.0 / 4031.0, 800.0 / 4031.0];
    for c in 0..3 {
        assert!((p[c] - expected[c]).abs() < 0.003, "{p:?}");
    }
}

#[test]
fn unsupported_four_plane_dng_rejected_before_native_demosaic() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("four.dng");
    let rational = |x: i32| [x.to_le_bytes(), 1_000_000i32.to_le_bytes()].concat();
    let overrides = vec![
        (50710, 1, 4, vec![0, 1, 2, 3]),
        (33422, 1, 4, vec![0, 1, 3, 2]),
        (
            50721,
            10,
            12,
            [
                3_240_454, -1_537_139, -498_531, -969_266, 1_876_011, 41_556, 55_643, -204_026,
                1_057_225, -969_266, 1_876_011, 41_556,
            ]
            .into_iter()
            .flat_map(rational)
            .collect(),
        ),
        (
            50728,
            5,
            4,
            [0u32.to_le_bytes(), 1u32.to_le_bytes()].concat().repeat(4),
        ),
    ];
    std::fs::write(&input, dng_with(overrides, None)).unwrap();
    // rawler 0.8 DNG make_camera ignores CFAPlaneColor and reports RGB planes.
    // Reject this inconsistent plane count before the native demosaicer.
    assert!(
        raw::plan(&input, &Default::default())
            .unwrap_err()
            .to_string()
            .contains("plane count")
    );
}
#[test]
fn raw_bayer_dng_decode_develop_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sensor.dng");
    std::fs::write(&input, dng()).unwrap();
    let (info, meta) = raw::inspect(&input).unwrap();
    assert_eq!(info.width, 32);
    assert_eq!(meta.make, "VibeColor");
    let f = load(&input, None).unwrap();
    assert_eq!((f.width, f.height), (32, 32));
    let p = f.pixels[16 * 32 + 16];
    // D65 camera matrix equals sRGB, WB is unity; black is 64, white is 4095.
    let expected = [1600.0 / 4031.0, 1200.0 / 4031.0, 800.0 / 4031.0];
    assert!(
        (0..3).all(|c| (p[c] - expected[c]).abs() < 0.003),
        "{p:?} expected {expected:?}"
    );
    let out = dir.path().join("developed.exr");
    vibecolor_io::export(
        &f,
        &out,
        vibecolor_io::ExportOptions {
            bit_depth: 32,
            space: vibecolor_color::ColorSpace {
                primaries: vibecolor_color::Primaries::Srgb,
                transfer: vibecolor_color::Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(out.exists());
}
