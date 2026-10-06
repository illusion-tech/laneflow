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

/// 选中记录的源序数三元组（§4：source-file ordinal 与 XML vehicle ordinal
/// 进转换报告——诊断模式不产 routes.toml，报告须自承载追溯链；序数不参与
/// 排序，仅供审计把选中 rank 追溯回源记录）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PopulationOrdinal {
    pub population_rank: u32,
    pub source_file_ordinal: u8,
    pub source_vehicle_ordinal: u64,
}

/// 候选谓词的单一事实源：depart ∈ [28800, 30600) 且为 passenger vtype。
/// `select_population` 的候选过滤与 DUE 解析期的 edges 惰性物化
/// （`parse_due_routes_xml_filtered`）共用此判定，两侧不得各自实现。
pub(crate) fn is_population_candidate(type_id: &str, depart: &ExactDecimal) -> bool {
    let start: ExactDecimal = POPULATION_DEPART_START_SECONDS
        .parse()
        .expect("literal decimal");
    let end: ExactDecimal = POPULATION_DEPART_END_SECONDS
        .parse()
        .expect("literal decimal");
    depart.is_greater_or_equal(start)
        && depart.is_less_than(end)
        && LUST_PASSENGER_VTYPE_IDS.contains(&type_id)
}

/// Filter / sort / rank DUE vehicles into the shared population table.
pub fn select_population(
    vehicles: &[DueVehicle],
    require_lust_candidate_count: bool,
) -> Result<Vec<PopulationRecord>> {
    let known: HashSet<&str> = KNOWN_SOURCE_VTYPE_IDS.iter().copied().collect();

    let mut candidates = Vec::new();
    for vehicle in vehicles {
        if !is_population_candidate(&vehicle.type_id, &vehicle.depart) {
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

    // #253 K3(a) 契约澄清：失败契约（unknown route / dangling edge）针对
    // **入选**的 10,000 候选——截断先行，route/dangling-edge 验证在选取
    // 集合上进行（build_routes_and_bind_population）。截断尾部不参与验证：
    // pinned 实测尾部 592 个候选中 558 个无完整车道级路径，截断实质承担
    // 过滤职能，非质量判定（§4 注记）。
    let selected_len = candidates.len().min(POPULATION_SELECTED_COUNT);
    let mut records = Vec::with_capacity(selected_len);
    // #253 Q2：重复 id 检查在选取集合上进行（K3(a) 契约口径——失败域限于
    // 入选 10,000；截断尾部的重复不参与判定）。
    let mut seen_ids = HashSet::new();
    for (rank, vehicle) in candidates.into_iter().take(selected_len).enumerate() {
        if !seen_ids.insert(vehicle.id.as_str()) {
            return Err(Error::SumoModel(format!(
                "duplicate DUE vehicle id {:?} within the selected population",
                vehicle.id
            )));
        }
        // #253 T1/U3：空 id、unknown vtype 与重复 id 同属入选集失败域
        // （K3(a) 契约口径），截断尾部不参与判定。passenger 过滤保证入选
        // 者必在 known 集内，vtype 检查为防御性（两集若未来分叉即在此拦截）。
        if vehicle.id.is_empty() {
            return Err(Error::SumoModel(
                "DUE vehicle id must not be empty within the selected population".to_owned(),
            ));
        }
        if !known.contains(vehicle.type_id.as_str()) {
            return Err(Error::SumoModel(format!(
                "unknown DUE vtype {:?} for selected vehicle {:?}",
                vehicle.type_id, vehicle.id
            )));
        }
        // #253 U5：departPos 必须存在且为 `random`（契约），失败域在入选集。
        if vehicle.depart_pos.as_deref() != Some("random") {
            return Err(Error::SumoModel(format!(
                "selected vehicle {:?} has departPos {:?}, expected \"random\"",
                vehicle.id, vehicle.depart_pos
            )));
        }
        // #253 W1：缺内联 <route> 的失败域限在入选集（K3(a) 契约口径，与空
        // id、departPos 一致）——解析期原样保留（缺失也收），入选者缺失即
        // fail-closed；Q9 的多个 route 仍由解析期拒绝（结构形态问题）。
        if vehicle.road_edge_ids.is_empty() {
            return Err(Error::SumoModel(format!(
                "selected vehicle {:?} missing inline <route>",
                vehicle.id
            )));
        }
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
            depart_pos: Some("random".to_owned()),
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
    fn unknown_vtype_is_filtered_before_selection() {
        // #253 T1：unknown vtype 失败域限于入选集——非 passenger 类型在候选
        // 过滤即被排除，不参与判定（pinned 全局/入选集均 0 实例，行为不变）。
        let vehicles = vec![vehicle("x", "truck", "28800", &["west"])];
        let records = select_population(&vehicles, false).expect("unknown vtype filtered");
        assert!(records.is_empty());
    }

    #[test]
    fn empty_id_and_depart_pos_checked_on_selected_set_only() {
        // #253 U3/U5：空 id 与 departPos（缺失/非 random）的失败域限在入选集
        // ——截断尾部的违规不参与判定，入选集内的违规 fail-closed。
        let selected = vehicle("keep", "passenger1", "28800", &["west", "east"]);
        let mut tail_empty = vehicle("", "passenger1", "28000", &["west", "east"]);
        tail_empty.depart_pos = None;
        let tail_only = vec![selected, tail_empty];
        assert_eq!(
            select_population(&tail_only, false)
                .expect("tail violations ignored")
                .len(),
            1
        );

        let mut bad_id = vehicle("", "passenger1", "28800", &["west", "east"]);
        bad_id.depart_pos = Some("random".to_owned());
        let error = select_population(&[bad_id], false).expect_err("selected empty id fails");
        assert!(error.to_string().contains("must not be empty"), "{error}");

        let mut bad_pos = vehicle("v", "passenger1", "28800", &["west", "east"]);
        bad_pos.depart_pos = Some("free".to_owned());
        let error =
            select_population(&[bad_pos], false).expect_err("selected non-random departPos fails");
        assert!(error.to_string().contains("departPos"), "{error}");

        let mut missing_pos = vehicle("v2", "passenger1", "28800", &["west", "east"]);
        missing_pos.depart_pos = None;
        let error =
            select_population(&[missing_pos], false).expect_err("selected missing departPos fails");
        assert!(error.to_string().contains("departPos"), "{error}");
    }

    #[test]
    fn missing_inline_route_checked_on_selected_set_only() {
        // #253 W1：缺内联 <route> 的失败域限在入选集——截断尾部的缺失不参与
        // 判定；入选集内的缺失 fail-closed。
        let selected = vehicle("keep", "passenger1", "28800", &["west", "east"]);
        let mut tail_missing = vehicle("tail", "passenger1", "28000", &["west", "east"]);
        tail_missing.road_edge_ids = Vec::new();
        let tail_only = vec![selected, tail_missing];
        assert_eq!(
            select_population(&tail_only, false)
                .expect("tail missing route ignored")
                .len(),
            1
        );

        let mut missing = vehicle("v", "passenger1", "28800", &["west"]);
        missing.road_edge_ids = Vec::new();
        let error = select_population(&[missing], false).expect_err("selected missing route fails");
        assert!(
            error.to_string().contains("missing inline <route>"),
            "{error}"
        );
    }

    #[test]
    fn duplicate_id_check_applies_to_selected_set_only() {
        // #253 Q2：重复 id 检查在选取集合上（K3(a) 契约口径）——截断尾部
        // 的重复不参与判定；入选集内的重复仍 fail-closed。
        let mut vehicles = vec![
            vehicle("keep", "passenger1", "28800", &["west", "east"]),
            vehicle("keep", "passenger1", "28000", &["west", "east"]), // 窗外尾部重复
        ];
        let records = select_population(&vehicles, false).expect("tail duplicate ignored");
        assert_eq!(records.len(), 1);
        vehicles.push(vehicle("keep", "passenger1", "28801", &["west", "east"]));
        let error = select_population(&vehicles, false).expect_err("selected duplicate fails");
        assert!(
            error.to_string().contains("duplicate DUE vehicle id"),
            "{error}"
        );
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
