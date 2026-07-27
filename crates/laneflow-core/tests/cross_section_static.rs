use laneflow_core::{CoreError, FacilityKind, FacilityKindCategory};

#[test]
fn seed_kinds_parse_to_expected_category() {
    let cases = [
        (
            "motorLane",
            FacilityKind::MotorLane,
            FacilityKindCategory::LaneBearing,
        ),
        (
            "nonMotorLane",
            FacilityKind::NonMotorLane,
            FacilityKindCategory::LaneBearing,
        ),
        (
            "sidewalk",
            FacilityKind::Sidewalk,
            FacilityKindCategory::NonTraversable,
        ),
        (
            "median",
            FacilityKind::Median,
            FacilityKindCategory::NonTraversable,
        ),
        (
            "plantingStrip",
            FacilityKind::PlantingStrip,
            FacilityKindCategory::NonTraversable,
        ),
        (
            "facilityStrip",
            FacilityKind::FacilityStrip,
            FacilityKindCategory::NonTraversable,
        ),
        (
            "shoulder",
            FacilityKind::Shoulder,
            FacilityKindCategory::NonTraversable,
        ),
    ];

    for (token, expected_kind, expected_category) in cases {
        let parsed = FacilityKind::parse(token).expect("seed kind must parse");
        assert_eq!(parsed, expected_kind, "kind mismatch for {token}");
        assert_eq!(
            parsed.category(),
            expected_category,
            "category mismatch for {token}"
        );
    }
}

#[test]
fn custom_kinds_parse_by_prefix() {
    let cases = [
        ("x-lane-tram", FacilityKindCategory::LaneBearing),
        ("x-lane-bus-only", FacilityKindCategory::LaneBearing),
        ("x-kiosk", FacilityKindCategory::NonTraversable),
        ("x-lane", FacilityKindCategory::NonTraversable),
        ("x-x", FacilityKindCategory::NonTraversable),
    ];

    for (token, expected_category) in cases {
        let parsed = FacilityKind::parse(token).expect("custom kind must parse");
        match &parsed {
            FacilityKind::CustomLaneBearing(retained) | FacilityKind::CustomBand(retained) => {
                assert_eq!(retained, token, "custom kind must retain full token");
            }
            other => panic!("custom token {token} parsed as seed kind {other:?}"),
        }
        assert_eq!(
            parsed.category(),
            expected_category,
            "category mismatch for {token}"
        );
    }
}

#[test]
fn unknown_or_malformed_kinds_are_rejected() {
    for token in [
        "motorlane",
        "MotorLane",
        "lane",
        "busLane",
        "",
        "x-",
        "x-lane-",
        "motorLane ",
    ] {
        let error = FacilityKind::parse(token).expect_err("unknown kind must fail");
        std::assert_matches!(
            error,
            CoreError::UnknownFacilityKind { kind } if kind == token
        );
    }
}

#[test]
fn overlong_custom_kind_is_rejected_at_schema_bound() {
    // 与 schema `facilityKindId` 的 maxLength 128 契约一致（loader 路径不执行 JSON Schema）。
    let overlong = format!("x-lane-{}", "a".repeat(122));
    std::assert_matches!(
        FacilityKind::parse(&overlong),
        Err(CoreError::FacilityKindTokenTooLong { len, .. }) if len == 129
    );
    // 边界：恰好 128 字符的自定义 kind 合法。
    let boundary = format!("x-lane-{}", "a".repeat(121));
    assert_eq!(boundary.chars().count(), 128);
    std::assert_matches!(
        FacilityKind::parse(&boundary),
        Ok(FacilityKind::CustomLaneBearing(_))
    );
}
