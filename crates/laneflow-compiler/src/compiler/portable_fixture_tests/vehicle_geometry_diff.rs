use super::*;
use crate::{RigidTwoAxleGeometryInput, RoadVehicleGeometryInput};
use laneflow_format::RegistryCheckedFieldValue as Value;

fn input() -> RigidTwoAxleGeometryInput {
    RigidTwoAxleGeometryInput {
        wheelbase_m: 2.6,
        track_width_m: 1.601,
        reference_height_m: 0.55,
        rear_overhang_m: 0.,
        width_m: 1.901,
        bottom_height_m: -0.3,
        top_height_m: 1.5,
        max_steering_angle_rad: 0.6,
        max_steering_rate_rad_s: 0.6,
    }
}

#[test]
fn geometry_admission_preserves_signed_height_quantization_and_never_raises_limits() {
    for source in [
        0.,
        -0.,
        f64::from_bits(1),
        f64::from(0.5_f32).next_down(),
        0.5,
        0.5_f64.next_up(),
        0.6,
        std::f64::consts::FRAC_PI_2.next_down(),
    ] {
        let geometry = RigidTwoAxleGeometryInput {
            max_steering_angle_rad: source,
            max_steering_rate_rad_s: source,
            ..input()
        }
        .canonical(4_500)
        .unwrap();
        for rounded in [
            geometry.max_steering_angle_rad,
            geometry.max_steering_rate_rad_s,
        ] {
            assert!(!rounded.is_sign_negative());
            assert!(f64::from(rounded) <= source && source < f64::from(rounded.next_up()));
        }
        assert_eq!(geometry.track_width_mm, 1601);
        assert_eq!(geometry.bottom_height_mm, -300);
        assert_eq!(geometry.rear_overhang_mm, 0);
    }
    assert_eq!(
        RigidTwoAxleGeometryInput {
            max_steering_rate_rad_s: f64::MAX,
            ..input()
        }
        .canonical(4_500)
        .unwrap()
        .max_steering_rate_rad_s,
        f32::MAX
    );
    for (source, field) in [
        (
            RigidTwoAxleGeometryInput {
                wheelbase_m: 4.501,
                ..input()
            },
            0,
        ),
        (
            RigidTwoAxleGeometryInput {
                width_m: 1.600,
                ..input()
            },
            4,
        ),
        (
            RigidTwoAxleGeometryInput {
                top_height_m: -0.3,
                ..input()
            },
            6,
        ),
        (
            RigidTwoAxleGeometryInput {
                max_steering_angle_rad: std::f64::consts::FRAC_PI_2,
                ..input()
            },
            7,
        ),
        (
            RigidTwoAxleGeometryInput {
                max_steering_rate_rad_s: -f64::from_bits(1),
                ..input()
            },
            8,
        ),
        (
            RigidTwoAxleGeometryInput {
                max_steering_rate_rad_s: f64::INFINITY,
                ..input()
            },
            8,
        ),
    ] {
        assert_eq!(source.canonical(4_500).unwrap_err().field_index(), field);
    }
}

fn vehicle(geometry: RoadVehicleGeometryInput) -> VehicleProfileInput<'static> {
    VehicleProfileInput {
        vehicle_profile_key: "standard-car",
        participant_class: ParticipantClassReference::local("car"),
        iidm: canonical_iidm_profile(),
        geometry,
    }
}

fn build(geometry: RoadVehicleGeometryInput) -> Result<SyntheticModule, crate::DiagnosticBundle> {
    let mut builder = portable_fixture_builder("city/geometry", "vehicle.document");
    builder.add_lane_edge(LaneEdgeInput {
        lane_edge_key: "street",
        length_meters: 30.,
        speed_limit_meters_per_second: 10.,
        successors: &[],
    })?;
    builder.add_participant_class(ParticipantClassInput {
        participant_class_key: "car",
        extends: None,
    })?;
    builder.add_vehicle_profile(vehicle(geometry))?;
    builder.finish()
}

