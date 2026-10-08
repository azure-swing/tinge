use vibecolor_color::{ColorSpace, Primaries, Transfer, convert_linear, decode};
use vibecolor_core::Frame;
use vibecolor_io::{ExportOptions, export, inspect, load, load_signal};

#[test]
fn wide_gamut_icc_png_and_float_tiff_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    for primary in [Primaries::DisplayP3, Primaries::Rec2020, Primaries::AcesCg] {
        for (ext, bits, rgb) in [
            ("png", 16, [0.18, 0.3, 0.2, 0.7]),
            ("tiff", 32, [-0.05, 2.0, 0.4, 1.0]),
        ] {
            let p = d.path().join(format!("{primary:?}.{ext}"));
            let frame = Frame::new(1, 1, vec![rgb]).unwrap();
            let space = ColorSpace {
                primaries: primary,
                transfer: if bits == 32 {
                    Transfer::Linear
                } else {
                    Transfer::Srgb
                },
            };
            let report = export(
                &frame,
                &p,
                ExportOptions {
                    bit_depth: bits,
                    space,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(report.color_metadata.icc_hash.is_some());
            assert_eq!(
                inspect(&p).unwrap().color_metadata.icc_hash,
                report.color_metadata.icc_hash,
                "{primary:?} {ext}"
            );
            let actual = load(&p, None).unwrap();
            for c in 0..4 {
                assert!(
                    (actual.pixels[0][c] - rgb[c]).abs() < 3e-4,
                    "{primary:?} {ext}: {:?} != {rgb:?}",
                    actual.pixels
                );
            }
        }
    }
}

#[test]
fn pq_png_has_exact_cicp_and_absolute_luminance_semantics() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("hdr.png");
    let frame = Frame::new(2, 1, vec![[1.0, 1.0, 1.0, 0.5], [4.0, 4.0, 4.0, 1.0]]).unwrap();
    let mut opt = ExportOptions {
        space: ColorSpace {
            primaries: Primaries::Rec2020,
            transfer: Transfer::Pq,
        },
        ..Default::default()
    };
    assert!(export(&frame, &p, opt).is_err());
    assert!(!p.exists());
    opt.linear_unit_nits = Some(203.0);
    let report = export(&frame, &p, opt).unwrap();
    assert_eq!(report.color_metadata.cicp, Some([9, 16, 0, 1]));
    let info = inspect(&p).unwrap();
    assert_eq!(info.color_metadata.cicp, Some([9, 16, 0, 1]));
    assert!(!info.has_icc);
    let signal = load_signal(&p).unwrap();
    let nits = decode(signal.pixels[0][0], Transfer::Pq) * 10000.0;
    assert!((nits - 203.0).abs() < 0.02, "{nits} nits");
    let scene = load(&p, None).unwrap();
    assert!((scene.pixels[1][0] - 812.0 / 10000.0).abs() < 2e-5);
    assert!((scene.pixels[0][3] - 0.5).abs() < 1e-5);
}

