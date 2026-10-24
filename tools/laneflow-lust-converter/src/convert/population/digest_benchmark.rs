//! #1029 非生产实验：对比持久工作负载派生的算法成本，不提供可替换算法的生产入口。

use std::{collections::HashMap, hint::black_box, path::Path, time::Instant};

use super::*;
use crate::{
    source::{LUST_COMMIT, PINNED_SOURCE_FILES, read_verified, recheck_source_revision},
    sumo::parse_due_routes_xml_filtered,
};

const DEPART_PREFIX: &[u8] = b"LF-REAL-LUST-DEMAND-v1\0";
const SAMPLES: usize = 11;

struct Case<'a> {
    name: &'static str,
    run: Box<dyn FnMut() + 'a>,
}

impl<'a> Case<'a> {
    fn new(name: &'static str, run: impl FnMut() + 'a) -> Self {
        Self {
            name,
            run: Box::new(run),
        }
    }
}

fn elapsed_ns(run: &mut dyn FnMut(), iterations: u32) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        run();
    }
    start.elapsed().as_secs_f64() * 1_000_000_000.0
}

fn measure(group: &str, mut cases: Vec<Case<'_>>) {
    // 计时在整批操作外；每样本至少约 40 ms，避免给每个短摘要插入计时器。
    let iterations = cases
        .iter_mut()
        .map(|case| {
            let mut iterations = 1;
            while elapsed_ns(case.run.as_mut(), iterations) < 40_000_000.0 && iterations < 256 {
                iterations *= 2;
            }
            iterations
        })
        .collect::<Vec<_>>();
    let mut times = vec![Vec::new(); cases.len()];
    // 每个样本轮转起点并交替方向，降低温度、调频与后台负载的顺序偏差。
    for sample in 0..SAMPLES {
        for offset in 0..cases.len() {
            let index = if sample % 2 == 0 {
                (sample + offset) % cases.len()
            } else {
                (sample + cases.len() - offset) % cases.len()
            };
            let case = &mut cases[index];
            let ns =
                elapsed_ns(case.run.as_mut(), iterations[index]) / f64::from(iterations[index]);
            times[index].push(ns);
            println!(
                "sample,{group},{},{sample},{},{ns:.3}",
                case.name, iterations[index]
            );
        }
    }
    for (case, mut values) in cases.iter().zip(times) {
        values.sort_by(f64::total_cmp);
        println!(
            "summary,{group},{},min_ns={:.3},median_ns={:.3},max_ns={:.3}",
            case.name,
            values[0],
            values[SAMPLES / 2],
            values[SAMPLES - 1]
        );
    }
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn blake(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn sha_depart(id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DEPART_PREFIX);
    hasher.update(id.as_bytes());
    hasher.finalize().into()
}

fn blake_depart(id: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DEPART_PREFIX);
    hasher.update(id.as_bytes());
    *hasher.finalize().as_bytes()
}

fn position_numerator(digest: [u8; 32]) -> u64 {
    u64::from_be_bytes(digest[..8].try_into().expect("eight bytes")) >> 11
}

fn position_fraction(digest: [u8; 32]) -> f64 {
    position_numerator(digest) as f64 / ((1_u64 << 53) as f64)
}

