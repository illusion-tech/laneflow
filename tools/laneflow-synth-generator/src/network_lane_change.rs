//! L1 换道网络发射（#1218）：cell 背景（车道边 + 停车位）+ 恰一横向过渡段的
//! 换道簇（R1b s12 钉版载体形制）+ 编辑侧唯一策略集。
//!
//! 独立于 `network.rs` 的原因（冻结面）：LFCA 规范制品嵌入发射调用点行号，
//! `fixtures/probe-vectors.json` 冻结的 W 族 network.lfca 字节要求
//! `network.rs` 逐字保持；本文件的行号只影响 L1 制品自身。

use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};

use crate::encoding::{config_bytes, digest};
use crate::{BUILD_ID, Error, Identity, NAMESPACE, compile_error, key};

/// L1 换道簇的边键；恰一横向过渡段 `lc-t`（R1b s12 钉版载体形制）。
pub(crate) const LC_EDGES: [&str; 8] = [
    "lc-e0", "lc-t", "lc-e2", "lc-int", "lc-exit", "lc-e3", "lc-e4", "lc-e5",
];
/// 换道者路线边链（e0→t→e2→int→exit），与 R1b 钉版同形。
pub(crate) const LC_ROUTE: [&str; 5] = ["lc-e0", "lc-t", "lc-e2", "lc-int", "lc-exit"];
/// 编辑侧模块命名空间（junction/zone/stream/policy 的稳定标识来源）。
pub(crate) const LC_EDIT_NAMESPACE: &str = "lf-synth-road";

pub(crate) fn compile(identity: &Identity) -> Result<Vec<u8>, Error> {
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "lf-synth-lane-change.document",
            generator_build_id: BUILD_ID,
            parameters_and_inputs_digest: digest(&config_bytes(identity)?),
            frontend_options_digest: digest(b"lf-synth-frontend-1"),
            random_seed: Some(0),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .map_err(|e| compile_error("source header", e))?;
    let mut builder = SyntheticModuleBuilder::new(header, &limits)
        .map_err(|e| compile_error("synthetic module", e))?;
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .map_err(|e| compile_error("participant class", e))?;
    // 换道者与背景共用车型；钉版参数承接 R1b s12 权威口径（a_service 4.5、
    // min_gap 2.5、d_stop(10 m/s)=15.61 m 前提）。
    builder
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 15.0,
                min_gap_meters: 2.5,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.5,
                comfortable_deceleration_meters_per_second_squared: 4.5,
                emergency_deceleration_meters_per_second_squared: 9.0,
            },
        })
        .map_err(|e| compile_error("vehicle profile", e))?;
    let mut geometry = Vec::new();
    for cell in 0..identity.cells() {
        let facility = key(cell, 0, "facility");
        builder
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: &facility,
                virtual_capacity: 0,
                virtual_entries: &[],
                virtual_exits: &[],
            })
            .map_err(|e| compile_error("parking facility", e))?;
        // L1 背景只保留车道边 + 停车位（D1：L 族自有背景语义）：cell 门/信号
        // 不入网，换道簇的门/流由编辑侧唯一策略集全覆盖（策略校验要求每个
        // 策略覆盖全网门与流，而合成侧门无法被编辑侧规则引用）。
        add_background_route(&mut builder, cell, &facility)?;
        for route in 0..4 {
            let [entry, exit] = crate::network::centerlines(cell, route);
            geometry.push((key(cell, route, "entry"), entry));
            geometry.push((key(cell, route, "exit"), exit));
        }
    }
    add_lane_change_cluster(&mut builder)?;
    let geometry_refs: Vec<_> = geometry
        .iter()
        .map(|(name, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(name),
            centerline_points: points,
        })
        .collect();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &geometry_refs,
        })
        .map_err(|e| compile_error("canonical frame", e))?;
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(
        builder
            .finish()
            .map_err(|e| compile_error("finish module", e))?,
    )
    .map_err(|e| compile_error("compilation unit", e))?;
    let editing_bytes = lane_change_editing(&limits)?;
    unit.add_road_editing_module(
        laneflow_compiler::road_editing::RoadEditingModuleInput::try_new(
            "lf-synth-road.document",
            &editing_bytes,
            None,
        )
        .map_err(|e| compile_error("editing module input", e))?,
    )
    .map_err(|e| compile_error("editing unit", e))?;
    let output = Compiler::new()
        .compile(unit.build().map_err(|e| compile_error("build unit", e))?)
        .map_err(|e| compile_error("compile", e))?;
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new(BUILD_ID)
            .map_err(|e| compile_error("provenance", e))?,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|e| compile_error("emit", e))?;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(|e| compile_error("post-emission", e))?;
    Ok(candidate.canonical_artifact().bytes().to_vec())
}

