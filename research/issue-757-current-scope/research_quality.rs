//! #757 研究观察；纳入完整 iteration，不进入 Runtime step 计时。仅有拍级时间分辨率。
use crate::{Harness, Result, runner::IndividualId};
use laneflow_runtime::{TrafficTransitionKind, VehicleHandle, VehicleStatus};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

struct Trip {
    id: IndividualId,
    start: u64,
    left: bool,
    stop: u64,
    run: u64,
    max_run: u64,
}
pub(crate) struct Quality {
    trips: Vec<Option<Trip>>,
    writer: BufWriter<File>,
    entered: HashMap<(VehicleHandle, laneflow_runtime::RouteHandle, u32), (u32, u64)>,
    held: HashMap<VehicleHandle, u64>,
    zone_runs: HashMap<u32, u64>,
    overlap_zone_ms: u64,
    max_overlap_zone_ms: u64,
    overlap_events: u64,
    orphan_clears: u64,
    released: u64,
    max_hold_ms: u64,
    completed: u64,
    max_stop_ms: u64,
    measured_ns: u128,
    distance_um: u128,
    hard_brakes: u64,
}
impl Quality {
    pub(crate) fn new(h: &Harness<'_>, output: &Path) -> Result<Self> {
        let mut writer = BufWriter::new(File::create(output.join("individual-quality.csv"))?);
        writeln!(
            writer,
            "tile,slot,incarnation,start_ms,end_ms,left_censored,right_censored,end_kind,total_stop_ms,max_continuous_stop_ms"
        )?;
        let trips = h
            .individuals
            .iter()
            .map(|i| {
                i.handle
                    .and_then(|v| h.world.vehicle(v))
                    .filter(|s| s.status() == VehicleStatus::Active)
                    .map(|_| Trip {
                        id: i.id,
                        start: 0,
                        left: true,
                        stop: 0,
                        run: 0,
                        max_run: 0,
                    })
            })
            .collect();
        Ok(Self {
            trips,
            writer,
            entered: HashMap::new(),
            held: HashMap::new(),
            zone_runs: HashMap::new(),
            overlap_zone_ms: 0,
            max_overlap_zone_ms: 0,
            overlap_events: 0,
            orphan_clears: 0,
            released: 0,
            max_hold_ms: 0,
            completed: 0,
            max_stop_ms: 0,
            measured_ns: 0,
            distance_um: 0,
            hard_brakes: 0,
        })
    }
    fn write_trip(&mut self, t: Trip, now: u64, right: bool, kind: &str) -> Result<()> {
        self.max_stop_ms = self.max_stop_ms.max(t.max_run);
        writeln!(
            self.writer,
            "{},{},{},{},{},{},{},{},{},{}",
            t.id.tile,
            t.id.slot,
            t.id.incarnation,
            t.start,
            now,
            t.left,
            right,
            kind,
            t.stop,
            t.max_run
        )?;
        Ok(())
    }
    pub(crate) fn observe(&mut self, h: &Harness<'_>) -> Result<()> {
        let timer = std::time::Instant::now();
        let now = h.world.time_ms();
        let dt = h.plan.dt;
        for (index, i) in h.individuals.iter().enumerate() {
            let state = i.handle.and_then(|v| h.world.vehicle(v));
            if let (Some(before), Some(after)) = (h.step_before[index], state) {
                if before.handle() == after.handle()
                    && before.route() == after.route()
                    && before.status() == VehicleStatus::Active
                {
                    let edges = h
                        .world
                        .route_edges(before.route())
                        .expect("registered route");
                    let lengths = h.artifacts.revision.traffic().lane_lengths_millimetres();
                    let position = |s: laneflow_runtime::VehicleState| -> u128 {
                        let prefix: u128 = edges[..s.route_edge_index() as usize]
                            .iter()
                            .map(|e| u128::from(lengths[e.index()]) * 1_000)
                            .sum();
                        prefix + u128::from(s.progress_mm()) * 1_000 + u128::from(s.carry_um())
                    };
                    self.distance_um += position(after).saturating_sub(position(before));
                    if before.speed_mm_s().saturating_sub(after.speed_mm_s()) as u64 > 8 * dt {
                        self.hard_brakes += 1;
                    }
                }
            }
            if self.trips[index].as_ref().is_some_and(|t| t.id != i.id) {
                let old = self.trips[index].take().unwrap();
                self.write_trip(old, now.saturating_sub(dt), true, "replaced")?;
            }
            if let Some(s) = state {
                if s.status() == VehicleStatus::Active {
                    let t = self.trips[index].get_or_insert(Trip {
                        id: i.id,
                        start: now.saturating_sub(dt),
                        left: false,
                        stop: 0,
                        run: 0,
                        max_run: 0,
                    });
                    if s.speed_mm_s() <= 100 {
                        t.stop += dt;
                        t.run += dt;
                        t.max_run = t.max_run.max(t.run);
                    } else {
                        t.run = 0;
                    }
                } else if let Some(t) = self.trips[index].take() {
                    if s.status() == VehicleStatus::Completed {
                        self.completed += 1;
                    }
                    self.write_trip(
                        t,
                        now,
                        false,
                        if s.status() == VehicleStatus::Completed {
                            "completed"
                        } else {
                            "parked"
                        },
                    )?;
                }
            } else if let Some(t) = self.trips[index].take() {
                self.write_trip(t, now, true, "absent")?;
            }
        }
        for event in h.world.latest_transition_events() {
            let v = event.vehicle();
            match event.kind() {
                TrafficTransitionKind::ConflictEntered { passage: p } => {
                    let zone = p.address().zone().raw();
                    if self
                        .entered
                        .iter()
                        .any(|((other, _, _), (z, _))| *other != v && *z == zone)
                    {
                        self.overlap_events += 1;
                    }
                    self.entered
                        .insert((v, p.route(), p.conflict_occurrence_index()), (zone, now));
                }
                TrafficTransitionKind::ConflictCleared { passage: p } => {
                    if self
                        .entered
                        .remove(&(v, p.route(), p.conflict_occurrence_index()))
                        .is_none()
                    {
                        self.orphan_clears += 1;
                    }
                }
                TrafficTransitionKind::ReservationAcquired { .. } => {
                    self.held.insert(v, now);
                }
                TrafficTransitionKind::ReservationReleased { .. } => {
                    if let Some(start) = self.held.remove(&v) {
                        self.max_hold_ms = self.max_hold_ms.max(now - start);
                        self.released += 1;
                    }
                }
                _ => {}
            }
        }
        let mut owners: HashMap<u32, HashSet<VehicleHandle>> = HashMap::new();
        for ((v, _, _), (zone, _)) in &self.entered {
            owners.entry(*zone).or_default().insert(*v);
        }
        self.zone_runs
            .retain(|zone, _| owners.get(zone).is_some_and(|v| v.len() > 1));
        for (zone, vehicles) in owners {
            if vehicles.len() > 1 {
                let run = self.zone_runs.entry(zone).or_default();
                *run += dt;
                self.overlap_zone_ms += dt;
                self.max_overlap_zone_ms = self.max_overlap_zone_ms.max(*run);
            }
        }
        self.measured_ns += timer.elapsed().as_nanos();
        Ok(())
    }
    pub(crate) fn finish(mut self, h: &Harness<'_>, output: &Path) -> Result<()> {
        let now = h.world.time_ms();
        for index in 0..self.trips.len() {
            if let Some(t) = self.trips[index].take() {
                self.write_trip(t, now, true, "window-end")?;
            }
        }
        self.writer.flush()?;
        let max_open_age = self.held.values().map(|s| now - s).max().unwrap_or(0);
        let completed_holding = self
            .held
            .keys()
            .filter(|v| {
                h.world
                    .vehicle(**v)
                    .is_some_and(|s| s.status() == VehicleStatus::Completed)
            })
            .count();
        std::fs::write(
            output.join("extended-quality.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "ticks":h.world.tick_index(),"observed_completed":self.completed,"max_continuous_stop_ms":self.max_stop_ms,
                "multi_owner_zone_ms":self.overlap_zone_ms,"longest_multi_owner_zone_ms":self.max_overlap_zone_ms,
                "multi_owner_enter_observations":self.overlap_events,"unmatched_clear_events":self.orphan_clears,
                "open_passages":self.entered.len(),"open_reservations":self.held.len(),"released_reservations":self.released,
                "max_released_reservation_ms":self.max_hold_ms,"max_open_reservation_age_ms":max_open_age,
                "completed_owners_still_holding":completed_holding,"extra_observation_ns":self.measured_ns,
                "distance_um":self.distance_um,"decelerations_over_8_m_s2":self.hard_brakes,
                "boundary":"inside iteration, outside Core; same-zone multi-owner exposure is not geometric collision certification; initial in-zone occupancy may be left-censored; distance excludes command relocations"
            }))?,
        )?;
        Ok(())
    }
}
