use serde_json::{Value, json};
use vibecolor_ocio::{
    CdlStyle, ConfigSource, Transform, WORKING_SPACE, cdl::*, compile_with_context,
};

#[test]
fn all_three_cdl_formats_match_official_parameters_and_twenty_style_direction_cases() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cdl-exchange-ocio-2.5.2.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for file in fixture["files"].as_array().unwrap() {
        let path = temp
            .path()
            .join(format!("test.{}", file["format"].as_str().unwrap()));
        std::fs::write(&path, file["xml"].as_str().unwrap()).unwrap();
        let (document, hash) = read(&path).unwrap();
        let exported = write(&document).unwrap();
        std::fs::write(&path, exported).unwrap();
        assert_eq!(read(&path).unwrap().0, document);
        for case in file["cases"].as_array().unwrap() {
            let index = case["index"].as_u64().unwrap() as u32;
            let p = &case["parameters"];
            let correction = &document.corrections[index as usize];
            assert_eq!(json!(correction.slope), p["slope"]);
            assert_eq!(json!(correction.offset), p["offset"]);
            assert_eq!(json!(correction.power), p["power"]);
            assert_eq!(json!(correction.saturation), p["saturation"]);
            assert_eq!(correction.id, p["id"].as_str().unwrap());
            let style: CdlStyle = serde_json::from_value(case["style"].clone()).unwrap();
            let (selected, grade) = document
                .import(
                    &hash,
                    Some(&CdlSelector::Id {
                        id: correction.id.clone(),
                    }),
                    WORKING_SPACE.into(),
                    style,
                    case["inverse"].as_bool().unwrap(),
                )
                .unwrap();
            assert_eq!(selected, index as usize);
            let cpu = compile_with_context(
                &ConfigSource::Builtin {
                    name: "studio-config-v4.0.0_aces-v2.0_ocio-v2.5".into(),
                },
                &Transform::Grade {
                    source: WORKING_SPACE.into(),
                    grade,
                },
                &Default::default(),
            )
            .unwrap();
            let mut pixels: Vec<[f32; 4]> = serde_json::from_value(case["input"].clone()).unwrap();
            let expected: Vec<[f32; 4]> = serde_json::from_value(case["expected"].clone()).unwrap();
            cpu.apply(&mut pixels).unwrap();
            for (a, b) in pixels.iter().zip(expected) {
                for c in 0..3 {
                    assert!(
                        (a[c] - b[c]).abs() <= 3e-5 * b[c].abs().max(1.),
                        "{a:?} != {b:?}"
                    );
                }
                assert_eq!(a[3], b[3]);
            }
        }
        assert!(!document.corrections[0].metadata.sop_descriptions.is_empty());
        if document.corrections.len() > 1 {
            assert!(
                document
                    .import(&hash, None, WORKING_SPACE.into(), CdlStyle::Asc, false)
                    .is_err()
            );
            let (index, _) = document
                .import(
                    &hash,
                    Some(&CdlSelector::Id { id: "1".into() }),
                    WORKING_SPACE.into(),
                    CdlStyle::Asc,
                    false,
                )
                .unwrap();
            assert_eq!(index, 0);
            let (index, _) = document
                .import(
                    &hash,
                    Some(&CdlSelector::Index { index: 1 }),
                    WORKING_SPACE.into(),
                    CdlStyle::Asc,
                    false,
                )
                .unwrap();
            assert_eq!(index, 1);
        }
    }
}

#[test]
fn cdl_rejects_dropped_references_typoes_duplicates_dtd_bad_numeric_and_wrong_roots() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("invalid.cc");
    for xml in [
        "<ColorCorrection><SOPNode><Slop>1 1 1</Slop></SOPNode></ColorCorrection>",
        "<ColorCorrection><SOPNode><Slope>1 1 1</Slope><Slope>2 2 2</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode></ColorCorrection>",
        "<ColorCorrection><SatNode><Saturation>NaN</Saturation></SatNode></ColorCorrection>",
        "<ColorCorrection><SatNode><Saturation>-1</Saturation></SatNode></ColorCorrection>",
        "<!DOCTYPE ColorCorrection [<!ENTITY x 'anything'>]><ColorCorrection/>",
        "<ColorCorrectionCollection><ColorCorrection/></ColorCorrectionCollection>",
        "<ColorCorrection><ColorCorrectionRef ref='other'/></ColorCorrection>",
        "<?xml version='1.0' encoding='ISO-8859-1'?><ColorCorrection/>",
        "<ColorCorrection name='silently dropped'/>",
    ] {
        std::fs::write(&path, xml).unwrap();
        assert!(read(&path).is_err(), "accepted {xml}");
    }
    let path = temp.path().join("duplicate.ccc");
    std::fs::write(&path,"<ColorCorrectionCollection><ColorCorrection id='same'/><ColorCorrection id='same'/></ColorCorrectionCollection>").unwrap();
    assert!(read(&path).is_err());
}

#[test]
fn cdl_snapshots_ignore_filename_cache_and_legacy_grade_hash_has_no_new_default() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("editable.cc");
    std::fs::write(
        &path,
        "<ColorCorrection id='1'><SatNode><Saturation>0.5</Saturation></SatNode></ColorCorrection>",
    )
    .unwrap();
    let first = read(&path).unwrap();
    std::fs::write(
        &path,
        "<ColorCorrection id='1'><SatNode><Saturation>0.8</Saturation></SatNode></ColorCorrection>",
    )
    .unwrap();
    let second = read(&path).unwrap();
    assert_ne!(first.1, second.1);
    assert_eq!(second.0.corrections[0].saturation, 0.8);
    let old = r#"{"type":"cdl","color_space":"ACEScct","slope":[1.0,1.0,1.0],"offset":[0.0,0.0,0.0],"power":[1.0,1.0,1.0],"saturation":1.0,"style":"no_clamp","inverse":false}"#;
    let grade: vibecolor_ocio::NodeGrade = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&grade).unwrap(), old);
}

#[test]
fn export_preserves_full_f64_precision_and_xml_entities_once() {
    let document:CdlDocument=serde_json::from_value(json!({"format":"cc","corrections":[{"id":"&\"雪\n","slope":[1.2345678901234567,1,1],"offset":[-0.12345678901234566,1e-25,0],"power":[1.0000000000000002,1,1],"saturation":0.12345678901234566,"metadata":{"descriptions":["literal &amp; and & < > \" 雪\r"],"sop_descriptions":[" a "],"sat_descriptions":["b"]}}]})).unwrap();
    let xml = write(&document).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("precise.cc");
    std::fs::write(&path, xml).unwrap();
    assert_eq!(read(&path).unwrap().0, document);
}
