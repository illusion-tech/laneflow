use laneflow_core::{
    AccessCell, AccessEffect, AccessRegistry, AccessRegulation, AccessRule, AccessTargetId,
    CoreError, CorridorElementId, CrossSectionRegistry, EdgeLength, FacilityBand, Junction,
    JunctionRegistry, LaneEdge, LaneGraph, LaneGroup, ManeuverPath, Movement, ParticipantClass,
    ParticipantClassRegistry, RoadCorridor, RoadSection, SectionLane, SpeedLimit,
};

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(10.0).expect("test edge length"),
        SpeedLimit::try_new(10.0).expect("test speed limit"),
        next.iter().copied(),
    )
}

/// fixture 图：`e1 -> e2`（section-a 唯一 lane）、`e3`（section-b），
/// 以及两条共享 internal edge 的 ManeuverPath（左转/直行）所属的 junction 边。
fn graph() -> LaneGraph {
    LaneGraph::try_new([
        edge("e1", &["e2"]),
        edge("e2", &[]),
        edge("e3", &[]),
        edge("je-entry-a", &["je-internal"]),
        edge("je-entry-b", &["je-internal"]),
        edge("je-internal", &["je-exit-left", "je-exit-straight"]),
        edge("je-exit-left", &[]),
        edge("je-exit-straight", &[]),
    ])
    .expect("test graph")
}

fn junctions() -> JunctionRegistry {
    JunctionRegistry::try_new(
        &graph(),
        [Junction::new("junction-1")],
        [Movement::new("movement-1", "junction-1")],
        [
            ManeuverPath::new(
                "path-left",
                "movement-1",
                "je-entry-a",
                ["je-internal"],
                "je-exit-left",
            ),
            ManeuverPath::new(
                "path-straight",
                "movement-1",
                "je-entry-b",
                ["je-internal"],
                "je-exit-straight",
            ),
        ],
    )
    .expect("test junctions")
}

fn cross_section() -> CrossSectionRegistry {
    CrossSectionRegistry::try_new(
        &graph(),
        [FacilityBand::new("band-m", "median")],
        [
            RoadSection::new(
                "section-a",
                "motorLane",
                [SectionLane::new(["e1", "e2"], Some("group-bus"))],
            ),
            RoadSection::new("section-b", "motorLane", [SectionLane::new(["e3"], None)]),
        ],
        [LaneGroup::new("group-bus", "section-a")],
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [
                CorridorElementId::band("band-m"),
                CorridorElementId::section("section-a"),
                CorridorElementId::section("section-b"),
            ],
        )],
    )
    .expect("test cross-section")
}

fn classes() -> ParticipantClassRegistry {
    ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
        ParticipantClass::new("bus", Some("motorVehicle")),
        ParticipantClass::new("truck", Some("motorVehicle")),
        ParticipantClass::new("largeBus", Some("bus")),
    ])
    .expect("test classes")
}

fn access(rules: Vec<AccessRule>) -> AccessRegistry {
    AccessRegistry::try_new(&graph(), &junctions(), &cross_section(), &classes(), rules)
        .expect("valid access registry")
}

fn class(
    registry: &ParticipantClassRegistry,
    external_id: &str,
) -> laneflow_core::ParticipantClassHandle {
    registry.class_handle(external_id).expect("class handle")
}

fn edge_cell(
    registry: &AccessRegistry,
    classes: &ParticipantClassRegistry,
    edge_id: &str,
    class_id: &str,
) -> AccessCell {
    let edge = graph().edge_handle(edge_id).expect("edge handle");
    registry.edge_access(edge, class(classes, class_id))
}

#[test]
fn registry_resolves_rules_in_normalization_order() {
    let rules = vec![
        AccessRule::new(
            "rule-b",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        ),
        AccessRule::new(
            "rule-a",
            AccessTargetId::road_section("section-b"),
            AccessEffect::Allow,
            ["bus"],
        ),
    ];
    let registry = access(rules);

    assert!(!registry.is_empty());
    assert_eq!(registry.rule_count(), 2);
    let ids = registry
        .rules()
        .map(|handle| registry.rule_external_id(handle).expect("rule external ID"))
        .collect::<Vec<_>>();
    assert_eq!(ids, ["rule-b", "rule-a"]);

    let rule_a = registry.rule_handle("rule-a").expect("rule-a");
    assert_eq!(
        registry.rule(rule_a).map(AccessRule::effect),
        Some(AccessEffect::Allow)
    );
    assert!(registry.rule_handle("missing").is_none());
}

