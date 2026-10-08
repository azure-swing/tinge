use vibecolor_io::temperature_white_xy;

#[test]
fn cct_and_signed_duv_match_independent_colour_science() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/raw-temperature-colour-0.4.7.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let xy = temperature_white_xy(
            case["kelvin"].as_f64().unwrap(),
            case["tint_duv"].as_f64().unwrap(),
        )
        .unwrap();
        for c in 0..2 {
            assert!(
                (xy[c] - case["xy"][c].as_f64().unwrap()).abs() < 3e-8,
                "{case}: got {xy:?}"
            );
        }
    }
    // A blackbody white at 6504K is distinct from the D65 daylight standard.
    let xy = temperature_white_xy(6504.0, 0.0).unwrap();
    assert!((xy[1] - 0.3290).abs() > 0.005);
    for (k, d) in [
        (1666.0, 0.0),
        (25001.0, 0.0),
        (6500.0, 0.051),
        (f64::NAN, 0.0),
        (6500.0, f64::INFINITY),
    ] {
        assert!(temperature_white_xy(k, d).is_err());
    }
}

#[test]
fn duv_uses_1960_uv_distance_and_correct_positive_normal() {
    let uv = |xy: [f64; 2]| {
        let d = -2.0 * xy[0] + 12.0 * xy[1] + 3.0;
        [4.0 * xy[0] / d, 6.0 * xy[1] / d]
    };
    for k in [1667.0, 3000.0, 6504.0, 25000.0] {
        let centre = uv(temperature_white_xy(k, 0.0).unwrap());
        for d in [-0.005, 0.005] {
            let p = uv(temperature_white_xy(k, d).unwrap());
            assert!(((p[0] - centre[0]).hypot(p[1] - centre[1]) - d.abs()).abs() < 1e-12);
            assert!((p[1] - centre[1]) * d > 0.0);
        }
    }
}
