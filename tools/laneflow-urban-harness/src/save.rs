//! 宿主存档与续跑（`docs/design/urban-demand-harness.md` §7）。
//!
//! 存档 = Runtime 快照（`world.lfrs`）+ 宿主耐久状态（`caller.json`）+ 清单
//! （`save.toml`）。宿主状态只用稳定身份与快照局部标识表示，不写进程句柄。
//! 恢复分三步：结构核对（任何下标、长度、引用在使用前核对）、重绑到新句柄、
//! 语义核对（需要恢复后的世界）。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::Path,
    time::Instant,
};

use laneflow_runtime::*;
use laneflow_urban_generator::verified_write;
use serde::{Deserialize, Serialize};

use crate::{
    Artifacts, Harness, IndividualId, ResolvedPlan, Result,
    artifacts::ArtifactFileDigest,
    checked,
    digest::{Domain, FileDigest},
    invalid, observe,
    requests::{RequestLedger, SavedLedger},
    runner::{
        CompletedRecycling, Individual, Request, RetryPolicy, TileEvidence, catalog_bindings,
        observation_pool,
    },
};

pub(crate) const SAVE_VERSION: &str = "urban-save-v3";
pub(crate) const CALLER_VERSION: &str = "urban-save-caller-v1";
pub(crate) const CALLER_DIGEST_VERSION: &str = "urban-caller-digest-v2";
const WORLD_FILE: &str = "world.lfrs";
const CALLER_FILE: &str = "caller.json";
const MANIFEST_FILE: &str = "save.toml";
const RESTORE_LIMITS: SnapshotRestoreLimits = SnapshotRestoreLimits::new(2_147_483_648, 4_096);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedIndividual {
    pub id: IndividualId,
    pub crossed_tile: bool,
    pub vehicle: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedRoute {
    pub key: String,
    pub snapshot_route_id: u64,
}

/// `urban-save-caller-v1`：宿主耐久状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallerState {
    pub version: String,
    pub individuals: Vec<SavedIndividual>,
    /// 按路线序号（目录 key 升序）排列。
    pub routes: Vec<SavedRoute>,
    pub schedule: Vec<Request>,
    pub ledger: SavedLedger,
    pub recycling: Option<Vec<Option<u32>>>,
    pub evidence: Vec<TileEvidence>,
    pub red_waiters: Vec<(IndividualId, u32)>,
    pub claims: Vec<((IndividualId, u32, u32), u32)>,
    pub error_counts: BTreeMap<String, u64>,
    pub atomic_rejections: BTreeMap<String, u32>,
    pub watched: BTreeMap<String, u32>,
    pub replacements: u64,
    pub births: u64,
    pub removals: u64,
}

/// 生成存档的执行来源；只作记录，不参与续跑判据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveSource {
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_digest: Option<String>,
    pub workers: u32,
}

/// `urban-save-v3` 清单（#1035：确定性状态摘要改为 BLAKE3-256 后由 v1 递增为 v2；
/// #1042：文件字节摘要、宿主状态摘要与观测摘要改为 BLAKE3-256 后递增为 v3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveManifest {
    pub version: String,
    pub tick: u64,
    pub time_ms: u64,
    pub command_cursor: u64,
    pub event_cursor: u64,
    pub fixed_step_ms: u64,
    pub plan_digest: String,
    pub observation_version: String,
    pub deterministic_state_digest: String,
    pub state_digest: String,
    /// 存档时内存中宿主状态的 `urban-caller-digest-v2` 摘要；恢复后重算比对，
    /// 发现本程序导出与导入的不对称。
    pub caller_digest: String,
    pub world: FileDigest,
    pub caller: FileDigest,
    pub source: SaveSource,
    pub artifact_files: BTreeMap<String, ArtifactFileDigest>,
}

/// 回执中的一次存档。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveRecord {
    pub tick: u64,
    pub path: String,
    pub manifest: FileDigest,
}

/// 存档写出各段耗时（只进 diagnostics，不进四项计时）。
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct SaveTiming {
    pub tick: u64,
    pub capture_ns: u64,
    pub encode_ns: u64,
    pub write_ns: u64,
}

/// 回执中的续跑来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumedFrom {
    pub tick: u64,
    pub manifest_digest: String,
    pub source: SaveSource,
}