#[test]
fn empty_rules_leave_everything_unconstrained() {
    let registry = access(Vec::new());
    let classes = classes();
    let junctions = junctions();

    assert!(registry.is_empty());
    assert_eq!(
        edge_cell(&registry, &classes, "e1", "car"),
        AccessCell::Unconstrained
    );
    let path = junctions
        .maneuver_path_handle("path-left")
        .expect("path-left");
    assert_eq!(
        registry.path_access(path, class(&classes, "truck")),
        AccessCell::Unconstrained
    );

    // 越界 class handle 同样返回 Unconstrained（不 panic）。
    let larger = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
        ParticipantClass::new("bus", Some("motorVehicle")),
        ParticipantClass::new("truck", Some("motorVehicle")),
        ParticipantClass::new("largeBus", Some("bus")),
        ParticipantClass::new("pedestrian", None),
    ])
    .expect("larger classes");
    let out_of_range = class(&larger, "pedestrian");
    let edge = graph().edge_handle("e1").expect("e1");
    assert_eq!(
        registry.edge_access(edge, out_of_range),
        AccessCell::Unconstrained
    );
    assert_eq!(
        registry.path_access(path, out_of_range),
        AccessCell::Unconstrained
    );
}

// ---------- phase 9.1-9.3：identity 与 unknown ----------

#[test]
fn rule_identity_errors() {
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![AccessRule::new(
            "bad id",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        )],
    )
    .expect_err("malformed rule id must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "accessRules[].id" && external_id == "bad id"
    );

    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-1",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["car"],
            ),
            AccessRule::new(
                "rule-1",
                AccessTargetId::lane_edge("e2"),
                AccessEffect::Allow,
                ["bus"],
            ),
        ],
    )
    .expect_err("duplicate rule id must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateAccessRuleId { rule_id } if rule_id == "rule-1"
    );
}

#[test]
fn unknown_targets_are_rejected_with_attribution() {
    let cases: [(AccessTargetId, &'static str); 5] = [
        (AccessTargetId::lane_edge("missing"), "laneEdge"),
        (AccessTargetId::lane_group("missing"), "laneGroup"),
        (AccessTargetId::road_section("missing"), "roadSection"),
        (AccessTargetId::maneuver_path("missing"), "maneuverPath"),
        // facilityBand 的 unknown 检查先于 capability guard。
        (AccessTargetId::facility_band("missing"), "facilityBand"),
    ];

    for (target, expected_kind) in cases {
        let error = AccessRegistry::try_new(
            &graph(),
            &junctions(),
            &cross_section(),
            &classes(),
            vec![AccessRule::new(
                "rule-1",
                target,
                AccessEffect::Deny,
                ["car"],
            )],
        )
        .expect_err("unknown target must fail");
        std::assert_matches!(
            error,
            CoreError::UnknownAccessRuleTarget { rule_id, target_kind, target_id }
                if rule_id == "rule-1" && target_kind == expected_kind && target_id == "missing"
        );
    }
}

#[test]
fn unknown_or_empty_participant_classes_are_rejected() {
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![AccessRule::new(
            "rule-1",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            Vec::<String>::new(),
        )],
    )
    .expect_err("empty participant classes must fail");
    std::assert_matches!(
        error,
        CoreError::EmptyAccessRuleParticipantClasses { rule_id } if rule_id == "rule-1"
    );

    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![AccessRule::new(
            "rule-1",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car", "missing"],
        )],
    )
    .expect_err("unknown participant class must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownAccessRuleParticipantClass { rule_id, class_id }
            if rule_id == "rule-1" && class_id == "missing"
    );
}

// ---------- phase 9.4：capability guard ----------