#[test]
fn following_only_profile_geometry_changes_identity_preserving_revision_and_exact_diff() {
    fn module(changed: bool) -> SyntheticModule {
        let mut geometry = input();
        if changed {
            geometry.track_width_m = 1.602;
        }
        build(geometry.into()).unwrap()
    }
    let doc_digest = |m: SyntheticModule| {
        *m.source_documents()
            .next()
            .unwrap()
            .source_document_digest()
    };
    assert_ne!(doc_digest(module(false)), doc_digest(module(true)));
    let (base, target) = pair(module);
    assert_ne!(base.network_revision(), target.network_revision());
    let diff = diff_view(target.semantic_diff().bytes());
    let changes = diff.section(1).unwrap().table(0).unwrap();
    assert_eq!(changes.row_count(), 1);
    let change = changes.row(0).unwrap();
    assert!(matches!(
        change.field_by_tag(1).unwrap().value().unwrap(),
        Value::U8(2)
    ));
    assert!(
        matches!(change.field_by_tag(2).unwrap().value().unwrap(), Value::U16(k) if k == EntityKind::VehicleProfile.code())
    );
    assert!(matches!(
        change.field_by_tag(6).unwrap().value().unwrap(),
        Value::U16(11)
    ));
    for (tag, candidate, track_mm) in [(9, &base, 1601), (10, &target, 1602)] {
        let artifact = artifact_view(candidate.canonical_artifact().bytes());
        let profile = artifact
            .section(2)
            .unwrap()
            .tables()
            .find(|t| t.kind() == EntityKind::VehicleProfile.code())
            .unwrap()
            .row(0)
            .unwrap();
        assert_eq!(
            change.field_by_tag(4).unwrap().value_bytes(),
            profile.field_by_tag(2).unwrap().value_bytes()
        );
        let geometry = profile.field_by_tag(11).unwrap();
        assert_eq!(geometry.value_bytes().len(), 164);
        assert_eq!(
            change.field_by_tag(tag).unwrap().value_bytes(),
            geometry.value_bytes()
        );
        let Value::RecordVector(rows) = geometry.value().unwrap() else {
            panic!("expected geometry record")
        };
        let row = rows.rows().next().unwrap();
        assert!(
            matches!(row.field_by_tag(2).unwrap().value().unwrap(), Value::U32(mm) if mm == track_mm)
        );
        assert!(
            matches!(row.field_by_tag(8).unwrap().value().unwrap(), Value::F32(angle) if angle.to_bits() == 0x3f19_9999)
        );
    }
    for i in [2, 3, 4, 5, 6, 7] {
        assert_eq!(diff.section(i).unwrap().table(0).unwrap().row_count(), 0);
    }
}

#[test]
fn unsupported_source_configurations_identify_the_vehicle_and_allow_retry() {
    let mut builder = portable_fixture_builder("city/geometry", "vehicle.document");
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .unwrap();
    for geometry in [
        RoadVehicleGeometryInput::FullTrailer,
        RoadVehicleGeometryInput::SemiTrailer,
        RoadVehicleGeometryInput::ArticulatedBus,
    ] {
        let failure = builder
            .add_vehicle_profile(vehicle(geometry))
            .err()
            .unwrap();
        let diagnostic = &failure.diagnostics()[0];
        assert!(
            matches!(diagnostic.payload(), crate::DiagnosticPayload::InvalidVehicleProfileValue {
            vehicle_profile_key, field, violation: crate::ScalarViolation::UnsupportedVehicleConfiguration, ..
        } if vehicle_profile_key.as_ref() == "standard-car" && field.as_ref() == "configuration")
        );
        assert_eq!(
            diagnostic.primary_span().unwrap().source_document_key(),
            "vehicle.document"
        );
    }
    builder
        .add_vehicle_profile(vehicle(input().into()))
        .unwrap();
    builder.finish().unwrap();
}
