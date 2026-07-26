//! Shared exact 10k source population selection (§4).

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    convert::profiles::LUST_PASSENGER_VTYPE_IDS,
    sumo::{decimal::ExactDecimal, due::DueVehicle},
};

/// Inclusive depart window start (seconds).
pub const POPULATION_DEPART_START_SECONDS: &str = "28800";
/// Exclusive depart window end (seconds).
pub const POPULATION_DEPART_END_SECONDS: &str = "30600";
/// Exact candidate count after filtering for full LuST.
pub const POPULATION_CANDIDATE_COUNT: usize = 10_592;
/// Selected population size.
pub const POPULATION_SELECTED_COUNT: usize = 10_000;

const KNOWN_SOURCE_VTYPE_IDS: &[&str] = &[
    "passenger1",
    "passenger2a",
    "passenger2b",
    "passenger3",
    "passenger4",
    "passenger5",
    "bus",
];

/// One selected population record (shared TOPO/DEMAND input; not Traffic schema).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopulationRecord {
    pub population_rank: u32,
    pub vehicle_id: String,
    pub type_id: String,
    pub depart: ExactDecimal,
    pub road_edge_ids: Vec<String>,
    pub source_file_ordinal: u8,
    pub source_vehicle_ordinal: u64,
}

/// Filter / sort / rank DUE vehicles into the shared population table.
pub fn select_population(
    vehicles: &[DueVehicle],
    require_lust_candidate_count: bool,
) -> Result<Vec<PopulationRecord>> {
    let start: ExactDecimal = POPULATION_DEPART_START_SECONDS
        .parse()
        .expect("literal decimal");
    let end: ExactDecimal = POPULATION_DEPART_END_SECONDS
        .parse()
        .expect("literal decimal");
    let passenger: HashSet<&str> = LUST_PASSENGER_VTYPE_IDS.iter().copied().collect();
    let known: HashSet<&str> = KNOWN_SOURCE_VTYPE_IDS.iter().copied().collect();

    let mut seen_ids = HashSet::new();
    let mut candidates = Vec::new();
    for vehicle in vehicles {
        if vehicle.id.is_empty() {
            return Err(Error::SumoModel(
                "DUE vehicle id must not be empty".to_owned(),
            ));
        }
        if !seen_ids.insert(vehicle.id.as_str()) {
            return Err(Error::SumoModel(format!(
                "duplicate DUE vehicle id {:?}",
                vehicle.id
            )));
        }
        if !known.contains(vehicle.type_id.as_str()) {
            return Err(Error::SumoModel(format!(
                "unknown DUE vtype {:?} for vehicle {:?}",
                vehicle.type_id, vehicle.id
            )));
        }
        if !vehicle.depart.is_greater_or_equal(start) || !vehicle.depart.is_less_than(end) {
            continue;
        }
        if !passenger.contains(vehicle.type_id.as_str()) {
            continue;
        }
        candidates.push(vehicle);
    }

    if require_lust_candidate_count && candidates.len() != POPULATION_CANDIDATE_COUNT {
        return Err(Error::SumoModel(format!(
            "LuST population candidate count mismatch: expected {POPULATION_CANDIDATE_COUNT}, got {}",
            candidates.len()
        )));
    }

    candidates.sort_by(|left, right| {
        let left_digest = Sha256::digest(left.id.as_bytes());
        let right_digest = Sha256::digest(right.id.as_bytes());
        left_digest
            .as_slice()
            .cmp(right_digest.as_slice())
            .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
    });

    if require_lust_candidate_count && candidates.len() < POPULATION_SELECTED_COUNT {
        return Err(Error::SumoModel(format!(
            "not enough population candidates to select {POPULATION_SELECTED_COUNT}"
        )));
    }

    let selected_len = candidates.len().min(POPULATION_SELECTED_COUNT);
    let mut records = Vec::with_capacity(selected_len);
    for (rank, vehicle) in candidates.into_iter().take(selected_len).enumerate() {
        records.push(PopulationRecord {
            population_rank: u32::try_from(rank).expect("rank fits u32"),
            vehicle_id: vehicle.id.clone(),
            type_id: vehicle.type_id.clone(),
            depart: vehicle.depart,
            road_edge_ids: vehicle.road_edge_ids.clone(),
            source_file_ordinal: vehicle.source_file_ordinal,
            source_vehicle_ordinal: vehicle.source_vehicle_ordinal,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sumo::due::DueVehicle;

    fn vehicle(id: &str, type_id: &str, depart: &str, edges: &[&str]) -> DueVehicle {
        DueVehicle {
            id: id.to_owned(),
            type_id: type_id.to_owned(),
            depart: depart.parse().unwrap(),
            road_edge_ids: edges.iter().map(|edge| (*edge).to_owned()).collect(),
            source_file_ordinal: 0,
            source_vehicle_ordinal: 0,
        }
    }

    #[test]
    fn filters_depart_window_and_passenger_types() {
        let vehicles = vec![
            vehicle("early", "passenger1", "28799.99", &["west", "east"]),
            vehicle("in-window", "passenger1", "28800", &["west", "east"]),
            vehicle("bus", "bus", "28800", &["west", "east"]),
            vehicle("late", "passenger1", "30600", &["west", "east"]),
            vehicle("also", "passenger2a", "30599.99", &["west", "east"]),
        ];
        let records = select_population(&vehicles, false).expect("select");
        let ids: HashSet<_> = records.iter().map(|r| r.vehicle_id.as_str()).collect();
        assert_eq!(ids, HashSet::from(["in-window", "also"]));
    }

    #[test]
    fn unknown_vtype_fails_closed() {
        let vehicles = vec![vehicle("x", "truck", "28800", &["west"])];
        let error = select_population(&vehicles, false).expect_err("unknown");
        assert!(error.to_string().contains("unknown DUE vtype"));
    }

    #[test]
    fn ranks_by_sha256_then_id_bytes() {
        let vehicles = vec![
            vehicle("b", "passenger1", "28800", &["west"]),
            vehicle("a", "passenger1", "28800", &["west"]),
            vehicle("c", "passenger1", "28800", &["west"]),
        ];
        let records = select_population(&vehicles, false).expect("select");
        assert_eq!(records.len(), 3);
        let mut expected = vehicles.clone();
        expected.sort_by(|left, right| {
            let left_digest = Sha256::digest(left.id.as_bytes());
            let right_digest = Sha256::digest(right.id.as_bytes());
            left_digest
                .as_slice()
                .cmp(right_digest.as_slice())
                .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
        });
        for (rank, (record, vehicle)) in records.iter().zip(expected.iter()).enumerate() {
            assert_eq!(record.population_rank, rank as u32);
            assert_eq!(record.vehicle_id, vehicle.id);
        }
    }
}
