use super::*;

const FIXTURE_PROVENANCE: &str = "laneflow-fixture-513-migration-v1";

fn migration_module(target: bool) -> SyntheticModule {
    let mut builder =
        portable_fixture_builder("city/portable-migration", "portable-migration.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: if target { 60.0 } else { 100.0 },
            speed_limit_meters_per_second: if target { 8.0 } else { 20.0 },
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 100.0,
            speed_limit_meters_per_second: 20.0,
            successors: &[],
        })
        .unwrap();
    if !target {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "doomed",
                length_meters: 50.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap();
    }
    builder
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "area-main",
            virtual_capacity: 0,
            virtual_entries: &[],
            virtual_exits: &[],
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-main",
            parking_facility: Some(ParkingFacilityReference::local("area-main")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 6.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .unwrap();
    if !target {
        builder
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space-doomed",
                parking_facility: Some(ParkingFacilityReference::local("area-main")),
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("entry"),
                    progress_meters: 8.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("entry"),
                    progress_meters: 10.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: 3.0,
                    heading_offset_radians: -0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .unwrap();
    }
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "passenger-car",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap();
    if target {
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: "restrict-exit",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("exit")),
                effect: AccessEffect::Deny,
                participant_classes: &[ParticipantClassReference::local("passenger-car")],
                regulation: None,
                priority: 10,
            })
            .unwrap();
    }
    builder
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("passenger-car"),
            iidm: canonical_iidm_profile(),
        })
        .unwrap();
    builder.finish().unwrap()
}

