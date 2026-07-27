use std::{fmt::Debug, hash::Hash};

use laneflow_core::{
    CoreError, IidmProfileSpec, ParticipantClass, ParticipantClassHandle, ParticipantClassRegistry,
    VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
};

const CURRENT_MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS: f64 = 1.0e-9;

fn canonical_spec() -> IidmProfileSpec {
    IidmProfileSpec {
        length: 4.5,
        desired_speed: 13.9,
        min_gap: 2.0,
        time_headway: 1.5,
        max_acceleration: 1.5,
        comfortable_deceleration: 2.0,
        emergency_deceleration: 6.0,
    }
}

fn profile(id: &str) -> VehicleProfile {
    VehicleProfile::try_new_iidm(id, participant_classes().1, canonical_spec())
        .expect("valid IIDM profile")
}

#[test]
fn valid_iidm_profile_preserves_external_id_and_parameters() {
    let profile = profile("passenger-car");

    assert_eq!(profile.external_id(), "passenger-car");
    assert_eq!(profile.iidm(), canonical_spec());
}

#[test]
fn profile_external_id_uses_shared_ascii_token_rule() {
    let error =
        VehicleProfile::try_new_iidm("passenger car", participant_classes().1, canonical_spec())
            .expect_err("invalid profile id must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId {
            field,
            external_id,
            ..
        } if field == "vehicleProfiles[].id" && external_id == "passenger car"
    );
}

#[test]
fn profile_rejects_non_finite_and_non_positive_values() {
    let cases = [
        ("length", f64::NAN),
        ("desiredSpeed", f64::INFINITY),
        ("timeHeadway", f64::NEG_INFINITY),
        ("maxAcceleration", 0.0),
        ("comfortableDeceleration", -1.0),
        ("emergencyDeceleration", 0.0),
    ];

    for (field, value) in cases {
        let mut spec = canonical_spec();
        match field {
            "length" => spec.length = value,
            "desiredSpeed" => spec.desired_speed = value,
            "timeHeadway" => spec.time_headway = value,
            "maxAcceleration" => spec.max_acceleration = value,
            "comfortableDeceleration" => spec.comfortable_deceleration = value,
            "emergencyDeceleration" => spec.emergency_deceleration = value,
            _ => unreachable!("all cases use known fields"),
        }

        let error = VehicleProfile::try_new_iidm("profile", participant_classes().1, spec)
            .expect_err("invalid profile value must fail");
        std::assert_matches!(
            error,
            CoreError::InvalidVehicleProfileValue {
                field: actual_field,
                value: actual_value,
                ..
            } if actual_field == field
                && (actual_value == value || actual_value.is_nan() && value.is_nan())
        );
    }
}

#[test]
fn profile_length_must_exceed_its_domain_minimum() {
    for length in [
        -0.0,
        0.0,
        CURRENT_MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS.next_down(),
        CURRENT_MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS,
    ] {
        let spec = IidmProfileSpec {
            length,
            ..canonical_spec()
        };
        let error = VehicleProfile::try_new_iidm("profile", participant_classes().1, spec)
            .expect_err("length at or below epsilon must fail");

        std::assert_matches!(
            error,
            CoreError::InvalidVehicleProfileValue {
                field,
                value,
                requirement,
                ..
            } if field == "length"
                && value == length
                && requirement.contains("GEOMETRY_GAP_EPSILON")
        );
    }

    let adjacent_valid = IidmProfileSpec {
        length: CURRENT_MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS.next_up(),
        ..canonical_spec()
    };
    VehicleProfile::try_new_iidm("adjacent-valid", participant_classes().1, adjacent_valid)
        .expect("value adjacent above the exclusive minimum must pass");
}