/// L1 背景路线（#1218 D1）：每 cell 四条路线，仅车道边 + 25 停车位，无机动/门/信号。
fn add_background_route(
    b: &mut SyntheticModuleBuilder,
    c: u32,
    facility: &str,
) -> Result<(), Error> {
    for route in 0..4 {
        let entry = key(c, route, "entry");
        let exit = key(c, route, "exit");
        b.add_lane_edge(LaneEdgeInput {
            lane_edge_key: &entry,
            length_meters: 5_130.0,
            speed_limit_meters_per_second: 13.9,
            successors: &[LaneEdgeReference::local(&exit)],
        })
        .map_err(|e| compile_error("background entry edge", e))?;
        b.add_lane_edge(LaneEdgeInput {
            lane_edge_key: &exit,
            length_meters: 5_130.0,
            speed_limit_meters_per_second: 13.9,
            successors: &[],
        })
        .map_err(|e| compile_error("background exit edge", e))?;
        for s in 0..25 {
            let anchor = || ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(&exit),
                progress_meters: 5.0 + 7.5 * f64::from(s),
            };
            b.add_parking_space(ParkingSpaceInput {
                parking_space_key: &key(c, route, &format!("space{s:02}")),
                parking_facility: Some(ParkingFacilityReference::local(facility)),
                entry: anchor(),
                exit: anchor(),
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: 3.0,
                    heading_offset_radians: 0.0,
                    length_meters: 5.0,
                    width_meters: 2.4,
                },
            })
            .map_err(|e| compile_error("background parking space", e))?;
        }
    }
    Ok(())
}

/// L1 换道簇的合成侧实体（#1218）：恰一横向过渡段 `lc-t` + 多车道 section/走廊
///（分类判据来源）。R1b s12 钉版载体形制逐项保持（边长/接缝/几何跨度）。
fn add_lane_change_cluster(b: &mut SyntheticModuleBuilder) -> Result<(), Error> {
    for (edge_key, length, successors) in [
        ("lc-e0", 30.0_f64, vec!["lc-t"]),
        ("lc-t", 5.0, vec!["lc-e2"]),
        ("lc-e2", 30.0, vec![]),
        ("lc-int", 8.0, vec![]),
        ("lc-exit", 10.0, vec!["lc-e5"]),
        ("lc-e3", 30.0, vec![]),
        ("lc-e4", 30.0, vec!["lc-e2"]),
        ("lc-e5", 30.0, vec![]),
    ] {
        let refs: Vec<_> = successors
            .iter()
            .map(|target| LaneEdgeReference::local(target))
            .collect();
        b.add_lane_edge(LaneEdgeInput {
            lane_edge_key: edge_key,
            length_meters: length,
            speed_limit_meters_per_second: 13.9,
            successors: &refs,
        })
        .map_err(|e| compile_error("lane change edge", e))?;
    }
    let chain = |edges: &[&'static str]| -> Vec<LaneEdgeReference<'static>> {
        edges.iter().map(|e| LaneEdgeReference::local(e)).collect()
    };
    let lane0 = chain(&["lc-e0"]);
    let lane1 = chain(&["lc-e4", "lc-e2"]);
    let lane2 = chain(&["lc-e3"]);
    let exit_lane = chain(&["lc-exit", "lc-e5"]);
    b.add_road_section(RoadSectionInput {
        road_section_key: "lc-section",
        kind_id: "motorLane",
        lanes: &[
            AuthoringLaneInput {
                authoring_lane_key: "lc-lane-0",
                edge_chain: &lane0,
                lane_group: None,
            },
            AuthoringLaneInput {
                authoring_lane_key: "lc-lane-1",
                edge_chain: &lane1,
                lane_group: None,
            },
            AuthoringLaneInput {
                authoring_lane_key: "lc-lane-2",
                edge_chain: &lane2,
                lane_group: None,
            },
        ],
    })
    .map_err(|e| compile_error("lane change section", e))?;
    b.add_road_section(RoadSectionInput {
        road_section_key: "lc-exit-section",
        kind_id: "motorLane",
        lanes: &[AuthoringLaneInput {
            authoring_lane_key: "lc-exit-lane",
            edge_chain: &exit_lane,
            lane_group: None,
        }],
    })
    .map_err(|e| compile_error("lane change exit section", e))?;
    b.add_road_corridor(RoadCorridorInput {
        road_corridor_key: "lc-corridor",
        reference_section: RoadSectionReference::local("lc-section"),
        elements: &[CorridorElementReference::road_section(
            RoadSectionReference::local("lc-section"),
        )],
    })
    .map_err(|e| compile_error("lane change corridor", e))?;
    b.add_road_corridor(RoadCorridorInput {
        road_corridor_key: "lc-exit-corridor",
        reference_section: RoadSectionReference::local("lc-exit-section"),
        elements: &[CorridorElementReference::road_section(
            RoadSectionReference::local("lc-exit-section"),
        )],
    })
    .map_err(|e| compile_error("lane change exit corridor", e))?;
    // 帧几何逐边跨度承接 R1b 钉版载体（T 段 x=30..35，冲突区多边形同坐标域）。
    let frame_spans = [
        (0.0_f32, 30.0_f32),
        (30.0, 35.0),
        (35.0, 65.0),
        (65.0, 73.0),
        (73.0, 83.0),
        (35.0, 65.0),
        (5.0, 35.0),
        (83.0, 113.0),
    ];
    let point_pairs: Vec<[CanonicalPoint3F32Input; 2]> = frame_spans
        .iter()
        .map(|(x0, x1)| {
            [
                CanonicalPoint3F32Input {
                    x: *x0,
                    y: 0.0,
                    z: 0.0,
                },
                CanonicalPoint3F32Input {
                    x: *x1,
                    y: 0.0,
                    z: 0.0,
                },
            ]
        })
        .collect();
    let geometries: Vec<_> = LC_EDGES
        .iter()
        .zip(point_pairs.iter())
        .map(|(edge_name, centerline_points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge_name),
            centerline_points,
        })
        .collect();
    b.add_canonical_frame(CanonicalFrameInput {
        canonical_frame_key: "lc-t-frame",
        lane_edge_geometries: &geometries,
    })
    .map_err(|e| compile_error("lane change frame", e))?;
    Ok(())
}