fn oracle_module(target: bool) -> SyntheticModule {
    let frame_points_a = [
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 100.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let frame_points_b = [
        CanonicalPoint3F32Input {
            x: 100.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 200.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let geometries = [
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("entry"),
            centerline_points: &frame_points_a,
        },
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("exit"),
            centerline_points: &frame_points_b,
        },
    ];
    let mut builder = portable_fixture_builder(
        "city/portable-migration-oracle",
        "portable-migration-oracle.document",
    );
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 100.0,
            speed_limit_meters_per_second: 20.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 100.0,
            speed_limit_meters_per_second: 20.0,
            successors: &[],
        })
        .unwrap()
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "area-main",
            virtual_capacity: 0,
            virtual_entries: &[],
            virtual_exits: &[],
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-main",
            parking_facility: Some(ParkingFacilityReference::local("area-main")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 6.0,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "passenger-car",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("passenger-car"),
            iidm: canonical_iidm_profile(),
        })
        .unwrap();
    if target {
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &geometries,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn profile_drift_module(target: bool) -> SyntheticModule {
    let iidm = IidmVehicleProfileInput {
        length_meters: if target { 6.0 } else { 4.5 },
        desired_speed_meters_per_second: 13.75,
        min_gap_meters: 2.0,
        time_headway_seconds: 1.4,
        max_acceleration_meters_per_second_squared: 1.8,
        comfortable_deceleration_meters_per_second_squared: 2.0,
        emergency_deceleration_meters_per_second_squared: 4.5,
    };
    let mut builder = portable_fixture_builder(
        "city/portable-migration-drift",
        "portable-migration-drift.document",
    );
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 100.0,
            speed_limit_meters_per_second: 20.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 100.0,
            speed_limit_meters_per_second: 20.0,
            successors: &[],
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "passenger-car",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("passenger-car"),
            iidm,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn pair(
    module: fn(bool) -> SyntheticModule,
) -> (
    crate::PortablePublicationCandidate,
    crate::PortablePublicationCandidate,
) {
    let provenance = crate::PortableEmissionProvenance::try_new(FIXTURE_PROVENANCE).unwrap();
    let base_output = Compiler::new().compile(unit([module(false)])).unwrap();
    let base_candidate = crate::emit_portable_candidate(
        &base_output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    let base = laneflow_format::preflight_object_values(
        base_candidate.canonical_artifact().bytes(),
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    let target_output = Compiler::new().compile(unit([module(true)])).unwrap();
    let target_candidate = crate::emit_portable_candidate(
        &target_output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();
    (base_candidate, target_candidate)
}

fn migration_pair() -> (
    crate::PortablePublicationCandidate,
    crate::PortablePublicationCandidate,
) {
    pair(migration_module)
}

fn profile_drift_pair() -> (
    crate::PortablePublicationCandidate,
    crate::PortablePublicationCandidate,
) {
    pair(profile_drift_module)
}

fn oracle_pair() -> (
    crate::PortablePublicationCandidate,
    crate::PortablePublicationCandidate,
) {
    pair(oracle_module)
}

const MIGRATION_BASE: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/base.lfca");
const MIGRATION_TARGET: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/target.lfca");
const MIGRATION_LFSD: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/expected.lfsd");
const ORACLE_BASE: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/oracle-base.lfca");
const ORACLE_TARGET: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/oracle-target.lfca");
const ORACLE_LFSD: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/oracle-expected.lfsd");
const PROFILE_BASE: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/profile-base.lfca");
const PROFILE_TARGET: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/profile-target.lfca");
const PROFILE_LFSD: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-migration/profile-expected.lfsd");

fn artifact_view(bytes: &[u8]) -> laneflow_format::RegistryCheckedObjectView<'_> {
    laneflow_format::preflight_object_values(
        bytes,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view()
}

fn diff_view(bytes: &[u8]) -> laneflow_format::RegistryCheckedObjectView<'_> {
    laneflow_format::preflight_object_values(
        bytes,
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view()
}

#[test]
fn portable_migration_pair_matches_frozen_exact_bytes() {
    let (base, target) = migration_pair();
    assert_eq!(base.canonical_artifact().bytes(), MIGRATION_BASE);
    assert_eq!(target.canonical_artifact().bytes(), MIGRATION_TARGET);
    assert_eq!(target.semantic_diff().bytes(), MIGRATION_LFSD);
    assert_eq!(
        target.semantic_diff().object_key(),
        "sha256/2c7eb0febf649773ce867f84b47684435918108ad667424cd1d26fcc86d54e97"
    );
    assert_ne!(base.network_revision(), target.network_revision());

    let (obase, otarget) = oracle_pair();
    assert_eq!(obase.canonical_artifact().bytes(), ORACLE_BASE);
    assert_eq!(otarget.canonical_artifact().bytes(), ORACLE_TARGET);
    assert_eq!(otarget.semantic_diff().bytes(), ORACLE_LFSD);
    assert_eq!(
        otarget.semantic_diff().object_key(),
        "sha256/5e40d2de4bda10ab36acf1498f4a734c70f192b9faaad4daeaaf57426637d4b2"
    );
    assert_ne!(obase.network_revision(), otarget.network_revision());

    // profile 漂移对：target 仅改 standard-car 车长（4.5 m → 6.0 m），
    // 派生不变量重验证的不可映射场景（运行时侧消费）。
    let (pbase, ptarget) = profile_drift_pair();
    assert_eq!(pbase.canonical_artifact().bytes(), PROFILE_BASE);
    assert_eq!(ptarget.canonical_artifact().bytes(), PROFILE_TARGET);
    assert_eq!(ptarget.semantic_diff().bytes(), PROFILE_LFSD);
    assert_eq!(
        ptarget.semantic_diff().object_key(),
        "sha256/ab070a572a2ebbaa06ed8f5f87c341d0839e872c23b055debd5d1f2d1ade291b"
    );
    assert_ne!(pbase.network_revision(), ptarget.network_revision());
}

#[test]
fn portable_migration_pair_carries_expected_semantics() {
    // oracle 对只有空间侧变更：契约、关系与静态执行约束段逐字节相等；
    // 身份/实体表/空间段只承载 CanonicalFrame 追加（交通种类序数不受
    // 影响，由运行时侧恒等 oracle 功能性验证）。直移恒等 oracle 的前提。
    let base_view = artifact_view(ORACLE_BASE);
    let target_view = artifact_view(ORACLE_TARGET);
    for ordinal in [0_u32, 3, 5] {
        assert_eq!(
            base_view.section(ordinal).unwrap().bytes(),
            target_view.section(ordinal).unwrap().bytes(),
            "oracle traffic section {ordinal} must be byte-equal"
        );
    }
    for ordinal in [1_u32, 2, 4] {
        assert_ne!(
            base_view.section(ordinal).unwrap().bytes(),
            target_view.section(ordinal).unwrap().bytes(),
            "oracle pair must register the canonical frame in section {ordinal}"
        );
    }

    let diff = diff_view(ORACLE_LFSD);
    // oracle LFSD 的实体变更只允许 CanonicalFrame（kind 22）新增；关系与
    // 静态规则变更必须为空。
    let entity_changes = diff.section(1).unwrap().table(0).unwrap();
    for ordinal in 0..entity_changes.row_count() {
        let row = entity_changes.row(ordinal).unwrap();
        assert!(
            matches!(
                row.field_by_tag(2).unwrap().value().unwrap(),
                laneflow_format::RegistryCheckedFieldValue::U16(22)
            ),
            "oracle entity change must be a canonical frame"
        );
    }
    for section_ordinal in [2_u32, 4] {
        assert_eq!(
            diff.section(section_ordinal)
                .unwrap()
                .table(0)
                .unwrap()
                .row_count(),
            0,
            "oracle diff section {section_ordinal} must stay empty"
        );
    }

    // 迁移对携带语义变更：实体（doomed 边/车位移除）与关系（访问规则
    // 新增）变更段非空。
    let migration_diff = diff_view(MIGRATION_LFSD);
    assert!(
        migration_diff
            .section(1)
            .unwrap()
            .table(0)
            .unwrap()
            .row_count()
            > 0
    );
    assert!(
        migration_diff
            .section(2)
            .unwrap()
            .table(0)
            .unwrap()
            .row_count()
            > 0
    );
}

#[test]
fn dump_portable_migration_when_requested() {
    if std::env::var_os("DUMP_PORTABLE").is_none() {
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable/lfsd-migration");
    std::fs::create_dir_all(&dir).unwrap();
    let (base, target) = migration_pair();
    std::fs::write(dir.join("base.lfca"), base.canonical_artifact().bytes()).unwrap();
    std::fs::write(dir.join("target.lfca"), target.canonical_artifact().bytes()).unwrap();
    std::fs::write(dir.join("expected.lfsd"), target.semantic_diff().bytes()).unwrap();
    let (obase, otarget) = oracle_pair();
    std::fs::write(
        dir.join("oracle-base.lfca"),
        obase.canonical_artifact().bytes(),
    )
    .unwrap();
    std::fs::write(
        dir.join("oracle-target.lfca"),
        otarget.canonical_artifact().bytes(),
    )
    .unwrap();
    std::fs::write(
        dir.join("oracle-expected.lfsd"),
        otarget.semantic_diff().bytes(),
    )
    .unwrap();
    let (pbase, ptarget) = profile_drift_pair();
    std::fs::write(
        dir.join("profile-base.lfca"),
        pbase.canonical_artifact().bytes(),
    )
    .unwrap();
    std::fs::write(
        dir.join("profile-target.lfca"),
        ptarget.canonical_artifact().bytes(),
    )
    .unwrap();
    std::fs::write(
        dir.join("profile-expected.lfsd"),
        ptarget.semantic_diff().bytes(),
    )
    .unwrap();
    std::fs::write(
        dir.join("bindings.txt"),
        format!(
            "migration_base_len={}\nmigration_base_key={}\nmigration_target_len={}\n\
             migration_target_key={}\nmigration_lfsd_len={}\nmigration_lfsd_key={}\n\
             oracle_base_len={}\noracle_base_key={}\noracle_target_len={}\n\
             oracle_target_key={}\noracle_lfsd_len={}\noracle_lfsd_key={}\n\
             drift_base_len={}\ndrift_base_key={}\ndrift_target_len={}\n\
             drift_target_key={}\ndrift_lfsd_len={}\ndrift_lfsd_key={}\n",
            base.canonical_artifact().bytes().len(),
            base.canonical_artifact().object_key(),
            target.canonical_artifact().bytes().len(),
            target.canonical_artifact().object_key(),
            target.semantic_diff().bytes().len(),
            target.semantic_diff().object_key(),
            obase.canonical_artifact().bytes().len(),
            obase.canonical_artifact().object_key(),
            otarget.canonical_artifact().bytes().len(),
            otarget.canonical_artifact().object_key(),
            otarget.semantic_diff().bytes().len(),
            otarget.semantic_diff().object_key(),
            pbase.canonical_artifact().bytes().len(),
            pbase.canonical_artifact().object_key(),
            ptarget.canonical_artifact().bytes().len(),
            ptarget.canonical_artifact().object_key(),
            ptarget.semantic_diff().bytes().len(),
            ptarget.semantic_diff().object_key(),
        ),
    )
    .unwrap();
}