#[test]
fn capability_guard_rejects_facility_band_target_and_time_windows() {
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![AccessRule::new(
            "rule-band",
            AccessTargetId::facility_band("band-m"),
            AccessEffect::Allow,
            ["car"],
        )],
    )
    .expect_err("facility band target must be guarded");
    std::assert_matches!(
        error,
        CoreError::AccessCapabilityUnavailable { rule_id, capability }
            if rule_id == "rule-band" && capability == "facilityBandTarget"
    );

    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-time",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["truck"],
            )
            .with_time_windows(true),
        ],
    )
    .expect_err("time windows must be guarded");
    std::assert_matches!(
        error,
        CoreError::AccessCapabilityUnavailable { rule_id, capability }
            if rule_id == "rule-time" && capability == "timeWindows"
    );

    // 同一条规则两类 guard 同时命中时 band target 先归因。
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-both",
                AccessTargetId::facility_band("band-m"),
                AccessEffect::Allow,
                ["car"],
            )
            .with_time_windows(true),
        ],
    )
    .expect_err("band target guard must win attribution");
    std::assert_matches!(
        error,
        CoreError::AccessCapabilityUnavailable { capability, .. }
            if capability == "facilityBandTarget"
    );
}

#[test]
fn capability_guard_orders_after_unknown_and_before_composition() {
    // guard 在 unknown class 检查之后。
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-band",
                AccessTargetId::facility_band("band-m"),
                AccessEffect::Allow,
                ["car"],
            ),
            AccessRule::new(
                "rule-unknown-class",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["missing"],
            ),
        ],
    )
    .expect_err("unknown class must precede guard");
    std::assert_matches!(
        error,
        CoreError::UnknownAccessRuleParticipantClass { rule_id, .. }
            if rule_id == "rule-unknown-class"
    );

    // guard 在组合歧义检查之前。
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-band",
                AccessTargetId::facility_band("band-m"),
                AccessEffect::Allow,
                ["car"],
            ),
            AccessRule::new(
                "rule-allow",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Allow,
                ["car"],
            ),
            AccessRule::new(
                "rule-deny",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["car"],
            ),
        ],
    )
    .expect_err("guard must precede composition checks");
    std::assert_matches!(
        error,
        CoreError::AccessCapabilityUnavailable { rule_id, .. } if rule_id == "rule-band"
    );
}

// ---------- phase 9.5：regulation provenance 单一性 ----------

#[test]
fn regulation_string_length_bounds_are_enforced() {
    // 空串与超长串拒绝（与 schema 的 1..=128 字符契约一致，loader 路径不执行 JSON Schema）。
    let long = "a".repeat(129);
    let boundary = "a".repeat(128);
    for (field, result) in [
        ("jurisdiction", AccessRegulation::try_new("", "2026", None)),
        (
            "jurisdiction",
            AccessRegulation::try_new(long.as_str(), "2026", None),
        ),
        ("version", AccessRegulation::try_new("CN", "", None)),
        (
            "version",
            AccessRegulation::try_new("CN", long.as_str(), None),
        ),
        ("source", AccessRegulation::try_new("CN", "2026", Some(""))),
        (
            "source",
            AccessRegulation::try_new("CN", "2026", Some(long.as_str())),
        ),
    ] {
        std::assert_matches!(
            result,
            Err(CoreError::InvalidAccessRegulationString { field: actual, .. })
                if actual == field
        );
    }
    // 边界值 1 与 128 字符合法。
    AccessRegulation::try_new("C", "2", Some("s")).expect("single-char fields must pass");
    AccessRegulation::try_new(
        boundary.as_str(),
        boundary.as_str(),
        Some(boundary.as_str()),
    )
    .expect("128-char fields must pass");
}

#[test]
fn regulation_provenance_must_be_uniform() {
    // (jurisdiction, version) 混合 → 拒绝。
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-1",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["car"],
            )
            .with_regulation(AccessRegulation::try_new("CN", "2026", Some("src-a")).unwrap()),
            AccessRule::new(
                "rule-2",
                AccessTargetId::lane_edge("e2"),
                AccessEffect::Deny,
                ["car"],
            )
            .with_regulation(AccessRegulation::try_new("CN", "2027", None).unwrap()),
        ],
    )
    .expect_err("mixed regulation provenance must fail");
    std::assert_matches!(
        error,
        CoreError::AccessRegulationMismatch {
            first_rule_id,
            jurisdiction,
            version,
            duplicate_rule_id,
            duplicate_jurisdiction,
            duplicate_version,
        } if first_rule_id == "rule-1" && jurisdiction == "CN" && version == "2026"
            && duplicate_rule_id == "rule-2" && duplicate_jurisdiction == "CN"
            && duplicate_version == "2027"
    );

    // 同一 (jurisdiction, version) 不同 source → 合法；未声明者不参与约束。
    access(vec![
        AccessRule::new(
            "rule-1",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        )
        .with_regulation(AccessRegulation::try_new("CN", "2026", Some("src-a")).unwrap()),
        AccessRule::new(
            "rule-2",
            AccessTargetId::lane_edge("e2"),
            AccessEffect::Deny,
            ["car"],
        )
        .with_regulation(AccessRegulation::try_new("CN", "2026", Some("src-b")).unwrap()),
        AccessRule::new(
            "rule-3",
            AccessTargetId::lane_edge("e3"),
            AccessEffect::Allow,
            ["bus"],
        ),
    ]);
}