// 与 select_population 相同的过滤、截断、校验及物化步骤；只有排序键是实验变量。
// 真实数据启动时逐字段断言 SHA 与生产结果相等，预计算键也必须得到相同记录。
// 不执行路线展开或静态编译，不能把此调用耗时当作 converter 端到端耗时。
fn select_with_key(
    vehicles: &[DueVehicle],
    key: impl Fn(&DueVehicle) -> [u8; 32],
) -> Result<Vec<PopulationRecord>> {
    let known: HashSet<&str> = KNOWN_SOURCE_VTYPE_IDS.iter().copied().collect();
    let mut candidates = Vec::new();
    for vehicle in vehicles {
        if !is_population_candidate(&vehicle.type_id, &vehicle.depart) {
            continue;
        }
        candidates.push(vehicle);
    }
    if candidates.len() != POPULATION_CANDIDATE_COUNT {
        return Err(Error::SumoModel(
            "实验候选数量不符合固定来源合同".to_owned(),
        ));
    }
    candidates.sort_by_cached_key(|vehicle| (key(vehicle), vehicle.id.as_bytes()));
    let selected_len = candidates.len().min(POPULATION_SELECTED_COUNT);
    let mut records = Vec::with_capacity(selected_len);
    let mut seen_ids = HashSet::new();
    for (rank, vehicle) in candidates.into_iter().take(selected_len).enumerate() {
        if !seen_ids.insert(vehicle.id.as_str()) {
            return Err(Error::SumoModel("入选车辆 ID 重复".to_owned()));
        }
        if vehicle.id.is_empty() {
            return Err(Error::SumoModel("入选车辆 ID 为空".to_owned()));
        }
        if !known.contains(vehicle.type_id.as_str()) {
            return Err(Error::SumoModel("入选车辆类型未知".to_owned()));
        }
        if vehicle.depart_pos.as_deref() != Some("random") {
            return Err(Error::SumoModel("入选车辆 departPos 不符合合同".to_owned()));
        }
        if vehicle.road_edge_ids.is_empty() {
            return Err(Error::SumoModel("入选车辆缺少路线".to_owned()));
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

fn print_semantic_changes(sha_records: &[PopulationRecord], blake_records: &[PopulationRecord]) {
    let sha_ranks = sha_records
        .iter()
        .map(|record| (record.vehicle_id.as_str(), record.population_rank))
        .collect::<HashMap<_, _>>();
    let mut common = 0;
    let mut same_rank = 0;
    for record in blake_records {
        if let Some(rank) = sha_ranks.get(record.vehicle_id.as_str()) {
            common += 1;
            same_rank += usize::from(*rank == record.population_rank);
        }
    }
    let changed_q = sha_records
        .iter()
        .filter(|record| {
            position_numerator(sha_depart(&record.vehicle_id))
                != position_numerator(blake_depart(&record.vehicle_id))
        })
        .count();
    let mean_abs_q_delta = sha_records
        .iter()
        .map(|record| {
            (position_fraction(sha_depart(&record.vehicle_id))
                - position_fraction(blake_depart(&record.vehicle_id)))
            .abs()
        })
        .sum::<f64>()
        / sha_records.len() as f64;
    println!(
        "semantic,common={common},removed={},added={},common_rank_changed={},same_rank={same_rank},q_changed={changed_q},mean_abs_q_delta={mean_abs_q_delta:.9}",
        sha_records.len() - common,
        blake_records.len() - common,
        common - same_rank,
    );
    for (name, records) in [("sha", sha_records), ("blake", blake_records)] {
        let edges = records
            .iter()
            .map(|record| record.road_edge_ids.len())
            .sum::<usize>();
        println!(
            "materialization,{name},records={},edge_strings={edges}",
            records.len()
        );
    }
}

#[test]
#[ignore = "#1029 证据测量：需要固定 LUST_SOURCE_DIR、release 构建与独立进程运行"]
fn profile_lust_digest_roles() {
    if cfg!(debug_assertions) {
        panic!("release build required");
    }
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let verified = crate::verify_source(Path::new(&source_dir)).expect("verified source");
    println!(
        "environment,source_commit={LUST_COMMIT},arch={}",
        std::env::consts::ARCH
    );
    #[cfg(target_arch = "x86_64")]
    println!(
        "features,sha={},avx2={},sse41={}",
        std::is_x86_feature_detected!("sha"),
        std::is_x86_feature_detected!("avx2"),
        std::is_x86_feature_detected!("sse4.1"),
    );
    let mut vehicles = Vec::new();
    for (ordinal, pinned) in PINNED_SOURCE_FILES
        .iter()
        .filter(|file| file.relative_path.contains("DUERoutes/"))
        .enumerate()
    {
        let xml = read_verified(&verified, pinned.relative_path).expect("verified DUE bytes");
        let parsed = parse_due_routes_xml_filtered(
            &xml,
            u8::try_from(ordinal).expect("file ordinal"),
            is_population_candidate,
        )
        .expect("DUE parse");
        println!(
            "input,{},bytes={},sha256={}",
            pinned.relative_path, pinned.bytes, pinned.sha256_hex
        );
        vehicles.extend(parsed);
    }
    recheck_source_revision(verified.source_dir()).expect("source revision after reads");
    let candidates = vehicles
        .iter()
        .filter(|vehicle| is_population_candidate(&vehicle.type_id, &vehicle.depart))
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), POPULATION_CANDIDATE_COUNT);
    println!(
        "population,total={},candidates={},id_bytes={},min_id_bytes={},max_id_bytes={}",
        vehicles.len(),
        candidates.len(),
        candidates
            .iter()
            .map(|vehicle| vehicle.id.len())
            .sum::<usize>(),
        candidates
            .iter()
            .map(|vehicle| vehicle.id.len())
            .min()
            .unwrap(),
        candidates
            .iter()
            .map(|vehicle| vehicle.id.len())
            .max()
            .unwrap(),
    );
    let production = select_population(&vehicles, true).expect("production selection");
    let reference =
        select_with_key(&vehicles, |vehicle| sha(vehicle.id.as_bytes())).expect("SHA reference");
    assert_eq!(
        production, reference,
        "SHA reference must match all production fields"
    );
    let blake_records = select_with_key(&vehicles, |vehicle| blake(vehicle.id.as_bytes()))
        .expect("BLAKE selection");
    print_semantic_changes(&production, &blake_records);

    let mut precomputed = [Vec::new(), Vec::new(), Vec::new()];
    for vehicle in &vehicles {
        let file = usize::from(vehicle.source_file_ordinal);
        assert_eq!(
            precomputed[file].len() as u64,
            vehicle.source_vehicle_ordinal
        );
        precomputed[file].push(
            if is_population_candidate(&vehicle.type_id, &vehicle.depart) {
                sha(vehicle.id.as_bytes())
            } else {
                [0; 32]
            },
        );
    }
    let cached = |vehicle: &DueVehicle| {
        precomputed[usize::from(vehicle.source_file_ordinal)]
            [vehicle.source_vehicle_ordinal as usize]
    };
    assert_eq!(
        production,
        select_with_key(&vehicles, cached).expect("precomputed reference")
    );
    println!("sample_header,group,case,sample,iterations,ns_per_call");
    measure(
        "candidate_hash",
        vec![
            Case::new("sha", || {
                for vehicle in &candidates {
                    black_box(sha(black_box(vehicle.id.as_bytes())));
                }
            }),
            Case::new("blake", || {
                for vehicle in &candidates {
                    black_box(blake(black_box(vehicle.id.as_bytes())));
                }
            }),
        ],
    );
    measure(
        "select_and_drop",
        vec![
            Case::new("production", || {
                black_box(select_population(black_box(&vehicles), true).unwrap());
            }),
            Case::new("sha", || {
                black_box(
                    select_with_key(black_box(&vehicles), |vehicle| sha(vehicle.id.as_bytes()))
                        .unwrap(),
                );
            }),
            Case::new("precomputed_sha", || {
                black_box(select_with_key(black_box(&vehicles), cached).unwrap());
            }),
            Case::new("blake", || {
                black_box(
                    select_with_key(black_box(&vehicles), |vehicle| blake(vehicle.id.as_bytes()))
                        .unwrap(),
                );
            }),
            // 固定为 SHA 人口，额外算出并消费候选摘要；两组具有相同排序与物化负载。
            Case::new("fixed_population_sha", || {
                black_box(
                    select_with_key(black_box(&vehicles), |vehicle| {
                        black_box(sha(vehicle.id.as_bytes()));
                        cached(vehicle)
                    })
                    .unwrap(),
                );
            }),
            Case::new("fixed_population_blake", || {
                black_box(
                    select_with_key(black_box(&vehicles), |vehicle| {
                        black_box(blake(vehicle.id.as_bytes()));
                        cached(vehicle)
                    })
                    .unwrap(),
                );
            }),
        ],
    );
    measure(
        "depart_fraction",
        vec![
            Case::new("sha", || {
                for record in &production {
                    black_box(position_fraction(sha_depart(black_box(&record.vehicle_id))));
                }
            }),
            Case::new("blake", || {
                for record in &production {
                    black_box(position_fraction(blake_depart(black_box(
                        &record.vehicle_id,
                    ))));
                }
            }),
        ],
    );
}