/// 已核对文件摘要与版本、尚未恢复世界的存档。
pub(crate) struct LoadedSave {
    pub manifest: SaveManifest,
    pub manifest_digest: FileDigest,
    world: Vec<u8>,
    caller: CallerState,
}

fn elapsed_ns(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

/// 第 T 拍完成 advance、逐拍日志与检查点之后写出存档。先写临时目录，三个文件
/// 都完成后再改名，不留半个存档目录被当成有效存档。
pub(crate) fn write_save(
    output: &Path,
    harness: &Harness<'_>,
    state_digest: &str,
    plan_digest: &str,
    source: SaveSource,
) -> Result<(SaveRecord, SaveTiming)> {
    let tick = harness.world.tick_index();
    let name = format!("tick-{tick}");
    let saves = output.join("saves");
    fs::create_dir_all(&saves)?;
    let target = saves.join(&name);
    let partial = saves.join(format!(".{name}.partial"));
    if target.exists() || partial.exists() {
        return Err(invalid(format!("save already exists: saves/{name}")));
    }
    let started = Instant::now();
    let snapshot = checked("capture save", harness.world.capture_snapshot())?;
    let routes: Vec<_> = harness.world.live_routes().collect();
    let vehicles = harness.world.live_vehicles();
    if routes.len() != snapshot.routes().len() || vehicles.len() != snapshot.live_order().len() {
        return Err(invalid("snapshot identity cardinalities differ"));
    }
    let route_ids: HashMap<_, _> = routes
        .iter()
        .zip(snapshot.routes())
        .map(|(handle, captured)| (*handle, captured.snapshot_route_id()))
        .collect();
    let vehicle_ids: HashMap<_, _> = vehicles
        .iter()
        .copied()
        .zip(snapshot.live_order().iter().copied())
        .collect();
    let deterministic = format!(
        "{:x}",
        checked("save digest", deterministic_state_digest(&snapshot))?
    );
    let capture_ns = elapsed_ns(started);
    let started = Instant::now();
    let world = encode_lfrs(&snapshot);
    let caller = serde_json::to_vec(&harness.export_caller(&route_ids, &vehicle_ids)?)?;
    let caller_digest = harness.caller_digest()?;
    let encode_ns = elapsed_ns(started);
    let started = Instant::now();
    fs::create_dir(&partial)?;
    // 写后读回校验、有界重写、耗尽删残件（#954）；rename 是发布边界，其前全部校验完成。
    let write_verified = |path: std::path::PathBuf, bytes: &[u8]| -> Result<()> {
        verified_write::write_verified(
            &path,
            bytes,
            &verified_write::SourceRegistration::of(bytes),
            verified_write::WritePolicy::DEFAULT,
        )
        .map(|_| ())
        .map_err(|error| invalid(format!("save write-readback {}: {error}", path.display())))
    };
    write_verified(partial.join(WORLD_FILE), &world)?;
    write_verified(partial.join(CALLER_FILE), &caller)?;
    let manifest = SaveManifest {
        version: SAVE_VERSION.into(),
        tick,
        time_ms: harness.world.time_ms(),
        command_cursor: harness.world.command_cursor(),
        event_cursor: harness.world.event_cursor(),
        fixed_step_ms: harness.plan.dt,
        plan_digest: plan_digest.into(),
        observation_version: observe::OBSERVATION_VERSION.into(),
        deterministic_state_digest: deterministic,
        state_digest: state_digest.into(),
        caller_digest,
        world: FileDigest::of(&world),
        caller: FileDigest::of(&caller),
        source,
        artifact_files: harness.artifacts.files.clone(),
    };
    let manifest_bytes = toml::to_string_pretty(&manifest)?.into_bytes();
    write_verified(partial.join(MANIFEST_FILE), &manifest_bytes)?;
    fs::rename(&partial, &target)?;
    let write_ns = elapsed_ns(started);
    Ok((
        SaveRecord {
            tick,
            path: format!("saves/{name}"),
            manifest: FileDigest::of(&manifest_bytes),
        },
        SaveTiming {
            tick,
            capture_ns,
            encode_ns,
            write_ns,
        },
    ))
}

/// 读取并核对存档：清单与两份文件的字节数和 SHA-256、载荷版本、观测编码版本、
/// 计划摘要与制品文件摘要。不尝试转换。
pub(crate) fn load_save(
    directory: &Path,
    artifacts: &Artifacts,
    plan_digest: &str,
) -> Result<LoadedSave> {
    let manifest_bytes = fs::read(directory.join(MANIFEST_FILE))?;
    let manifest: SaveManifest = toml::from_str(
        std::str::from_utf8(&manifest_bytes).map_err(|error| invalid(error.to_string()))?,
    )?;
    if manifest.version != SAVE_VERSION {
        return Err(invalid("unsupported save version"));
    }
    if manifest.observation_version != observe::OBSERVATION_VERSION {
        return Err(invalid(
            "save observation encoding differs from this binary; state digest cannot be checked",
        ));
    }
    if manifest.plan_digest != plan_digest {
        return Err(invalid("save belongs to a different resolved plan"));
    }
    if manifest.artifact_files != artifacts.files {
        return Err(invalid("save belongs to different artifacts"));
    }
    let read = |name: &str, expected: &FileDigest| -> Result<Vec<u8>> {
        let bytes = fs::read(directory.join(name))?;
        if &FileDigest::of(&bytes) != expected {
            return Err(invalid(format!("save file changed: {name}")));
        }
        Ok(bytes)
    };
    let world = read(WORLD_FILE, &manifest.world)?;
    let caller: CallerState = serde_json::from_slice(&read(CALLER_FILE, &manifest.caller)?)?;
    if caller.version != CALLER_VERSION {
        return Err(invalid("unsupported caller state version"));
    }
    Ok(LoadedSave {
        manifest_digest: FileDigest::of(&manifest_bytes),
        manifest,
        world,
        caller,
    })
}

impl<'a> Harness<'a> {
    /// 宿主耐久状态；车辆与路线以本次捕获的快照局部标识表示。
    pub(crate) fn export_caller(
        &self,
        route_ids: &HashMap<RouteHandle, u64>,
        vehicle_ids: &HashMap<VehicleHandle, u64>,
    ) -> Result<CallerState> {
        let individuals = self
            .individuals
            .iter()
            .map(|individual| {
                Ok(SavedIndividual {
                    id: individual.id,
                    crossed_tile: individual.crossed_tile,
                    vehicle: individual
                        .handle
                        .map(|handle| {
                            vehicle_ids
                                .get(&handle)
                                .copied()
                                .ok_or_else(|| invalid("caller vehicle absent from snapshot"))
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<_>>()?;
        let routes = (0..self.route_table.len() as u32)
            .map(|index| {
                Ok(SavedRoute {
                    key: self.route_table.key(index).into(),
                    snapshot_route_id: *route_ids
                        .get(&self.route_table.handle(index))
                        .ok_or_else(|| invalid("caller route absent from snapshot"))?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(CallerState {
            version: CALLER_VERSION.into(),
            individuals,
            routes,
            schedule: self.schedule.values().flatten().cloned().collect(),
            ledger: self.requests.save(),
            recycling: self
                .recycling
                .as_ref()
                .map(|controller| controller.pending_by_slot.clone()),
            evidence: self.evidence.clone(),
            red_waiters: self.red_waiters.iter().copied().collect(),
            claims: self.claims.iter().map(|(k, v)| (*k, *v)).collect(),
            error_counts: self.error_counts.clone(),
            atomic_rejections: self.atomic_rejections.clone(),
            watched: self.watched.clone(),
            replacements: self.replacements,
            births: self.births,
            removals: self.removals,
        })
    }

    /// 终点宿主状态摘要（`urban-caller-digest-v2`）：与槽位无关的规范形式。
    /// 车辆以其在世界 live 序中的位置表示，路线以 key 连同边序列表示。
    pub(crate) fn caller_digest(&self) -> Result<String> {
        #[derive(Serialize)]
        struct Payload<'p> {
            version: &'static str,
            individuals: Vec<(IndividualId, bool, Option<usize>)>,
            routes: Vec<(&'p str, &'p [String])>,
            schedule: Vec<&'p Request>,
            ledger: SavedLedger,
            recycling: Option<&'p [Option<u32>]>,
            evidence: &'p [TileEvidence],
            red_waiters: &'p BTreeSet<(IndividualId, u32)>,
            claims: Vec<(&'p (IndividualId, u32, u32), &'p u32)>,
            error_counts: &'p BTreeMap<String, u64>,
            atomic_rejections: &'p BTreeMap<String, u32>,
            watched: &'p BTreeMap<String, u32>,
            replacements: u64,
            births: u64,
            removals: u64,
        }
        let positions: HashMap<_, _> = self
            .world
            .live_vehicles()
            .iter()
            .enumerate()
            .map(|(position, handle)| (*handle, position))
            .collect();
        let individuals = self
            .individuals
            .iter()
            .map(|individual| {
                Ok((
                    individual.id,
                    individual.crossed_tile,
                    individual
                        .handle
                        .map(|handle| {
                            positions
                                .get(&handle)
                                .copied()
                                .ok_or_else(|| invalid("caller vehicle is not live"))
                        })
                        .transpose()?,
                ))
            })
            .collect::<Result<_>>()?;
        let mut schedule: Vec<_> = self.schedule.values().flatten().collect();
        schedule.sort_by_key(|request| (request.due, request.sequence, request.attempt));
        let payload = Payload {
            version: CALLER_DIGEST_VERSION,
            individuals,
            routes: (0..self.route_table.len() as u32)
                .map(|index| {
                    (
                        self.route_table.key(index),
                        self.route_table.edge_keys(index),
                    )
                })
                .collect(),
            schedule,
            ledger: self.requests.save(),
            recycling: self
                .recycling
                .as_ref()
                .map(|controller| controller.pending_by_slot.as_slice()),
            evidence: &self.evidence,
            red_waiters: &self.red_waiters,
            claims: self.claims.iter().collect(),
            error_counts: &self.error_counts,
            atomic_rejections: &self.atomic_rejections,
            watched: &self.watched,
            replacements: self.replacements,
            births: self.births,
            removals: self.removals,
        };
        Ok(Domain::CallerState.hex(&serde_json::to_vec(&payload)?))
    }

    /// 从已核对文件的存档恢复：结构核对 → 恢复世界并重绑 → 语义核对。
    pub(crate) fn resume(
        artifacts: &'a Artifacts,
        plan: &'a ResolvedPlan,
        execution: ExecutionConfig,
        save: LoadedSave,
    ) -> Result<Self> {
        plan.validate(artifacts)?;
        let LoadedSave {
            manifest,
            world,
            caller,
            ..
        } = save;
        let restored = checked(
            "restore save",
            restore_lfrs(
                &world,
                artifacts.revision.clone(),
                artifacts.committed_source()?,
                artifacts.world_config()?,
                execution,
                RESTORE_LIMITS,
            ),
        )?;
        drop(world);
        artifacts.check_policy(restored.world())?;
        let (targets, target_keys, profiles) = catalog_bindings(artifacts)?;
        let mut catalog_routes: Vec<_> = artifacts.catalog.routes.iter().collect();
        catalog_routes.sort_by_key(|route| &route.key);
        check_structure(
            plan,
            &caller,
            manifest.tick,
            &restored,
            &catalog_routes,
            &targets,
            &profiles,
        )?;

        // 重绑：结构核对已保证每个局部标识恰好对应一个恢复出的句柄。
        let lengths = artifacts.revision.traffic().lane_lengths_millimetres();
        let mut routes = BTreeMap::new();
        let mut route_table = crate::route_table::RouteTable::default();
        for (saved, route) in caller.routes.iter().zip(&catalog_routes) {
            let handle = restored
                .route_handle(saved.snapshot_route_id)
                .expect("structure check covers route ids");
            let edges: Vec<_> = route
                .edge_keys
                .iter()
                .map(|key| artifacts.edges[key])
                .collect();
            routes.insert(route.key.clone(), handle);
            route_table.register(
                handle,
                route.key.clone(),
                route.edge_keys.clone(),
                &edges,
                lengths,
            );
        }
        let mut individuals = Vec::with_capacity(caller.individuals.len());
        let mut slots = crate::fastmap::FastMap::default();
        for (slot, saved) in caller.individuals.iter().enumerate() {
            let handle = saved.vehicle.map(|id| {
                restored
                    .vehicle_handle(id)
                    .expect("structure check covers vehicle ids")
            });
            if let Some(handle) = handle {
                slots.insert(handle, slot);
            }
            individuals.push(Individual {
                id: saved.id,
                handle,
                crossed_tile: saved.crossed_tile,
            });
        }
        let mut schedule: BTreeMap<u64, Vec<Request>> = BTreeMap::new();
        for request in &caller.schedule {
            schedule
                .entry(request.due)
                .or_default()
                .push(request.clone());
        }
        let world = restored.into_world();
        let last_signals = crate::runner::signal_signature(&world)?;
        let mut harness = Self {
            artifacts,
            plan,
            world: crate::host::Host::Headless(Box::new(world)),
            individuals,
            slots,
            routes,
            profiles,
            route_table,
            targets,
            target_keys,
            schedule,
            requests: RequestLedger::restore(&caller.ledger)?,
            recycling: caller
                .recycling
                .map(|pending_by_slot| CompletedRecycling { pending_by_slot }),
            evidence: caller.evidence,
            red_waiters: caller.red_waiters.into_iter().collect(),
            claims: caller.claims.into_iter().collect(),
            transition_gates: observe::transition_gates(artifacts),
            step_before: Vec::new(),
            scratch_red: crate::fastmap::FastMap::default(),
            commands: Vec::new(),
            events: Vec::new(),
            error_counts: caller.error_counts,
            atomic_rejections: caller.atomic_rejections,
            watched: caller.watched,
            replacements: caller.replacements,
            births: caller.births,
            removals: caller.removals,
            last_signals,
            last_step_ns: 0,
            last_command_ns: 0,
            last_observation_ns: 0,
            observation: Default::default(),
            pool: observation_pool(execution)?,
        };
        harness.check_restored(&manifest)?;
        Ok(harness)
    }

    /// 语义核对：清单拍号与游标、完整检查点、路线边序列、观测摘要、生命周期计数、
    /// 停车不变量。观测摘要不读上一拍对照：step_before 置为等长全 None，不产生
    /// 生命周期事件、不改动 crossed_tile 与证据计数。
    fn check_restored(&mut self, manifest: &SaveManifest) -> Result<()> {
        if (
            self.world.tick_index(),
            self.world.time_ms(),
            self.world.command_cursor(),
            self.world.event_cursor(),
        ) != (
            manifest.tick,
            manifest.time_ms,
            manifest.command_cursor,
            manifest.event_cursor,
        ) {
            return Err(invalid(
                "restored world clock or cursors differ from the save manifest",
            ));
        }
        if self.checkpoint()? != manifest.deterministic_state_digest {
            return Err(invalid("restored deterministic state digest differs"));
        }
        for index in 0..self.route_table.len() as u32 {
            let edges = self
                .world
                .route_edges(self.route_table.handle(index))
                .ok_or_else(|| invalid("restored route is not registered"))?;
            if !edges.iter().map(|edge| edge.raw()).eq(self
                .route_table
                .edge_raws(index)
                .iter()
                .copied())
            {
                return Err(invalid(format!(
                    "restored route edges differ: {}",
                    self.route_table.key(index)
                )));
            }
        }
        self.step_before = vec![None; self.individuals.len()];
        let (state_digest, counts) = observe::state(self)?;
        if state_digest != manifest.state_digest {
            return Err(invalid("restored observation state digest differs"));
        }
        if !self.events.is_empty() {
            return Err(invalid("restored observation produced events"));
        }
        if observe::counts(self)? != counts {
            return Err(invalid("restored lifecycle counts differ"));
        }
        observe::parking_invariants(self)?;
        if self.caller_digest()? != manifest.caller_digest {
            return Err(invalid(
                "restored caller state digest differs from the save manifest",
            ));
        }
        Ok(())
    }
}

/// 结构核对：`caller.json` 的每个下标、长度与引用在任何使用之前核对。
fn check_structure(
    plan: &ResolvedPlan,
    caller: &CallerState,
    tick: u64,
    restored: &RestoredSnapshot,
    catalog_routes: &[&laneflow_urban_generator::Route],
    targets: &BTreeMap<String, ParkingTarget>,
    profiles: &BTreeMap<String, laneflow_static_contract::VehicleProfileOrdinal>,
) -> Result<()> {
    let count = caller.individuals.len();
    if count != plan.initial.len()
        || caller
            .individuals
            .iter()
            .zip(&plan.initial)
            .any(|(saved, initial)| (saved.id.tile, saved.id.slot) != (initial.tile, initial.slot))
    {
        return Err(invalid(
            "saved individual table differs from the plan layout",
        ));
    }
    if caller.evidence.len() != plan.tiles as usize {
        return Err(invalid(
            "saved tile evidence does not have one entry per plan tile",
        ));
    }
    if caller.recycling.is_some() != plan.recycling.is_some()
        || caller
            .recycling
            .as_ref()
            .is_some_and(|pending| pending.len() != count)
    {
        return Err(invalid("saved recycling controller differs from the plan"));
    }
    // 路线：目录 key 与快照路线 ID 完整一一对应。
    let route_ids: BTreeSet<_> = restored
        .route_mappings()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let saved_route_ids: BTreeSet<_> = caller.routes.iter().map(|r| r.snapshot_route_id).collect();
    if caller.routes.len() != catalog_routes.len()
        || caller
            .routes
            .iter()
            .zip(catalog_routes)
            .any(|(saved, route)| saved.key != route.key)
        || saved_route_ids.len() != caller.routes.len()
        || saved_route_ids != route_ids
    {
        return Err(invalid(
            "saved route bindings are not a one-to-one cover of restored routes",
        ));
    }
    // 车辆：有车辆的个体与恢复出的车辆完整一一对应。
    let vehicle_ids: BTreeSet<_> = restored
        .vehicle_mappings()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let saved_vehicles: Vec<_> = caller
        .individuals
        .iter()
        .filter_map(|i| i.vehicle)
        .collect();
    let saved_vehicle_ids: BTreeSet<_> = saved_vehicles.iter().copied().collect();
    if saved_vehicle_ids.len() != saved_vehicles.len() || saved_vehicle_ids != vehicle_ids {
        return Err(invalid(
            "saved vehicle bindings are not a one-to-one cover of restored vehicles",
        ));
    }
    let individual = |id: &IndividualId| {
        let index = id.tile as usize * 1_000 + id.slot as usize;
        index < count && caller.individuals[index].id == *id
    };
    if caller
        .claims
        .iter()
        .any(|((id, route, _), _)| !individual(id) || *route as usize >= catalog_routes.len())
        || caller.red_waiters.iter().any(|(id, _)| !individual(id))
    {
        return Err(invalid(
            "saved claims or red waiters reference unknown entities",
        ));
    }
    // 排队请求：引用在范围内，并与请求登记表双向一一对应。
    let route_known = |key: &str| {
        catalog_routes
            .binary_search_by(|r| r.key.as_str().cmp(key))
            .is_ok()
    };
    let mut queued = BTreeMap::new();
    for request in &caller.schedule {
        use crate::runner::Command;
        let references = match &request.command {
            Command::Replace { route, .. } => route_known(route),
            Command::Spawn { profile, route, .. } => {
                route_known(route) && profiles.contains_key(profile)
            }
            Command::Leave { target, .. }
            | Command::Reserve { target, .. }
            | Command::Park { target } => targets.contains_key(target),
            Command::Despawn => true,
        };
        if request.due < tick || request.slot >= count || !references {
            return Err(invalid(format!(
                "saved request {} is stale or references unknown entities",
                request.sequence
            )));
        }
        if queued
            .insert(request.sequence, (request.due, request.attempt))
            .is_some()
        {
            return Err(invalid(format!(
                "saved request {} is queued twice",
                request.sequence
            )));
        }
    }
    let ledger = RequestLedger::restore(&caller.ledger)?;
    let registered: BTreeMap<_, _> = ledger
        .queued()
        .map(|(sequence, due, attempt)| (sequence, (due, attempt)))
        .collect();
    if queued != registered {
        return Err(invalid("saved request queue and ledger disagree"));
    }
    if let Some(pending) = &caller.recycling {
        let mut owners = BTreeMap::new();
        for request in &caller.schedule {
            if matches!(request.retry, RetryPolicy::UntilWindowEnd { .. }) {
                owners.insert(request.sequence, request.slot);
            }
        }
        let claimed: BTreeMap<_, _> = pending
            .iter()
            .enumerate()
            .filter_map(|(slot, sequence)| sequence.map(|sequence| (sequence, slot)))
            .collect();
        if owners != claimed {
            return Err(invalid(
                "saved recycling owners and queued recycling requests disagree",
            ));
        }
    }
    Ok(())
}