#[test]
fn profile_min_gap_allows_zero_but_rejects_negative_or_non_finite() {
    let zero_gap = IidmProfileSpec {
        min_gap: 0.0,
        ..canonical_spec()
    };
    VehicleProfile::try_new_iidm("zero-gap", participant_classes().1, zero_gap)
        .expect("zero min gap is valid");

    for min_gap in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let spec = IidmProfileSpec {
            min_gap,
            ..canonical_spec()
        };
        let error = VehicleProfile::try_new_iidm("invalid-gap", participant_classes().1, spec)
            .expect_err("invalid min gap must fail");
        std::assert_matches!(
            error,
            CoreError::InvalidVehicleProfileValue { field, value, .. }
                if field == "minGap"
                    && (value == min_gap || value.is_nan() && min_gap.is_nan())
        );
    }
}

#[test]
fn emergency_deceleration_must_cover_comfortable_deceleration() {
    let spec = IidmProfileSpec {
        comfortable_deceleration: 4.0,
        emergency_deceleration: 3.0,
        ..canonical_spec()
    };
    let error = VehicleProfile::try_new_iidm("invalid-braking", participant_classes().1, spec)
        .expect_err("invalid deceleration order must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidVehicleProfileDecelerationOrder {
            profile_id,
            comfortable_deceleration: 4.0,
            emergency_deceleration: 3.0,
        } if profile_id == "invalid-braking"
    );
}

#[test]
fn registry_assigns_stable_input_order_handles_and_resolves_both_directions() {
    let registry = VehicleProfileRegistry::try_new(
        &participant_classes().0,
        [profile("truck"), profile("passenger-car"), profile("bus")],
    )
    .expect("valid profile registry");

    assert_eq!(registry.len(), 3);
    assert_eq!(
        registry
            .profiles()
            .map(VehicleProfile::external_id)
            .collect::<Vec<_>>(),
        ["truck", "passenger-car", "bus"]
    );

    let passenger_car = registry
        .profile_handle("passenger-car")
        .expect("profile handle exists");
    assert_eq!(
        registry.profile_external_id(passenger_car),
        Some("passenger-car")
    );
    assert_eq!(
        registry
            .profile(passenger_car)
            .map(VehicleProfile::external_id),
        Some("passenger-car")
    );
    assert_eq!(registry.profile_handle("missing"), None);
}

#[test]
fn duplicate_profile_id_is_rejected_in_input_order() {
    let error = VehicleProfileRegistry::try_new(
        &participant_classes().0,
        [profile("truck"), profile("passenger-car"), profile("truck")],
    )
    .expect_err("duplicate profile id must fail");

    std::assert_matches!(
        error,
        CoreError::DuplicateVehicleProfileId { profile_id } if profile_id == "truck"
    );
}

#[test]
fn empty_registry_and_handle_public_traits_match_contract() {
    fn assert_handle_traits<T: Clone + Copy + Debug + Eq + Hash>() {}

    assert_handle_traits::<VehicleProfileHandle>();
    let registry = VehicleProfileRegistry::empty();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

fn participant_classes() -> (ParticipantClassRegistry, ParticipantClassHandle) {
    let classes = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("participant classes must be valid");
    let car = classes.class_handle("car").expect("car class must exist");
    (classes, car)
}

#[test]
fn profile_preserves_participant_class_attribution() {
    let (classes, car) = participant_classes();
    let registry = VehicleProfileRegistry::try_new(&classes, [profile("passenger-car")])
        .expect("valid profile registry");

    let handle = registry
        .profile_handle("passenger-car")
        .expect("profile handle exists");
    assert_eq!(
        registry
            .profile(handle)
            .map(VehicleProfile::participant_class),
        Some(car)
    );
}

#[test]
fn registry_rejects_participant_class_handle_outside_class_registry() {
    // profile helper 归属两类 registry 的 `car`（index 1），超出单类 registry 范围。
    let single_class_registry =
        ParticipantClassRegistry::try_new(vec![ParticipantClass::new("motorVehicle", None)])
            .expect("valid class registry");

    let error = VehicleProfileRegistry::try_new(&single_class_registry, [profile("passenger-car")])
        .expect_err("out-of-range participant class handle must fail");

    std::assert_matches!(
        error,
        CoreError::VehicleProfileParticipantClassOutOfRange {
            profile_id,
            class_index: 1,
            class_count: 1,
        } if profile_id == "passenger-car"
    );
}
