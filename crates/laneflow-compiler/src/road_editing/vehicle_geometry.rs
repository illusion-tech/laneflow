use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_static_contract::RoadVehicleGeometry;

use crate::{RigidTwoAxleGeometryInput, RoadEditingInputViolation, RoadVehicleConfiguration};

const FIELD_NAMES: [&str; 10] = [
    "vehicleProfile.geometry.wheelbaseM",
    "vehicleProfile.geometry.trackWidthM",
    "vehicleProfile.geometry.referenceHeightM",
    "vehicleProfile.geometry.rearOverhangM",
    "vehicleProfile.geometry.widthM",
    "vehicleProfile.geometry.bottomHeightM",
    "vehicleProfile.geometry.topHeightM",
    "vehicleProfile.geometry.maxSteeringAngleRad",
    "vehicleProfile.geometry.maxSteeringRateRadS",
    "vehicleProfile.geometry.configuration",
];

pub(super) fn field_name(index: Option<u16>) -> &'static str {
    index.map_or("vehicleProfile.geometry", |i| FIELD_NAMES[usize::from(i)])
}

pub(super) fn violation(error: crate::VehicleGeometryInputError) -> RoadEditingInputViolation {
    match error.violation {
        crate::ScalarViolation::UnsupportedVehicleConfiguration => {
            RoadEditingInputViolation::UnsupportedVehicleConfiguration {
                model: error.value as u8,
            }
        }
        crate::ScalarViolation::NotFinite => RoadEditingInputViolation::NonFinite {
            value_bits: error.value.to_bits(),
        },
        crate::ScalarViolation::NotLessThan {
            inclusive_minimum_bits: 0,
        } => RoadEditingInputViolation::LessThanZero {
            value_bits: error.value.to_bits(),
        },
        _ => RoadEditingInputViolation::InvalidCombination,
    }
}

pub(super) fn admit(
    geometry: Option<wire::RoadVehicleGeometry<'_>>,
    length_mm: u32,
) -> Result<RoadVehicleGeometry, (Option<u16>, RoadEditingInputViolation)> {
    let missing = RoadEditingInputViolation::MissingVehicleGeometryField;
    let geometry = geometry.ok_or((None, missing))?;
    let model = geometry.configuration().ok_or((Some(9), missing))?;
    let Some(RoadVehicleConfiguration::RigidTwoAxle) = RoadVehicleConfiguration::from_code(model)
    else {
        return Err((
            Some(9),
            RoadEditingInputViolation::UnsupportedVehicleConfiguration { model },
        ));
    };
    let input = RigidTwoAxleGeometryInput {
        wheelbase_m: geometry.wheelbase_m().ok_or((Some(0), missing))?,
        track_width_m: geometry.track_width_m().ok_or((Some(1), missing))?,
        reference_height_m: geometry.reference_height_m().ok_or((Some(2), missing))?,
        rear_overhang_m: geometry.rear_overhang_m().ok_or((Some(3), missing))?,
        width_m: geometry.width_m().ok_or((Some(4), missing))?,
        bottom_height_m: geometry.bottom_height_m().ok_or((Some(5), missing))?,
        top_height_m: geometry.top_height_m().ok_or((Some(6), missing))?,
        max_steering_angle_rad: geometry
            .max_steering_angle_rad()
            .ok_or((Some(7), missing))?,
        max_steering_rate_rad_s: geometry
            .max_steering_rate_rad_s()
            .ok_or((Some(8), missing))?,
    };
    input
        .canonical(length_mm)
        .map_err(|error| (Some(error.field_index()), violation(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{
        IidmVehicleProfileInput, ParticipantClassReference, VehicleProfileInput,
    };

    #[test]
    fn authoring_builder_names_the_unsupported_vehicle_and_configuration() {
        let geometry = crate::RoadVehicleGeometryInput::SemiTrailer;
        let error = VehicleProfileInput::try_new(
            "freight",
            ParticipantClassReference::local("motor").unwrap(),
            IidmVehicleProfileInput::try_new(12., 14., 2., 1.4, 2., 3., 6.).unwrap(),
            geometry,
        )
        .unwrap_err();
        let diagnostic = &error.diagnostics()[0];
        assert_eq!(diagnostic.stable_key(), Some("freight"));
        assert!(
            matches!(diagnostic.payload(), crate::DiagnosticPayload::InvalidRoadEditingInput {
            field, violation: RoadEditingInputViolation::UnsupportedVehicleConfiguration { model: 2 }
        } if field.as_ref() == "vehicleProfile.geometry.configuration")
        );
    }
}