// ---------- §6.4 组合裁决 ----------

#[test]
fn bus_lane_pattern_denies_motor_vehicles_but_allows_bus() {
    let registry = access(vec![
        AccessRule::new(
            "rule-deny-motor",
            AccessTargetId::lane_group("group-bus"),
            AccessEffect::Deny,
            ["motorVehicle"],
        ),
        AccessRule::new(
            "rule-allow-bus",
            AccessTargetId::lane_group("group-bus"),
            AccessEffect::Allow,
            ["bus"],
        ),
    ]);
    let classes = classes();

    // group 展开覆盖 lane 的全部 edge。
    for edge_id in ["e1", "e2"] {
        let car_cell = edge_cell(&registry, &classes, edge_id, "car");
        std::assert_matches!(
            car_cell,
            AccessCell::Decided {
                effect: AccessEffect::Deny,
                ..
            }
        );
        let bus_cell = edge_cell(&registry, &classes, edge_id, "bus");
        std::assert_matches!(
            bus_cell,
            AccessCell::Decided { rule, effect: AccessEffect::Allow }
                if registry.rule_external_id(rule) == Some("rule-allow-bus")
        );
        // 传递后代同样匹配（largeBus extends bus）。
        let large_bus_cell = edge_cell(&registry, &classes, edge_id, "largeBus");
        std::assert_matches!(
            large_bus_cell,
            AccessCell::Decided {
                effect: AccessEffect::Allow,
                ..
            }
        );
        let truck_cell = edge_cell(&registry, &classes, edge_id, "truck");
        std::assert_matches!(
            truck_cell,
            AccessCell::Decided {
                effect: AccessEffect::Deny,
                ..
            }
        );
    }

    // 未被 target 覆盖的 edge 无约束。
    assert_eq!(
        edge_cell(&registry, &classes, "e3", "car"),
        AccessCell::Unconstrained
    );
}

#[test]
fn single_lane_exception_uses_target_specificity() {
    // section 级 deny + edge 级 allow（同参与者深度）→ edge 获胜。
    let registry = access(vec![
        AccessRule::new(
            "rule-deny-section",
            AccessTargetId::road_section("section-a"),
            AccessEffect::Deny,
            ["car"],
        ),
        AccessRule::new(
            "rule-allow-edge",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Allow,
            ["car"],
        ),
    ]);
    let classes = classes();

    std::assert_matches!(
        edge_cell(&registry, &classes, "e1", "car"),
        AccessCell::Decided { rule, effect: AccessEffect::Allow }
            if registry.rule_external_id(rule) == Some("rule-allow-edge")
    );
    std::assert_matches!(
        edge_cell(&registry, &classes, "e2", "car"),
        AccessCell::Decided {
            effect: AccessEffect::Deny,
            ..
        }
    );
}

#[test]
fn participant_specificity_precedes_target_specificity_frozen_example() {
    // SSOT §6.4 冻结示例：deny motorVehicle @ edge-1 + allow bus @ roadSection-A，
    // 对 class=bus，参与者轴先裁决：allow bus（更深）胜。
    let registry = access(vec![
        AccessRule::new(
            "rule-deny-motor",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["motorVehicle"],
        ),
        AccessRule::new(
            "rule-allow-bus",
            AccessTargetId::road_section("section-a"),
            AccessEffect::Allow,
            ["bus"],
        ),
    ]);
    let classes = classes();

    std::assert_matches!(
        edge_cell(&registry, &classes, "e1", "bus"),
        AccessCell::Decided { rule, effect: AccessEffect::Allow }
            if registry.rule_external_id(rule) == Some("rule-allow-bus")
    );
    std::assert_matches!(
        edge_cell(&registry, &classes, "e1", "car"),
        AccessCell::Decided {
            effect: AccessEffect::Deny,
            ..
        }
    );
    // section 内其他 edge 只命中 section 级 allow。
    std::assert_matches!(
        edge_cell(&registry, &classes, "e2", "bus"),
        AccessCell::Decided {
            effect: AccessEffect::Allow,
            ..
        }
    );
}

