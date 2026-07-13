use std::{fmt::Debug, hash::Hash};

use laneflow_core::{
    CoreError, GEOMETRY_GAP_EPSILON, IidmProfileSpec, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry,
};

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
    VehicleProfile::try_new_iidm(id, canonical_spec()).expect("valid IIDM profile")
}

#[test]
fn valid_iidm_profile_preserves_external_id_and_parameters() {
    let profile = profile("passenger-car");

    assert_eq!(profile.external_id(), "passenger-car");
    assert_eq!(profile.iidm(), canonical_spec());
}

#[test]
fn profile_external_id_uses_shared_ascii_token_rule() {
    let error = VehicleProfile::try_new_iidm("passenger car", canonical_spec())
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

        let error = VehicleProfile::try_new_iidm("profile", spec)
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
fn profile_length_must_exceed_geometry_gap_epsilon() {
    for length in [0.0, GEOMETRY_GAP_EPSILON / 2.0, GEOMETRY_GAP_EPSILON] {
        let spec = IidmProfileSpec {
            length,
            ..canonical_spec()
        };
        let error = VehicleProfile::try_new_iidm("profile", spec)
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
}

#[test]
fn profile_min_gap_allows_zero_but_rejects_negative_or_non_finite() {
    let zero_gap = IidmProfileSpec {
        min_gap: 0.0,
        ..canonical_spec()
    };
    VehicleProfile::try_new_iidm("zero-gap", zero_gap).expect("zero min gap is valid");

    for min_gap in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let spec = IidmProfileSpec {
            min_gap,
            ..canonical_spec()
        };
        let error = VehicleProfile::try_new_iidm("invalid-gap", spec)
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
    let error = VehicleProfile::try_new_iidm("invalid-braking", spec)
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
    let registry = VehicleProfileRegistry::try_new([
        profile("truck"),
        profile("passenger-car"),
        profile("bus"),
    ])
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
    let error = VehicleProfileRegistry::try_new([
        profile("truck"),
        profile("passenger-car"),
        profile("truck"),
    ])
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
