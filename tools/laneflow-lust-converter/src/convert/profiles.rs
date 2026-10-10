//! Convert SUMO passenger vTypes into Traffic vehicleProfiles (§3.4).

use crate::{
    Error, Result,
    output::model::VehicleProfile,
    sumo::{SUMO_ID_PREFIX, vtype::SumoVType},
};

/// Exact passenger vType IDs accepted by LuST v1 static conversion.
pub const LUST_PASSENGER_VTYPE_IDS: &[&str] = &[
    "passenger1",
    "passenger2a",
    "passenger2b",
    "passenger3",
    "passenger4",
    "passenger5",
];

const EMERGENCY_DECELERATION: f64 = 8.0;
const TIME_HEADWAY: f64 = 1.0;

/// Select only passenger vTypes from a parsed vtypes file (drops bus for health-aware callers).
///
/// The LuST source file contains a bus type for health anchors; static profile conversion
/// consumes only passengers via [`convert_vehicle_profiles`] after filtering.
pub fn select_passenger_vtypes(vtypes: &[SumoVType]) -> Result<Vec<SumoVType>> {
    let mut passengers = Vec::new();
    for vtype in vtypes {
        match vtype.v_class.as_str() {
            "passenger" => passengers.push(vtype.clone()),
            "bus" => {}
            other => {
                return Err(Error::SumoModel(format!(
                    "unknown vType class {other:?} for id {:?}",
                    vtype.id
                )));
            }
        }
    }
    Ok(passengers)
}

/// Normalize passenger vTypes into LaneFlow vehicle profiles.
///
/// Bus and unknown classes are source-selection errors. Exactly the six pinned
/// passenger IDs must be present.
pub(crate) fn convert_vehicle_profiles(vtypes: &[SumoVType]) -> Result<Vec<VehicleProfile>> {
    let mut profiles = Vec::with_capacity(LUST_PASSENGER_VTYPE_IDS.len());
    for vtype in vtypes {
        match vtype.v_class.as_str() {
            "passenger" => {
                if !LUST_PASSENGER_VTYPE_IDS.contains(&vtype.id.as_str()) {
                    return Err(Error::SumoModel(format!(
                        "unknown passenger vType id {:?} (not in pinned LuST set)",
                        vtype.id
                    )));
                }
                profiles.push(VehicleProfile {
                    id: format!("{SUMO_ID_PREFIX}{}", vtype.id),
                    length: vtype.length.to_f64()?,
                    model: "iidm",
                    desired_speed: vtype.max_speed.to_f64()?,
                    min_gap: vtype.min_gap.to_f64()?,
                    time_headway: TIME_HEADWAY,
                    max_acceleration: vtype.accel.to_f64()?,
                    comfortable_deceleration: vtype.decel.to_f64()?,
                    emergency_deceleration: EMERGENCY_DECELERATION,
                });
            }
            "bus" => {
                return Err(Error::SumoModel(format!(
                    "bus vType {:?} is a source selection error for LuST v1 static profiles",
                    vtype.id
                )));
            }
            other => {
                return Err(Error::SumoModel(format!(
                    "unknown vType class {other:?} for id {:?}",
                    vtype.id
                )));
            }
        }
    }

    profiles.sort_by(|left, right| left.id.cmp(&right.id));
    let expected = LUST_PASSENGER_VTYPE_IDS
        .iter()
        .map(|id| format!("{SUMO_ID_PREFIX}{id}"))
        .collect::<Vec<_>>();
    let actual = profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(Error::SumoModel(format!(
            "passenger profile set mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sumo::parse_vtypes_xml;

    #[test]
    fn fixture_profiles_skip_bus_and_emit_constants() {
        let xml = include_str!("../../tests/fixtures/minimal/vtypes.add.xml");
        let vtypes = parse_vtypes_xml(xml).expect("parse");
        let passengers = select_passenger_vtypes(&vtypes).expect("select");
        let profiles = convert_vehicle_profiles(&passengers).expect("convert");
        assert_eq!(profiles.len(), 6);
        assert_eq!(profiles[0].id, "sumo:passenger1");
        assert_eq!(profiles[0].emergency_deceleration, 8.0);
        assert_eq!(profiles[0].time_headway, 1.0);
        assert_eq!(profiles[0].desired_speed, 70.0);
    }

    #[test]
    fn bus_in_selected_set_fails() {
        let xml = include_str!("../../tests/fixtures/minimal/vtypes.add.xml");
        let vtypes = parse_vtypes_xml(xml).expect("parse");
        let error = convert_vehicle_profiles(&vtypes).expect_err("bus");
        assert!(error.to_string().contains("bus"));
    }
}