#[test]
fn explicit_priority_breaks_ties_after_specificity_axes() {
    let registry = access(vec![
        AccessRule::new(
            "rule-deny",
            AccessTargetId::road_section("section-a"),
            AccessEffect::Deny,
            ["car"],
        )
        .with_priority(1),
        AccessRule::new(
            "rule-allow",
            AccessTargetId::road_section("section-a"),
            AccessEffect::Allow,
            ["car"],
        )
        .with_priority(5),
    ]);
    let classes = classes();
    std::assert_matches!(
        edge_cell(&registry, &classes, "e1", "car"),
        AccessCell::Decided { rule, effect: AccessEffect::Allow }
            if registry.rule_external_id(rule) == Some("rule-allow")
    );

    // 负 priority 参与同一数值轴。
    let registry = access(vec![
        AccessRule::new(
            "rule-deny",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        ),
        AccessRule::new(
            "rule-allow",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Allow,
            ["car"],
        )
        .with_priority(-1),
    ]);
    std::assert_matches!(
        edge_cell(&registry, &classes, "e1", "car"),
        AccessCell::Decided {
            effect: AccessEffect::Deny,
            ..
        }
    );
}

#[test]
fn residual_allow_deny_tie_is_rejected_as_ambiguity() {
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-allow",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Allow,
                ["car"],
            ),
            AccessRule::new(
                "rule-deny",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["car"],
            ),
        ],
    )
    .expect_err("residual tie must fail");
    std::assert_matches!(
        error,
        CoreError::AccessRuleAmbiguity {
            plane,
            target_id,
            class_id,
            first_rule_id,
            second_rule_id,
        } if plane == "edge" && target_id == "e1" && class_id == "car"
            && first_rule_id == "rule-allow" && second_rule_id == "rule-deny"
    );

    // 歧义按 (edge, class) 判定：不同 class 的规则不构成歧义。
    access(vec![
        AccessRule::new(
            "rule-allow-car",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Allow,
            ["car"],
        ),
        AccessRule::new(
            "rule-deny-truck",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["truck"],
        ),
    ]);

    // 参与者 specificity 取规则内使匹配成功的最深 class：
    // [motorVehicle, bus] 对 bus profile 计 bus 深度，与 [bus] 同深。
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-allow-multi",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Allow,
                ["motorVehicle", "bus"],
            ),
            AccessRule::new(
                "rule-deny-bus",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["bus"],
            ),
        ],
    )
    .expect_err("deepest matching class must drive specificity");
    std::assert_matches!(
        error,
        CoreError::AccessRuleAmbiguity { class_id, .. } if class_id == "bus"
    );
}

#[test]
fn same_effect_tie_is_legal_and_keeps_first_rule_for_attribution() {
    let registry = access(vec![
        AccessRule::new(
            "rule-deny-1",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        ),
        AccessRule::new(
            "rule-deny-2",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        ),
    ]);
    let classes = classes();

    std::assert_matches!(
        edge_cell(&registry, &classes, "e1", "car"),
        AccessCell::Decided { rule, effect: AccessEffect::Deny }
            if registry.rule_external_id(rule) == Some("rule-deny-1")
    );
}

#[test]
fn higher_priority_rule_outranks_lower_key_allow_deny_tie_under_any_input_order() {
    // §6.4：歧义检查只在最大 key contenders 内部进行。deny/allow 在 priority 0
    // 并列，但 priority 5 的第三条规则整体胜出——任何输入排列都必须解析为
    // allow 且不报错（旧实现按 input order 扫描会在 deny 先出现时误报歧义）。
    let deny_low = || {
        AccessRule::new(
            "rule-deny-low",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        )
    };
    let allow_low = || {
        AccessRule::new(
            "rule-allow-low",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Allow,
            ["car"],
        )
    };
    let allow_high = || {
        AccessRule::new(
            "rule-allow-high",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Allow,
            ["car"],
        )
        .with_priority(5)
    };

    for rules in [
        vec![deny_low(), allow_low(), allow_high()],
        vec![deny_low(), allow_high(), allow_low()],
        vec![allow_low(), deny_low(), allow_high()],
        vec![allow_low(), allow_high(), deny_low()],
        vec![allow_high(), deny_low(), allow_low()],
        vec![allow_high(), allow_low(), deny_low()],
    ] {
        let registry = access(rules);
        let classes = classes();
        std::assert_matches!(
            edge_cell(&registry, &classes, "e1", "car"),
            AccessCell::Decided { rule, effect: AccessEffect::Allow }
                if registry.rule_external_id(rule) == Some("rule-allow-high")
        );
    }
}