/// L1 换道簇的编辑侧声明（#1218）：junction/冲突区/两条流/无信号准入门 + 策略
///（gap profile + 流规则 + 门规则）。返回规范编辑源字节；L1 唯一策略集覆盖全部门
///（编译期 MissingRule 校验），L 族确定性比较在 L 族内部逐运行相等（D1）。
fn lane_change_editing(limits: &CompileLimits) -> Result<Vec<u8>, Error> {
    use laneflow_compiler::road_editing as editing;

    let header = editing::RoadEditingModuleHeader::try_new(
        LC_EDIT_NAMESPACE,
        "lf-synth-road.document",
        vec![NAMESPACE.to_owned()],
        editing::RoadEditingProvenance::direct("lf-synth lane-change v1")
            .map_err(|e| compile_error("editing provenance", e))?,
    )
    .map_err(|e| compile_error("editing header", e))?;
    let mut builder = editing::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )
    .map_err(|e| compile_error("editing module", e))?;
    let imported = |key_name: &'static str| {
        editing::LaneEdgeReference::imported(NAMESPACE, Vec::new(), key_name)
            .map_err(|e| compile_error("imported edge", e))
    };
    let junction = editing::JunctionReference::local("lc-junction")
        .map_err(|e| compile_error("junction ref", e))?;
    let zone =
        editing::ConflictZoneReference::owner_scoped(vec!["lc-junction".to_owned()], "lc-zone")
            .map_err(|e| compile_error("zone ref", e))?;
    builder
        .add_declaration(editing::RoadEditingDeclaration::Junction(
            editing::JunctionInput::try_new(
                "lc-junction",
                vec![
                    imported("lc-e0")?,
                    imported("lc-e2")?,
                    imported("lc-e3")?,
                    imported("lc-exit")?,
                ],
                vec![imported("lc-int")?],
            )
            .map_err(|e| compile_error("junction", e))?,
        ))
        .map_err(|e| compile_error("junction declaration", e))?;
    builder
        .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
            editing::ConflictZoneInput::try_new("lc-zone", junction.clone())
                .map_err(|e| compile_error("zone", e))?,
        ))
        .map_err(|e| compile_error("zone declaration", e))?;
    let mut gate_rules = Vec::new();
    let mut stream_rules = Vec::new();
    for (entry_edge, movement_key, path_key, stop_key, gate_key, stream_key) in [
        (
            "lc-e2",
            "lc-movement",
            "lc-path",
            "lc-stop",
            "lc-admission",
            "lc-stream",
        ),
        (
            "lc-e3",
            "lc-movement-2",
            "lc-path-2",
            "lc-stop-2",
            "lc-admission-2",
            "lc-stream-2",
        ),
    ] {
        let movement =
            editing::MovementReference::owner_scoped(vec!["lc-junction".to_owned()], movement_key)
                .map_err(|e| compile_error("movement ref", e))?;
        let path = editing::ManeuverPathReference::owner_scoped(
            vec!["lc-junction".to_owned(), movement_key.to_owned()],
            path_key,
        )
        .map_err(|e| compile_error("path ref", e))?;
        let admission_gate = editing::ManeuverGateReference::owner_scoped(
            vec![
                "lc-junction".to_owned(),
                movement_key.to_owned(),
                path_key.to_owned(),
            ],
            gate_key,
        )
        .map_err(|e| compile_error("gate ref", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::Movement(
                editing::MovementInput::try_new(
                    movement_key,
                    junction.clone(),
                    entry_edge.to_owned(),
                    "lc-exit".to_owned(),
                )
                .map_err(|e| compile_error("movement", e))?,
            ))
            .map_err(|e| compile_error("movement declaration", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                editing::ManeuverPathInput::try_new(
                    path_key,
                    movement.clone(),
                    imported(entry_edge)?,
                    vec![imported("lc-int")?],
                    imported("lc-exit")?,
                )
                .map_err(|e| compile_error("path", e))?,
            ))
            .map_err(|e| compile_error("path declaration", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::StopLine(
                editing::StopLineInput::try_new(stop_key, imported(entry_edge)?)
                    .map_err(|e| compile_error("stop line", e))?,
            ))
            .map_err(|e| compile_error("stop line declaration", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                editing::ManeuverGateInput::try_new(
                    gate_key,
                    path.clone(),
                    0,
                    editing::StopLineReference::local(stop_key)
                        .map_err(|e| compile_error("stop ref", e))?,
                    editing::RoadEditingSignalControl::None,
                )
                .map_err(|e| compile_error("gate", e))?,
            ))
            .map_err(|e| compile_error("gate declaration", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                editing::ParticipantStreamInput::try_new(
                    stream_key,
                    junction.clone(),
                    path.clone(),
                    vec![editing::ConflictPassageInput::new(
                        zone.clone(),
                        editing::PathAnchorInput::gate(admission_gate.clone()),
                        editing::PathAnchorInput::edge_boundary(2),
                    )],
                )
                .map_err(|e| compile_error("stream", e))?,
            ))
            .map_err(|e| compile_error("stream declaration", e))?;
        gate_rules.push(
            editing::PolicyGateRuleInput::try_new(
                gate_key,
                admission_gate,
                None,
                GateInterpretation::Uncontrolled,
                GateProhibition::None,
                vec![],
            )
            .map_err(|e| compile_error("gate rule", e))?,
        );
        stream_rules.push(
            editing::PolicyStreamRuleInput::try_new(
                stream_key,
                editing::ParticipantStreamReference::owner_scoped(
                    vec!["lc-junction".to_owned()],
                    stream_key,
                )
                .map_err(|e| compile_error("stream ref", e))?,
                None,
                0,
                vec![],
                None,
                vec![],
            )
            .map_err(|e| compile_error("stream rule", e))?,
        );
    }
    builder
        .add_declaration(editing::RoadEditingDeclaration::RightOfWayPolicySet(
            editing::RightOfWayPolicySetInput::try_new(
                "policy",
                RegulationIdentity::try_new("engineering", "lf-synth-v1")
                    .map_err(|e| compile_error("regulation", e))?
                    .with_source("repository:laneflow")
                    .map_err(|e| compile_error("regulation source", e))?,
                vec![],
                vec![
                    editing::PolicyGapProfileInput::try_new(
                        "calibration-gap",
                        "lf-synth-v1",
                        500,
                        500,
                        100,
                    )
                    .map_err(|e| compile_error("gap profile", e))?,
                ],
                stream_rules,
                gate_rules,
            )
            .map_err(|e| compile_error("policy set", e))?,
        ))
        .map_err(|e| compile_error("policy declaration", e))?;
    let region_points = vec![
        editing::RoadEditingPoint2::try_new(0.0, -2.0)
            .map_err(|e| compile_error("region point", e))?,
        editing::RoadEditingPoint2::try_new(30.0, -2.0)
            .map_err(|e| compile_error("region point", e))?,
        editing::RoadEditingPoint2::try_new(30.0, 2.0)
            .map_err(|e| compile_error("region point", e))?,
        editing::RoadEditingPoint2::try_new(0.0, 2.0)
            .map_err(|e| compile_error("region point", e))?,
    ];
    builder
        .add_conflict_zone_region(
            editing::ConflictZoneRegionInput::try_new(
                zone,
                editing::CanonicalFrameReference::imported(NAMESPACE, Vec::new(), "lc-t-frame")
                    .map_err(|e| compile_error("frame ref", e))?,
                -1.0,
                1.0,
                region_points,
            )
            .map_err(|e| compile_error("conflict region", e))?,
        )
        .map_err(|e| compile_error("conflict region declaration", e))?;
    let written = editing::RoadEditingSourceWriter::new(limits)
        .write(
            builder
                .finish()
                .map_err(|e| compile_error("editing finish", e))?,
        )
        .map_err(|e| compile_error("editing write", e))?;
    Ok(written.as_bytes().to_vec())
}