#[test]
fn exr_standard_primaries_metadata_and_premultiplied_samples() {
    use exr::prelude::*;
    let d = tempfile::tempdir().unwrap();
    for (primary, id) in [
        (Primaries::Srgb, "lin_rec709_scene"),
        (Primaries::DisplayP3, "lin_p3d65_scene"),
        (Primaries::Rec2020, "lin_rec2020_scene"),
        (Primaries::AcesCg, "lin_ap1_scene"),
    ] {
        let p = d.path().join(format!("{primary:?}.exr"));
        let frame = Frame::new(1, 1, vec![[-0.05, 4.0, 0.18, 0.25]]).unwrap();
        export(
            &frame,
            &p,
            ExportOptions {
                bit_depth: 32,
                space: ColorSpace {
                    primaries: primary,
                    transfer: Transfer::Linear,
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            inspect(&p)
                .unwrap()
                .color_metadata
                .color_interop_id
                .as_deref(),
            Some(id)
        );
        let physical = read_first_rgba_layer_from_file(
            &p,
            |_, _| [0.0; 4],
            |p, _, (r, g, b, a): (f32, f32, f32, f32)| *p = [r, g, b, a],
        )
        .unwrap();
        let raw = physical.layer_data.channel_data.pixels;
        let expected =
            convert_linear([-0.05, 4.0, 0.18], Primaries::Srgb, primary).map(|v| v * 0.25);
        for c in 0..3 {
            assert!((raw[c] - expected[c]).abs() < 1e-6);
        }
        let roundtrip = load(&p, None).unwrap();
        for c in 0..4 {
            assert!((roundtrip.pixels[0][c] - frame.pixels[0][c]).abs() < 3e-5);
        }
    }
}

#[test]
fn exr_conflicting_and_unknown_ids_require_explicit_resolution() {
    use exr::{meta::attribute::AttributeValue, prelude::*};
    let d = tempfile::tempdir().unwrap();
    for id in ["lin_ap1_scene", "my-studio:unknown-space"] {
        let p = d.path().join(format!(
            "{}.exr",
            if id == "lin_ap1_scene" {
                "conflict"
            } else {
                "unknown"
            }
        ));
        let mut image = Image::from_channels(
            (1, 1),
            SpecificChannels::rgba(|_| (0.2f32, 0.2f32, 0.2f32, 1.0f32)),
        );
        image.attributes.chromaticities = Some(exr::meta::attribute::Chromaticities {
            red: (0.64, 0.33).into(),
            green: (0.30, 0.60).into(),
            blue: (0.15, 0.06).into(),
            white: (0.3127, 0.3290).into(),
        });
        image.attributes.other.insert(
            Text::from("colorInteropID"),
            AttributeValue::Text(Text::from(id)),
        );
        image.write().to_file(&p).unwrap();
        assert!(load(&p, None).is_err());
        assert!(
            load(
                &p,
                Some(ColorSpace {
                    primaries: Primaries::Srgb,
                    transfer: Transfer::Linear
                })
            )
            .is_ok()
        );
    }
}

#[test]
fn external_exr_custom_chromaticities_use_real_cms() {
    use exr::{meta::attribute::Chromaticities, prelude::*};
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("adobe-rgb.exr");
    let mut image = Image::from_channels(
        (2, 1),
        SpecificChannels::rgba(|pos: Vec2<usize>| {
            if pos.x() == 0 {
                (1.0f32, 1.0f32, 1.0f32, 1.0f32)
            } else {
                (0.0, 1.0, 0.0, 1.0)
            }
        }),
    );
    image.attributes.chromaticities = Some(Chromaticities {
        red: (0.64, 0.33).into(),
        green: (0.21, 0.71).into(),
        blue: (0.15, 0.06).into(),
        white: (0.3127, 0.3290).into(),
    });
    image.write().to_file(&p).unwrap();
    let actual = load(&p, None).unwrap();
    // Official OCIO 2.5.2 Linear AdobeRGB -> Linear Rec.709 reference.
    let expected = [-0.39835575, 1.0, -0.04292899];
    for (c, reference) in expected.iter().enumerate() {
        assert!((actual.pixels[0][c] - 1.0).abs() < 3e-4);
        assert!(
            (actual.pixels[1][c] - reference).abs() < 3e-4,
            "{:?}",
            actual.pixels
        );
    }
}

#[test]
fn generated_icc_tags_recover_standard_xy_and_pcs_white() {
    use moxcms::{Matrix3d, Vector3d};
    let dir = tempfile::tempdir().unwrap();
    for (primary, xy) in [
        (
            Primaries::Srgb,
            [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]],
        ),
        (
            Primaries::DisplayP3,
            [[0.68, 0.32], [0.265, 0.69], [0.15, 0.06], [0.3127, 0.3290]],
        ),
        (
            Primaries::Rec2020,
            [
                [0.708, 0.292],
                [0.170, 0.797],
                [0.131, 0.046],
                [0.3127, 0.3290],
            ],
        ),
        (
            Primaries::AcesCg,
            [
                [0.713, 0.293],
                [0.165, 0.830],
                [0.128, 0.044],
                [0.32168, 0.33767],
            ],
        ),
    ] {
        let path = dir.path().join(format!("{primary:?}.png"));
        export(
            &Frame::new(1, 1, vec![[0.2, 0.3, 0.4, 1.0]]).unwrap(),
            &path,
            ExportOptions {
                space: ColorSpace {
                    primaries: primary,
                    transfer: Transfer::Srgb,
                },
                ..Default::default()
            },
        )
        .unwrap();
        let reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
                .read_info()
                .unwrap();
        let bytes = reader.info().icc_profile.as_ref().unwrap();
        // Decode the ICC tag table and signed 15.16 numbers independently of the CMS parser.
        let u32_at = |i| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let fixed = |i| i32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as f64 / 65536.0;
        let tag = |signature: &[u8]| {
            (0..u32_at(128))
                .map(|i| 132 + i * 12)
                .find(|i| &bytes[*i..*i + 4] == signature)
                .map(|i| u32_at(i + 4))
                .unwrap()
        };
        let chad = tag(b"chad") + 8;
        let inverse = Matrix3d {
            v: std::array::from_fn(|r| std::array::from_fn(|c| fixed(chad + (r * 3 + c) * 4))),
        }
        .inverse();
        for (i, signature) in [b"rXYZ", b"gXYZ", b"bXYZ", b"wtpt"].into_iter().enumerate() {
            let offset = tag(signature) + 8;
            let value = [fixed(offset), fixed(offset + 4), fixed(offset + 8)];
            if i == 3 {
                for (actual, expected) in value.iter().zip([0.9642, 1.0, 0.8249]) {
                    assert!((actual - expected).abs() < 2e-5);
                }
            }
            let native = inverse.mul_vector(Vector3d { v: value }).v;
            let sum: f64 = native.iter().sum();
            for c in 0..2 {
                assert!((native[c] / sum - xy[i][c]).abs() < 1e-4, "{primary:?} {i}");
            }
        }
    }
}

#[test]
fn exr_alpha_conventions_are_explicit_at_zero_alpha_and_straight_override() {
    use exr::{meta::attribute::AttributeValue, prelude::*};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("transparent.exr");
    let report = export(
        &Frame::new(1, 1, vec![[2.0, 0.2, 0.1, 0.0]]).unwrap(),
        &path,
        ExportOptions {
            bit_depth: 32,
            space: ColorSpace {
                primaries: Primaries::Srgb,
                transfer: Transfer::Linear,
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("hidden RGB discarded"))
    );
    assert_eq!(load(&path, None).unwrap().pixels[0], [0.0; 4]);
    let path = dir.path().join("additive.exr");
    let mut image = Image::from_channels(
        (1, 1),
        SpecificChannels::rgba(|_| (2.0f32, 0.2f32, 0.1f32, 0.0f32)),
    );
    image.write().to_file(&path).unwrap();
    assert!(
        load(&path, None)
            .unwrap_err()
            .to_string()
            .contains("additive")
    );
    image.layer_data.attributes.other.insert(
        Text::from("vibecolorAlphaMode"),
        AttributeValue::Text(Text::from("straight")),
    );
    image.write().to_file(&path).unwrap();
    assert_eq!(load(&path, None).unwrap().pixels[0], [2.0, 0.2, 0.1, 0.0]);
    assert!(
        inspect(&path)
            .unwrap()
            .color_metadata
            .alpha_mode
            .starts_with("straight")
    );
}