#[test]
fn mixed_effects_at_max_key_are_rejected_despite_lower_key_rules() {
    // 最大 key（priority 5）处 allow/deny 混合仍是 authoring 歧义；低 key 的
    // 第三条规则不参与裁决。归因保留 input order 先声明的 contender 与首个
    // effect 相反者。
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-allow-low",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Allow,
                ["car"],
            ),
            AccessRule::new(
                "rule-deny-high",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Deny,
                ["car"],
            )
            .with_priority(5),
            AccessRule::new(
                "rule-allow-high",
                AccessTargetId::lane_edge("e1"),
                AccessEffect::Allow,
                ["car"],
            )
            .with_priority(5),
        ],
    )
    .expect_err("mixed effects at max key must fail");
    std::assert_matches!(
        error,
        CoreError::AccessRuleAmbiguity {
            plane,
            target_id,
            class_id,
            first_rule_id,
            second_rule_id,
        } if plane == "edge" && target_id == "e1" && class_id == "car"
            && first_rule_id == "rule-deny-high" && second_rule_id == "rule-allow-high"
    );
}

// ---------- path 平面 ----------

#[test]
fn path_plane_rules_do_not_expand_to_edges() {
    let registry = access(vec![AccessRule::new(
        "rule-deny-truck-left",
        AccessTargetId::maneuver_path("path-left"),
        AccessEffect::Deny,
        ["truck"],
    )]);
    let classes = classes();
    let junctions = junctions();
    let path_left = junctions
        .maneuver_path_handle("path-left")
        .expect("path-left");
    let path_straight = junctions
        .maneuver_path_handle("path-straight")
        .expect("path-straight");

    // deny truck @ 左转 path 只作用于该 path。
    std::assert_matches!(
        registry.path_access(path_left, class(&classes, "truck")),
        AccessCell::Decided { rule, effect: AccessEffect::Deny }
            if registry.rule_external_id(rule) == Some("rule-deny-truck-left")
    );
    // 共享 internal edge 的直行 path 不受影响。
    assert_eq!(
        registry.path_access(path_straight, class(&classes, "truck")),
        AccessCell::Unconstrained
    );
    // path 规则不展平进 edge 平面：共享 edge 的 edge 平面查询无约束。
    for edge_id in ["je-entry-a", "je-internal", "je-exit-left"] {
        assert_eq!(
            edge_cell(&registry, &classes, edge_id, "truck"),
            AccessCell::Unconstrained
        );
    }
    // 其他 class 无约束。
    assert_eq!(
        registry.path_access(path_left, class(&classes, "car")),
        AccessCell::Unconstrained
    );
}

#[test]
fn path_plane_residual_tie_is_rejected_with_path_attribution() {
    let error = AccessRegistry::try_new(
        &graph(),
        &junctions(),
        &cross_section(),
        &classes(),
        vec![
            AccessRule::new(
                "rule-allow",
                AccessTargetId::maneuver_path("path-left"),
                AccessEffect::Allow,
                ["truck"],
            ),
            AccessRule::new(
                "rule-deny",
                AccessTargetId::maneuver_path("path-left"),
                AccessEffect::Deny,
                ["truck"],
            ),
        ],
    )
    .expect_err("path plane tie must fail");
    std::assert_matches!(
        error,
        CoreError::AccessRuleAmbiguity { plane, target_id, class_id, .. }
            if plane == "path" && target_id == "path-left" && class_id == "truck"
    );
}

// ---------- permutation / rebind ----------

