use std::error::Error;

use laneflow_core::CoreWorld;
use laneflow_spatial::{
    CANONICAL_FRAME_ID_PATTERN, CANONICAL_POINT_COMPONENT_MAX_METERS,
    CANONICAL_POINT_COMPONENT_MIN_METERS, CanonicalFrameId, CanonicalPoint3F32,
    CanonicalUnitVector3F32, CanonicalVector3F32, SpatialAxis, SpatialError, SpatialRegistry,
};

#[test]
fn frame_id_accepts_frozen_ascii_token_boundaries() {
    for value in ["A", "frame_1", "campus/main:road-2.3"] {
        let frame_id = CanonicalFrameId::try_new(value).expect("valid frame ID");
        assert_eq!(frame_id.as_str(), value);
        assert_eq!(frame_id.to_string(), value);
    }

    let max_length = "a".repeat(128);
    assert_eq!(
        CanonicalFrameId::try_new(max_length.clone())
            .expect("128-byte frame ID is valid")
            .as_str(),
        max_length
    );
}

#[test]
fn frame_id_rejects_invalid_tokens_with_frozen_pattern() {
    let too_long = "a".repeat(129);
    for value in ["", "_frame", "-frame", "frame 1", "坐标框架", &too_long] {
        assert_eq!(
            CanonicalFrameId::try_new(value).expect_err("invalid frame ID"),
            SpatialError::InvalidFrameId {
                value: value.to_owned(),
                pattern: CANONICAL_FRAME_ID_PATTERN,
            }
        );
    }
}

#[test]
fn point_and_vector_reject_every_non_finite_axis() {
    for (axis, coordinates) in [
        (SpatialAxis::X, [f32::NAN, 0.0, 0.0]),
        (SpatialAxis::Y, [0.0, f32::INFINITY, 0.0]),
        (SpatialAxis::Z, [0.0, 0.0, f32::NEG_INFINITY]),
    ] {
        std::assert_matches!(
            CanonicalPoint3F32::try_new(coordinates[0], coordinates[1], coordinates[2])
                .expect_err("non-finite point must fail"),
            SpatialError::NonFiniteComponent {
                value_kind: "CanonicalPoint3F32",
                axis: actual_axis,
                ..
            } if actual_axis == axis
        );
        std::assert_matches!(
            CanonicalVector3F32::try_new(coordinates[0], coordinates[1], coordinates[2])
                .expect_err("non-finite vector must fail"),
            SpatialError::NonFiniteComponent {
                value_kind: "CanonicalVector3F32",
                axis: actual_axis,
                ..
            } if actual_axis == axis
        );
    }
}

#[test]
fn point_accepts_closed_range_and_rejects_each_out_of_range_axis() {
    let boundary = CanonicalPoint3F32::try_new(
        CANONICAL_POINT_COMPONENT_MIN_METERS,
        0.0,
        CANONICAL_POINT_COMPONENT_MAX_METERS,
    )
    .expect("closed range boundaries are valid");
    assert_eq!(boundary.x(), -16_384.0);
    assert_eq!(boundary.z(), 16_384.0);

    let below = f32::from_bits(CANONICAL_POINT_COMPONENT_MIN_METERS.to_bits() + 1);
    let above = f32::from_bits(CANONICAL_POINT_COMPONENT_MAX_METERS.to_bits() + 1);
    for (axis, coordinates, value) in [
        (SpatialAxis::X, [below, 0.0, 0.0], below),
        (SpatialAxis::Y, [0.0, above, 0.0], above),
        (SpatialAxis::Z, [0.0, 0.0, below], below),
    ] {
        assert_eq!(
            CanonicalPoint3F32::try_new(coordinates[0], coordinates[1], coordinates[2])
                .expect_err("out-of-range point must fail"),
            SpatialError::PointComponentOutOfRange {
                axis,
                value,
                min: CANONICAL_POINT_COMPONENT_MIN_METERS,
                max: CANONICAL_POINT_COMPONENT_MAX_METERS,
            }
        );
    }

    assert!(CanonicalVector3F32::try_new(32_768.0, -32_768.0, 0.0).is_ok());
}

#[test]
fn valid_primitives_canonicalize_signed_zero() {
    let point = CanonicalPoint3F32::try_new(-0.0, 0.0, -0.0).expect("valid point");
    let vector = CanonicalVector3F32::try_new(-0.0, 0.0, -0.0).expect("valid vector");

    for value in [
        point.x(),
        point.y(),
        point.z(),
        vector.x(),
        vector.y(),
        vector.z(),
    ] {
        assert_eq!(value.to_bits(), 0.0_f32.to_bits());
    }
}

#[test]
fn checked_named_arithmetic_rejects_out_of_range_and_non_finite_results() {
    let point = CanonicalPoint3F32::try_new(16_384.0, 0.0, 0.0).expect("valid point");
    let displacement = CanonicalVector3F32::try_new(1.0, 0.0, 0.0).expect("finite vector");
    let vector = CanonicalVector3F32::try_new(f32::MAX, 0.0, 0.0).expect("finite vector");

    assert_eq!(
        point
            .checked_add_vector(displacement)
            .expect_err("point range must be revalidated"),
        SpatialError::PointComponentOutOfRange {
            axis: SpatialAxis::X,
            value: 16_385.0,
            min: -16_384.0,
            max: 16_384.0,
        }
    );
    std::assert_matches!(
        vector.checked_scale(2.0).expect_err("overflow must fail"),
        SpatialError::NonFiniteComponent {
            value_kind: "CanonicalVector3F32",
            axis: SpatialAxis::X,
            ..
        }
    );
}

#[test]
fn checked_point_and_vector_operations_preserve_type_boundaries() {
    let start = CanonicalPoint3F32::try_new(1.0, 2.0, 3.0).expect("valid point");
    let displacement = CanonicalVector3F32::try_new(4.0, 5.0, 6.0).expect("valid vector");
    let end = start
        .checked_add_vector(displacement)
        .expect("finite point result");

    assert_eq!([end.x(), end.y(), end.z()], [5.0, 7.0, 9.0]);
    assert_eq!(
        start.checked_vector_to(end).expect("finite vector result"),
        displacement
    );
    assert_eq!(
        end.checked_sub_vector(displacement)
            .expect("finite point result"),
        start
    );
    assert_eq!(
        displacement
            .checked_add(CanonicalVector3F32::try_new(1.0, 1.0, 1.0).expect("valid vector"))
            .expect("finite vector sum"),
        CanonicalVector3F32::try_new(5.0, 6.0, 7.0).expect("valid vector")
    );
}

#[test]
fn unit_vector_rejects_zero_and_normalizes_extreme_finite_inputs() {
    let zero = CanonicalVector3F32::try_new(0.0, 0.0, 0.0).expect("valid zero vector");
    assert_eq!(
        CanonicalUnitVector3F32::try_from_vector(zero).expect_err("zero direction must fail"),
        SpatialError::ZeroLengthDirection
    );

    for vector in [
        CanonicalVector3F32::try_new(f32::MAX, f32::MAX, 0.0).expect("large finite vector"),
        CanonicalVector3F32::try_new(f32::from_bits(1), 0.0, 0.0).expect("small finite vector"),
    ] {
        let unit = vector
            .try_normalize()
            .expect("scaled normalization succeeds");
        let length = unit.x().hypot(unit.y()).hypot(unit.z());

        assert!(unit.x().is_finite());
        assert!(unit.y().is_finite());
        assert!(unit.z().is_finite());
        assert!((length - 1.0).abs() <= 2.0 * f32::EPSILON);
        assert_eq!(unit.as_vector().x(), unit.x());
    }
}

#[test]
fn public_types_are_laneflow_owned_and_errors_implement_std_error() {
    fn assert_copy<T: Copy>() {}
    fn assert_error<T: Error>() {}
    fn assert_public_type<T>() {}

    assert_copy::<CanonicalPoint3F32>();
    assert_copy::<CanonicalVector3F32>();
    assert_copy::<CanonicalUnitVector3F32>();
    assert_error::<SpatialError>();
    assert_public_type::<SpatialRegistry>();
}

#[test]
fn core_world_remains_usable_without_spatial_registry() {
    assert!(CoreWorld::new(1_000).is_ok());
}