fn bus_lane_rules() -> Vec<AccessRule> {
    vec![
        AccessRule::new(
            "rule-deny-motor",
            AccessTargetId::lane_group("group-bus"),
            AccessEffect::Deny,
            ["motorVehicle"],
        ),
        AccessRule::new(
            "rule-allow-bus",
            AccessTargetId::road_section("section-a"),
            AccessEffect::Allow,
            ["bus"],
        ),
        AccessRule::new(
            "rule-deny-truck-left",
            AccessTargetId::maneuver_path("path-left"),
            AccessEffect::Deny,
            ["truck"],
        ),
        AccessRule::new(
            "rule-allow-e3",
            AccessTargetId::lane_edge("e3"),
            AccessEffect::Allow,
            ["car"],
        ),
    ]
}

/// 以 (edge/path external ID, class external ID) 对齐提取裁决语义快照。
fn access_snapshot(
    registry: &AccessRegistry,
    classes: &ParticipantClassRegistry,
    junctions: &JunctionRegistry,
) -> Vec<(String, String, Option<(String, AccessEffect)>)> {
    let mut snapshot = Vec::new();
    for edge_id in ["e1", "e2", "e3", "je-entry-a", "je-internal"] {
        for class_id in ["motorVehicle", "car", "bus", "truck", "largeBus"] {
            let cell = edge_cell(registry, classes, edge_id, class_id);
            snapshot.push((
                format!("edge:{edge_id}"),
                class_id.to_owned(),
                cell_attribution(registry, cell),
            ));
        }
    }
    for path_id in ["path-left", "path-straight"] {
        let path = junctions
            .maneuver_path_handle(path_id)
            .expect("path handle");
        for class_id in ["motorVehicle", "car", "bus", "truck", "largeBus"] {
            let cell = registry.path_access(path, class(classes, class_id));
            snapshot.push((
                format!("path:{path_id}"),
                class_id.to_owned(),
                cell_attribution(registry, cell),
            ));
        }
    }
    snapshot
}

fn cell_attribution(registry: &AccessRegistry, cell: AccessCell) -> Option<(String, AccessEffect)> {
    match cell {
        AccessCell::Unconstrained => None,
        AccessCell::Decided { rule, effect } => Some((
            registry
                .rule_external_id(rule)
                .expect("rule external ID")
                .to_owned(),
            effect,
        )),
    }
}

#[test]
fn input_permutation_preserves_external_id_aligned_semantics() {
    let canonical = access(bus_lane_rules());

    let mut permuted_rules = bus_lane_rules();
    permuted_rules.reverse();
    let permuted = access(permuted_rules);

    let junctions = junctions();
    let classes = classes();
    assert_eq!(
        access_snapshot(&canonical, &classes, &junctions),
        access_snapshot(&permuted, &classes, &junctions)
    );
}

#[test]
fn rebind_reproduces_semantics_and_validates_new_inputs() {
    let registry = access(bus_lane_rules());

    // 同定义、不同 handle 分配的新 registry 组：rebind 后语义等价。
    let rebound = registry
        .rebind(&graph(), &junctions(), &cross_section(), &classes())
        .expect("rebind must succeed");
    let junctions = junctions();
    let classes = classes();
    assert_eq!(
        access_snapshot(&registry, &classes, &junctions),
        access_snapshot(&rebound, &classes, &junctions)
    );

    // 缺失 target edge 的 graph：rebind 必须重新校验。
    let reduced_graph = LaneGraph::try_new([
        edge("e1", &["e2"]),
        edge("e2", &[]),
        edge("je-entry-a", &["je-internal"]),
        edge("je-entry-b", &["je-internal"]),
        edge("je-internal", &["je-exit-left", "je-exit-straight"]),
        edge("je-exit-left", &[]),
        edge("je-exit-straight", &[]),
    ])
    .expect("reduced graph");
    let reduced_cross_section = CrossSectionRegistry::try_new(
        &reduced_graph,
        [FacilityBand::new("band-m", "median")],
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["e1", "e2"], Some("group-bus"))],
        )],
        [LaneGroup::new("group-bus", "section-a")],
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [
                CorridorElementId::band("band-m"),
                CorridorElementId::section("section-a"),
            ],
        )],
    )
    .expect("reduced cross-section");
    let error = registry
        .rebind(&reduced_graph, &junctions, &reduced_cross_section, &classes)
        .expect_err("rebind against graph missing rule target edge must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownAccessRuleTarget { target_kind, target_id, .. }
            if target_kind == "laneEdge" && target_id == "e3"
    );
}
