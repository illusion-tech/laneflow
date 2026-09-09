use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use crate::{Artifacts, Result, artifacts::FileDigest, invalid, sha256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum UrbanCase {
    #[serde(rename = "MIXED-PEAK")]
    MixedPeak,
    #[serde(rename = "GARAGE-EGRESS")]
    GarageEgress,
    #[serde(rename = "GARAGE-INGRESS")]
    GarageIngress,
    #[serde(rename = "WAITING-RELEASE")]
    WaitingRelease,
    #[serde(rename = "PERMISSIVE-LEFT")]
    PermissiveLeft,
    #[serde(rename = "UNCONTROLLED-YIELD")]
    UncontrolledYield,
    #[serde(rename = "BOUNDARY-BURST")]
    BoundaryBurst,
}

impl UrbanCase {
    pub const ALL: [Self; 7] = [
        Self::MixedPeak,
        Self::GarageEgress,
        Self::GarageIngress,
        Self::WaitingRelease,
        Self::PermissiveLeft,
        Self::UncontrolledYield,
        Self::BoundaryBurst,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MixedPeak => "MIXED-PEAK",
            Self::GarageEgress => "GARAGE-EGRESS",
            Self::GarageIngress => "GARAGE-INGRESS",
            Self::WaitingRelease => "WAITING-RELEASE",
            Self::PermissiveLeft => "PERMISSIVE-LEFT",
            Self::UncontrolledYield => "UNCONTROLLED-YIELD",
            Self::BoundaryBurst => "BOUNDARY-BURST",
        }
    }
}

impl std::str::FromStr for UrbanCase {
    type Err = crate::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|case| case.as_str() == value)
            .ok_or_else(|| invalid(format!("unknown urban case: {value}")))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Window {
    pub purpose: String,
    pub warm_up_ticks: u64,
    pub observation_ticks: u64,
}

impl Window {
    pub fn correctness(artifacts: &Artifacts) -> Result<Self> {
        if !matches!(artifacts.catalog.scale.as_str(), "10k" | "100k") {
            return Err(invalid(
                "correctness requires 10k or 100k artifacts; use probe for fixtures",
            ));
        }
        let cycle = cycle_ticks(artifacts)?;
        Ok(Self {
            purpose: "correctness".into(),
            warm_up_ticks: cycle,
            observation_ticks: 2 * cycle,
        })
    }
    pub fn probe(ticks: u64) -> Result<Self> {
        Self::probe_after(0, ticks)
    }
    pub fn probe_after(warm_up_ticks: u64, observation_ticks: u64) -> Result<Self> {
        if observation_ticks == 0 {
            return Err(invalid("probe must contain ticks"));
        }
        Ok(Self {
            purpose: "probe".into(),
            warm_up_ticks,
            observation_ticks,
        })
    }
    pub fn performance(artifacts: &Artifacts) -> Result<Self> {
        if !matches!(artifacts.catalog.scale.as_str(), "10k" | "100k") {
            return Err(invalid("performance requires 10k or 100k artifacts"));
        }
        let cycle = cycle_ticks(artifacts)?;
        Ok(Self {
            purpose: "performance".into(),
            warm_up_ticks: (4 * cycle).max(512),
            observation_ticks: (8 * cycle).max(4_096),
        })
    }
    pub fn end(&self) -> u64 {
        self.warm_up_ticks + self.observation_ticks
    }

    pub(crate) fn contains_completed_step(&self, tick: u64) -> bool {
        tick > self.warm_up_ticks && tick <= self.end()
    }
}

fn cycle_ticks(artifacts: &Artifacts) -> Result<u64> {
    let dt = artifacts.dt;
    if !matches!(dt, 16 | 33)
        || artifacts.catalog.signals.iter().any(|s| {
            s.offset_ms % dt != 0
                || s.phases
                    .iter()
                    .any(|p| p.duration_ms == 0 || p.duration_ms % dt != 0)
        })
    {
        return Err(invalid(
            "phase/offset is not a positive fixed-step multiple",
        ));
    }
    let longest = artifacts
        .catalog
        .signals
        .iter()
        .map(|s| s.cycle_ms)
        .max()
        .ok_or_else(|| invalid("missing signals"))?;
    Ok(longest / dt)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct InitialVehicle {
    pub tile: u32,
    pub slot: u32,
    pub profile: String,
    pub route: String,
    pub occurrence: u32,
    pub progress_mm: u32,
    pub parking: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DepartureBatch {
    pub due_tick: u64,
    pub first_slot: u32,
    pub sequence: u32,
    /// Exactly ten routes: seven eastbound and three westbound.
    pub routes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParkingArrival {
    pub slot: u32,
    pub sequence: u32,
    pub target: String,
    pub reserve_tick: u64,
    /// An actual arrival is also required before the derived park command can run.
    pub park_not_before_tick: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParkingDeparture {
    pub due_tick: u64,
    pub slot: u32,
    pub sequence: u32,
    pub target: String,
    pub exit: usize,
    pub direction: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReservationRejection {
    pub due_tick: u64,
    pub slot: u32,
    pub sequence: u32,
    pub target: String,
    /// Closed expected result: `exclusive-occupied` or `virtual-full`.
    pub expected: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoleDeparture {
    pub due_tick: u64,
    pub slot: u32,
    pub sequence: u32,
    pub route: String,
    pub occurrence: u32,
    pub progress_mm: u32,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BoundaryWindow {
    pub tile: u32,
    pub controller: String,
    pub groups: Vec<u32>,
    pub before_tick: u64,
    pub after_tick: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LifecycleBurst {
    pub tile: u32,
    pub slot: u32,
    pub despawn_tick: u64,
    pub spawn_tick: u64,
    pub sequence: u32,
    pub profile: String,
    pub route: String,
    pub occurrence: u32,
    pub progress_mm: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LifecycleCounts {
    pub active: u32,
    pub parked: u32,
    pub completed: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResolvedPlan {
    pub version: String,
    pub case: String,
    pub seed: u32,
    pub scale: String,
    pub dt: u64,
    pub cycle_ticks: u64,
    pub tiles: u32,
    pub individuals: u32,
    pub manifest_digest: String,
    pub files: BTreeMap<String, FileDigest>,
    pub window: Window,
    pub required_per_tile: BTreeMap<String, u64>,
    pub initial_counts: LifecycleCounts,
    pub max_attempts: u32,
    pub retry_ticks: u64,
    pub initial: Vec<InitialVehicle>,
    pub departures: Vec<DepartureBatch>,
    pub leaves: Vec<ParkingDeparture>,
    pub arrivals: Vec<ParkingArrival>,
    pub reservation_rejections: Vec<ReservationRejection>,
    pub role_departures: Vec<RoleDeparture>,
    pub boundary_windows: Vec<BoundaryWindow>,
    pub lifecycle_bursts: Vec<LifecycleBurst>,
}

impl ResolvedPlan {
    /// Expands the accepted MIXED-PEAK rules without advancing a traffic world.
    pub fn mixed(artifacts: &Artifacts, window: Window) -> Result<Self> {
        Self::for_case(artifacts, UrbanCase::MixedPeak, window)
    }

    /// Expands one of the seven accepted, closed LF-CN-URBAN cases.
    pub fn for_case(artifacts: &Artifacts, case: UrbanCase, window: Window) -> Result<Self> {
        let catalog = &artifacts.catalog;
        let cycle = cycle_ticks(artifacts)?;
        match window.purpose.as_str() {
            "correctness" if window == Window::correctness(artifacts)? => {}
            "performance"
                if case == UrbanCase::MixedPeak && window == Window::performance(artifacts)? => {}
            "probe"
                if window.warm_up_ticks <= cycle
                    && window.observation_ticks <= 3 * cycle
                    && window.end() > 0 => {}
            _ => return Err(invalid("unsupported run window")),
        }
        let quantum = 528 / artifacts.dt;
        let mut initial = Vec::new();
        let mut departures = Vec::new();
        let mut leaves = Vec::new();
        let mut arrivals = Vec::new();
        let mut reservation_rejections = Vec::new();
        let mut role_departures = Vec::new();
        let mut boundary_windows = Vec::new();
        let mut lifecycle_bursts = Vec::new();
        let active_per_tile = if case == UrbanCase::GarageEgress {
            250
        } else {
            750
        };
        let mut route_choices: BTreeMap<&str, Vec<_>> = BTreeMap::new();
        let rank = |kind: &str| match kind {
            "cross-tile" => 0,
            "cross-cell" => 1,
            "junction" => 2,
            _ => 3,
        };
        for r in &catalog.routes {
            if rank(&r.category) < 3 {
                for (i, e) in r.edge_keys.iter().enumerate() {
                    route_choices.entry(e).or_default().push((r, i));
                }
            }
        }
        for candidates in route_choices.values_mut() {
            candidates.sort_by_key(|(r, i)| (rank(&r.category), &r.key, *i));
        }
        for tile in 0..artifacts.tiles {
            let prefix = format!("t{tile:03}.");
            let arms: Vec<_> = catalog
                .edge_ids
                .keys()
                .filter(|e| e.starts_with(&prefix) && (e.ends_with(".in") || e.ends_with(".out")))
                .collect();
            if arms.len() != 74 {
                return Err(invalid(format!("tile {tile}: expected 74 arms")));
            }
            let mut roles = BTreeMap::new();
            let mut arrival_fronts = BTreeMap::new();
            let mut traffic_role_edges = BTreeSet::new();
            let mut background_route_exclusions = BTreeSet::new();
            let garage = catalog
                .parking
                .iter()
                .find(|p| p.tile == tile && p.kind == "virtual" && p.capacity == 1_000)
                .ok_or_else(|| invalid("missing garage"))?;
            if matches!(case, UrbanCase::GarageEgress | UrbanCase::BoundaryBurst) {
                for anchor in &garage.exits {
                    background_route_exclusions.insert(anchor.edge.as_str());
                }
            }
            let arrival_roles: Vec<_> = match case {
                UrbanCase::MixedPeak | UrbanCase::BoundaryBurst => vec![
                    (741, "c08.bay1", "explicit-arrival"),
                    (743, "c09.mixed", "virtual-arrival"),
                ],
                UrbanCase::GarageIngress => vec![
                    (741, "c08.bay1", "explicit-arrival"),
                    (743, "c08.mixed", "virtual-arrival"),
                ],
                _ => Vec::new(),
            };
            for (slot, suffix, role) in arrival_roles {
                let target = format!("{prefix}{suffix}");
                let parking = catalog
                    .parking
                    .iter()
                    .find(|p| p.key == target)
                    .ok_or_else(|| invalid("missing arrival role target"))?;
                let anchor = parking
                    .entries
                    .first()
                    .ok_or_else(|| invalid("missing entry"))?;
                if !arms.contains(&&anchor.edge) {
                    return Err(invalid("arrival role must start on an external arm"));
                }
                let progress_mm = (0..11)
                    .map(|layer| 7_000 + 8_500 * layer)
                    .rfind(|position| *position < anchor.progress_mm)
                    .ok_or_else(|| invalid("no initial position before parking entry"))?;
                arrival_fronts.insert(anchor.edge.as_str(), progress_mm);
                roles.insert(
                    slot,
                    InitialVehicle {
                        tile,
                        slot,
                        profile: profile(slot).into(),
                        route: anchor.route.clone(),
                        occurrence: anchor.route_edge_index,
                        progress_mm,
                        parking: None,
                        role: Some(role.into()),
                    },
                );
                arrivals.push(ParkingArrival {
                    slot: tile * 1_000 + slot,
                    sequence: 11_000_000 + tile * 1_000 + slot,
                    target,
                    reserve_tick: 0,
                    park_not_before_tick: window.warm_up_ticks + u64::from(slot - 740) * quantum,
                });
            }
            if case == UrbanCase::GarageIngress {
                for (slot, suffix, role, expected) in [
                    (740, "c08.bay0", "exclusive-rejection", "exclusive-occupied"),
                    (742, "c09.mixed", "full-rejection", "virtual-full"),
                ] {
                    let target = format!("{prefix}{suffix}");
                    let parking = catalog
                        .parking
                        .iter()
                        .find(|p| p.key == target)
                        .ok_or_else(|| invalid("missing rejection role target"))?;
                    let anchor = parking
                        .entries
                        .first()
                        .ok_or_else(|| invalid("missing entry"))?;
                    let progress_mm = (0..11)
                        .map(|layer| 7_000 + 8_500 * layer)
                        .rfind(|position| {
                            *position < anchor.progress_mm
                                && arrival_fronts
                                    .get(anchor.edge.as_str())
                                    .is_none_or(|front| position < front)
                        })
                        .ok_or_else(|| invalid("no initial position for rejection role"))?;
                    arrival_fronts
                        .entry(anchor.edge.as_str())
                        .and_modify(|front| *front = (*front).min(progress_mm))
                        .or_insert(progress_mm);
                    roles.insert(
                        slot,
                        InitialVehicle {
                            tile,
                            slot,
                            profile: profile(slot).into(),
                            route: anchor.route.clone(),
                            occurrence: anchor.route_edge_index,
                            progress_mm,
                            parking: None,
                            role: Some(role.into()),
                        },
                    );
                    reservation_rejections.push(ReservationRejection {
                        due_tick: window.warm_up_ticks,
                        slot: tile * 1_000 + slot,
                        sequence: 12_000_000 + tile * 1_000 + slot,
                        target,
                        expected: expected.into(),
                    });
                    if window.warm_up_ticks > 0 {
                        role_departures.push(RoleDeparture {
                            due_tick: window.warm_up_ticks,
                            slot: tile * 1_000 + slot,
                            sequence: 13_000_000 + tile * 1_000 + slot,
                            route: anchor.route.clone(),
                            occurrence: anchor.route_edge_index,
                            progress_mm,
                            role: role.into(),
                        });
                    }
                }
            }
            let phase_delta = |cell: &str, phase_key: &str| -> Result<u64> {
                let controller_key = format!("{prefix}{cell}.controller");
                let controller = catalog
                    .signals
                    .iter()
                    .find(|signal| signal.key == controller_key)
                    .ok_or_else(|| invalid("missing role signal controller"))?;
                let phase_start = controller
                    .phases
                    .iter()
                    .take_while(|phase| phase.key != phase_key)
                    .map(|phase| phase.duration_ms)
                    .sum::<u64>();
                if !controller.phases.iter().any(|phase| phase.key == phase_key) {
                    return Err(invalid("missing role signal phase"));
                }
                let position = (window.warm_up_ticks * artifacts.dt + controller.offset_ms)
                    % controller.cycle_ms;
                Ok(
                    (phase_start + controller.cycle_ms - position) % controller.cycle_ms
                        / artifacts.dt,
                )
            };
            let waiting_left_start = phase_delta("c00", "p1.green")?;
            let c00 = catalog
                .signals
                .iter()
                .find(|signal| signal.key == format!("{prefix}c00.controller"))
                .ok_or_else(|| invalid("missing waiting controller"))?;
            let waiting_left_duration = c00
                .phases
                .iter()
                .find(|phase| phase.key == "p1.green")
                .ok_or_else(|| invalid("missing waiting release phase"))?
                .duration_ms
                / artifacts.dt;
            let waiting_phase_start = c00
                .phases
                .iter()
                .take_while(|phase| phase.key != "p1.green")
                .map(|phase| phase.duration_ms)
                .sum::<u64>()
                / artifacts.dt;
            let waiting_position = ((window.warm_up_ticks * artifacts.dt + c00.offset_ms)
                % c00.cycle_ms)
                / artifacts.dt;
            let waiting_source_due = if (waiting_phase_start
                ..waiting_phase_start + waiting_left_duration)
                .contains(&waiting_position)
            {
                waiting_phase_start + waiting_left_duration - waiting_position + 1
            } else {
                0
            };
            let waiting_cycle_ticks = c00.cycle_ms / artifacts.dt;
            if waiting_cycle_ticks == 0 {
                return Err(invalid("waiting controller cycle is shorter than one tick"));
            }
            let waiting_release_delta = if waiting_left_start == 0 {
                waiting_cycle_ticks
            } else {
                waiting_left_start
            };
            let boundary_delta = phase_delta("c00", "p0.yellow")?;
            let boundary_tick = window.warm_up_ticks
                + if boundary_delta == 0 {
                    c00.cycle_ms / artifacts.dt
                } else {
                    boundary_delta
                };
            let boundary_before = boundary_tick.saturating_sub(1);
            let traffic_roles: Vec<_> = match case {
                UrbanCase::WaitingRelease => vec![
                    (
                        740,
                        "c00.w-n",
                        0,
                        92_000,
                        92_000,
                        "waiting-fifo-0",
                        waiting_source_due,
                    ),
                    (
                        741,
                        "c00.w-n",
                        0,
                        83_500,
                        83_500,
                        "waiting-fifo-1",
                        waiting_source_due,
                    ),
                    (
                        742,
                        "c00.w-n",
                        0,
                        75_000,
                        75_000,
                        "waiting-fifo-2",
                        waiting_source_due,
                    ),
                    (
                        743,
                        "c00.w-n",
                        4,
                        7_000,
                        66_500,
                        "waiting-storage-pulse",
                        waiting_release_delta.saturating_sub(1),
                    ),
                ],
                UrbanCase::PermissiveLeft => {
                    let due = phase_delta("c01", "p0.green")?;
                    vec![
                        (740, "c01.w-n", 0, 92_000, 92_000, "permissive-left", due),
                        (741, "c01.e-w", 0, 92_000, 92_000, "opposing-pulse-0", due),
                        (742, "c01.e-w", 0, 83_500, 83_500, "opposing-pulse-1", due),
                        (743, "c01.e-w", 0, 75_000, 75_000, "opposing-pulse-2", due),
                    ]
                }
                UrbanCase::UncontrolledYield => vec![
                    (740, "c04.w-e", 0, 92_000, 92_000, "mainline-pulse-0", 0),
                    (741, "c04.w-e", 0, 83_500, 83_500, "mainline-pulse-1", 0),
                    (742, "c04.n-e", 0, 92_000, 75_000, "yield-role", 0),
                ],
                UrbanCase::BoundaryBurst => vec![
                    (
                        740,
                        "c04.w-e",
                        2,
                        92_000,
                        92_000,
                        "boundary-respawn",
                        boundary_before.saturating_sub(window.warm_up_ticks),
                    ),
                    (
                        742,
                        "c04.w-e",
                        2,
                        83_500,
                        83_500,
                        "boundary-replace",
                        boundary_before.saturating_sub(window.warm_up_ticks),
                    ),
                ],
                _ => Vec::new(),
            };
            let isolated_cell = match case {
                UrbanCase::WaitingRelease => Some("c00"),
                UrbanCase::PermissiveLeft => Some("c01"),
                UrbanCase::UncontrolledYield => Some("c04"),
                UrbanCase::BoundaryBurst => Some("c04"),
                _ => None,
            };
            if let Some(cell) = isolated_cell {
                let cell_prefix = format!("{prefix}{cell}.");
                traffic_role_edges.extend(
                    arms.iter()
                        .filter(|edge| edge.starts_with(&cell_prefix) && edge.ends_with(".in"))
                        .map(|edge| edge.as_str()),
                );
            }
            for (
                slot,
                suffix,
                departure_occurrence,
                departure_progress_mm,
                initial_progress_mm,
                role,
                due_offset,
            ) in traffic_roles
            {
                let route_key = format!("{prefix}{suffix}");
                let route = catalog
                    .routes
                    .iter()
                    .find(|route| route.key == route_key)
                    .ok_or_else(|| invalid("missing traffic role route"))?;
                let initial_occurrence =
                    if window.warm_up_ticks == 0 && case != UrbanCase::BoundaryBurst {
                        departure_occurrence
                    } else {
                        route.edge_keys.len().saturating_sub(1) as u32
                    };
                let initial_progress_mm =
                    if window.warm_up_ticks == 0 && case != UrbanCase::BoundaryBurst {
                        departure_progress_mm
                    } else {
                        initial_progress_mm
                    };
                let edge = route
                    .edge_keys
                    .get(initial_occurrence as usize)
                    .ok_or_else(|| invalid("traffic role route occurrence is missing"))?;
                if !arms.contains(&edge) || !(7_000..=92_000).contains(&initial_progress_mm) {
                    return Err(invalid(
                        "traffic role must use a fixed external-arm position",
                    ));
                }
                arrival_fronts
                    .entry(edge.as_str())
                    .and_modify(|front| *front = (*front).min(initial_progress_mm))
                    .or_insert(initial_progress_mm);
                traffic_role_edges.insert(edge.as_str());
                roles.insert(
                    slot,
                    InitialVehicle {
                        tile,
                        slot,
                        profile: profile(slot).into(),
                        route: route_key.clone(),
                        occurrence: initial_occurrence,
                        progress_mm: initial_progress_mm,
                        parking: None,
                        role: Some(role.into()),
                    },
                );
                if window.warm_up_ticks > 0 && role != "boundary-respawn" {
                    let first_due = window.warm_up_ticks + due_offset;
                    if role == "waiting-storage-pulse" {
                        let mut due_tick = first_due;
                        let mut pulse = 0;
                        while due_tick < window.end() {
                            role_departures.push(RoleDeparture {
                                due_tick,
                                slot: tile * 1_000 + slot,
                                sequence: 13_500_000 + pulse * artifacts.tiles + tile,
                                route: route_key.clone(),
                                occurrence: departure_occurrence,
                                progress_mm: departure_progress_mm,
                                role: role.into(),
                            });
                            due_tick = due_tick
                                .checked_add(waiting_cycle_ticks)
                                .ok_or_else(|| invalid("waiting pulse schedule overflow"))?;
                            pulse += 1;
                        }
                    } else {
                        role_departures.push(RoleDeparture {
                            due_tick: first_due,
                            slot: tile * 1_000 + slot,
                            sequence: 13_000_000 + tile * 1_000 + slot,
                            route: route_key.clone(),
                            occurrence: departure_occurrence,
                            progress_mm: departure_progress_mm,
                            role: role.into(),
                        });
                    }
                }
                if case == UrbanCase::BoundaryBurst && role == "boundary-respawn" {
                    lifecycle_bursts.push(LifecycleBurst {
                        tile,
                        slot: tile * 1_000 + slot,
                        despawn_tick: boundary_before,
                        spawn_tick: boundary_tick,
                        sequence: 14_000_000 + tile * 1_000 + slot,
                        profile: profile(slot).into(),
                        route: route_key,
                        occurrence: departure_occurrence,
                        progress_mm: departure_progress_mm,
                    });
                }
            }
            if matches!(case, UrbanCase::GarageEgress | UrbanCase::BoundaryBurst) {
                let slot = if case == UrbanCase::GarageEgress {
                    active_per_tile - 1
                } else {
                    744
                };
                let anchor = garage
                    .exits
                    .first()
                    .ok_or_else(|| invalid("garage lacks departure route"))?;
                let departure_route = catalog
                    .routes
                    .iter()
                    .find(|route| route.key == anchor.route)
                    .ok_or_else(|| invalid("missing garage departure route"))?;
                let due_tick = if case == UrbanCase::BoundaryBurst {
                    boundary_before.saturating_sub(1)
                } else {
                    window.warm_up_ticks.saturating_sub(1)
                };
                let initial_route = if due_tick > 0 && case == UrbanCase::BoundaryBurst {
                    catalog
                        .routes
                        .iter()
                        .find(|route| route.key == format!("{prefix}c04.w-e"))
                        .ok_or_else(|| invalid("missing boundary blocker staging route"))?
                } else {
                    departure_route
                };
                let (initial_occurrence, initial_progress_mm) = if due_tick == 0 {
                    (anchor.route_edge_index, anchor.progress_mm)
                } else if case == UrbanCase::BoundaryBurst {
                    (
                        initial_route.edge_keys.len().saturating_sub(1) as u32,
                        75_000,
                    )
                } else {
                    (
                        initial_route.edge_keys.len().saturating_sub(1) as u32,
                        92_000,
                    )
                };
                let initial_edge = initial_route
                    .edge_keys
                    .get(initial_occurrence as usize)
                    .ok_or_else(|| invalid("garage blocker route occurrence is missing"))?;
                if !arms.contains(&initial_edge) || !(7_000..=92_000).contains(&initial_progress_mm)
                {
                    return Err(invalid(
                        "garage blocker must use a fixed external-arm position",
                    ));
                }
                traffic_role_edges.insert(initial_edge.as_str());
                if roles
                    .insert(
                        slot,
                        InitialVehicle {
                            tile,
                            slot,
                            profile: profile(slot).into(),
                            route: initial_route.key.clone(),
                            occurrence: initial_occurrence,
                            progress_mm: initial_progress_mm,
                            parking: None,
                            role: Some("garage-exit-blocker".into()),
                        },
                    )
                    .is_some()
                {
                    return Err(invalid("garage blocker slot is already assigned"));
                }
                if due_tick > 0 {
                    role_departures.push(RoleDeparture {
                        due_tick,
                        slot: tile * 1_000 + slot,
                        sequence: 15_000_000 + tile * 1_000 + slot,
                        route: departure_route.key.clone(),
                        occurrence: anchor.route_edge_index,
                        progress_mm: anchor.progress_mm,
                        role: "garage-exit-blocker".into(),
                    });
                }
            }
            if case == UrbanCase::BoundaryBurst {
                let controller_key = format!("{prefix}c00.controller");
                let controller = catalog
                    .signals
                    .iter()
                    .find(|signal| signal.key == controller_key)
                    .ok_or_else(|| invalid("missing boundary controller"))?;
                let mut groups = controller
                    .phases
                    .first()
                    .ok_or_else(|| invalid("boundary controller has no phases"))?
                    .states
                    .keys()
                    .map(|key| Ok(artifacts.signal_group(key)?.raw()))
                    .collect::<Result<Vec<_>>>()?;
                groups.sort_unstable();
                boundary_windows.push(BoundaryWindow {
                    tile,
                    controller: controller_key,
                    groups,
                    before_tick: boundary_before,
                    after_tick: boundary_tick,
                });
                for arrival in arrivals
                    .iter_mut()
                    .filter(|arrival| arrival.slot / 1_000 == tile)
                {
                    arrival.park_not_before_tick = if arrival.slot % 1_000 == 741 {
                        boundary_before
                    } else {
                        boundary_tick
                    };
                }
            }
            // Roles own their initial slot and the finite forward space on their entry arm.
            // Fill all remaining individuals from the same ordered 814-position table.
            let mut positions = (0..11).flat_map(|layer| {
                let fronts = &arrival_fronts;
                let blocked = &traffic_role_edges;
                arms.iter().filter_map(move |edge| {
                    let position = 7_000 + 8_500 * layer;
                    (!blocked.contains(edge.as_str()))
                        .then_some(())
                        .and_then(|()| {
                            fronts
                                .get(edge.as_str())
                                .is_none_or(|front| position < *front)
                                .then_some((*edge, position))
                        })
                })
            });
            for slot in 0..active_per_tile {
                if let Some(role) = roles.remove(&slot) {
                    initial.push(role);
                    continue;
                }
                let (edge, position) = positions
                    .next()
                    .ok_or_else(|| invalid("not enough initial arm positions"))?;
                let choices = route_choices
                    .get(edge.as_str())
                    .ok_or_else(|| invalid("unrouted arm"))?;
                let isolated_marker = isolated_cell.map(|cell| format!(".{cell}."));
                let (route, occurrence) = choices
                    .iter()
                    .copied()
                    .find(|(route, occurrence)| {
                        route.edge_keys[*occurrence + 1..].iter().all(|edge| {
                            !background_route_exclusions.contains(edge.as_str())
                                && isolated_marker
                                    .as_ref()
                                    .is_none_or(|marker| !edge.contains(marker))
                        })
                    })
                    .ok_or_else(|| invalid("no background route avoids reserved role edges"))?;
                if artifacts.revision.traffic().lane_lengths_millimetres()
                    [artifacts.edges[edge].index()]
                    != 95_000
                    || position > 95_000
                {
                    return Err(invalid("initial arm geometry differs"));
                }
                initial.push(InitialVehicle {
                    tile,
                    slot,
                    profile: profile(slot).into(),
                    route: route.key.clone(),
                    occurrence: occurrence as u32,
                    progress_mm: position,
                    parking: None,
                    role: None,
                });
            }
            let mut targets: Vec<_> = catalog.parking.iter().filter(|p| p.tile == tile).collect();
            targets.sort_by_key(|p| {
                (
                    if p.kind == "explicit" {
                        0
                    } else if p.capacity == 100 {
                        1
                    } else {
                        2
                    },
                    &p.key,
                )
            });
            let mut next_slot = active_per_tile;
            for target in &targets {
                let count = if target.kind == "explicit" {
                    u32::from(target.key.ends_with(".bay0"))
                } else if target.capacity == 100 {
                    if case == UrbanCase::GarageIngress && target.key.ends_with(".c09.mixed") {
                        100
                    } else if case == UrbanCase::GarageEgress {
                        24
                    } else {
                        12
                    }
                } else if target.capacity == 1_000 {
                    match case {
                        UrbanCase::GarageEgress => 500,
                        UrbanCase::GarageIngress => 32,
                        _ => 120,
                    }
                } else {
                    return Err(invalid("unexpected parking capacity"));
                };
                let anchor = target
                    .exits
                    .first()
                    .ok_or_else(|| invalid("parking lacks departure route"))?;
                for _ in 0..count {
                    initial.push(InitialVehicle {
                        tile,
                        slot: next_slot,
                        profile: profile(next_slot).into(),
                        route: anchor.route.clone(),
                        occurrence: anchor.route_edge_index,
                        progress_mm: anchor.progress_mm,
                        parking: Some(target.key.clone()),
                        role: None,
                    });
                    next_slot += 1;
                }
            }
            if next_slot != 1_000 {
                return Err(invalid("initial parking allocation differs"));
            }
            let east: Vec<_> = catalog
                .routes
                .iter()
                .filter(|r| {
                    r.key.starts_with(&prefix)
                        && r.key.ends_with(".cross.e")
                        && r.edge_keys
                            .iter()
                            .all(|edge| !background_route_exclusions.contains(edge.as_str()))
                        && isolated_cell.is_none_or(|cell| {
                            let marker = format!(".{cell}.");
                            r.edge_keys.iter().all(|edge| !edge.contains(&marker))
                        })
                })
                .collect();
            let west: Vec<_> = catalog
                .routes
                .iter()
                .filter(|r| {
                    r.key.starts_with(&prefix)
                        && r.key.ends_with(".cross.w")
                        && r.edge_keys
                            .iter()
                            .all(|edge| !background_route_exclusions.contains(edge.as_str()))
                        && isolated_cell.is_none_or(|cell| {
                            let marker = format!(".{cell}.");
                            r.edge_keys.iter().all(|edge| !edge.contains(&marker))
                        })
                })
                .collect();
            if east.is_empty() || west.is_empty() {
                return Err(invalid("missing directional routes"));
            }
            let mut east = east;
            let mut west = west;
            east.sort_by_key(|r| &r.key);
            west.sort_by_key(|r| &r.key);
            for group in 0..active_per_tile / 10 {
                let mut due = u64::from((tile * 75 + group + 544) % 64) * quantum;
                let mut period = 0;
                while due < window.end() {
                    let mut routes = Vec::new();
                    for i in 0..10 {
                        let list = if i < 7 { &east } else { &west };
                        let route = list[(group as usize * 10 + i + period) % list.len()]
                            .key
                            .clone();
                        routes.push(route);
                    }
                    departures.push(DepartureBatch {
                        due_tick: due,
                        first_slot: tile * 1_000 + group * 10,
                        sequence: (period as u32 * artifacts.tiles * 75 + tile * 75 + group) * 10,
                        routes,
                    });
                    due += 64 * quantum;
                    period += 1;
                }
            }
            let garage_first_slot = active_per_tile
                + 10
                + if case == UrbanCase::GarageEgress {
                    240
                } else if case == UrbanCase::GarageIngress {
                    208
                } else {
                    120
                };
            let leave_count = if matches!(
                case,
                UrbanCase::MixedPeak | UrbanCase::GarageEgress | UrbanCase::BoundaryBurst
            ) {
                20
            } else {
                0
            };
            for i in 0..leave_count {
                let eastbound = i % 10 < 7;
                let exit = garage
                    .exits
                    .iter()
                    .position(|a| a.edge.ends_with(if eastbound { ".w.in" } else { ".e.in" }))
                    .ok_or_else(|| invalid("garage lacks a directional exit"))?;
                let due = if case == UrbanCase::BoundaryBurst {
                    if i < 10 {
                        boundary_before
                    } else {
                        boundary_tick
                    }
                } else {
                    window.warm_up_ticks + u64::from(i / 10) * 8 * quantum
                };
                if due < window.end() {
                    leaves.push(ParkingDeparture {
                        due_tick: due,
                        slot: tile * 1_000 + garage_first_slot + i,
                        sequence: 10_000_000 + tile * 20 + i,
                        target: garage.key.clone(),
                        exit,
                        direction: if eastbound { "east" } else { "west" }.into(),
                    });
                }
            }
        }
        departures.sort_by_key(|b| (b.due_tick, b.first_slot, b.sequence));
        leaves.sort_by_key(|b| (b.due_tick, b.slot, b.sequence));
        reservation_rejections.sort_by_key(|r| (r.due_tick, r.slot, r.sequence));
        role_departures.sort_by_key(|r| (r.due_tick, r.slot, r.sequence));
        boundary_windows.sort_by_key(|b| (b.before_tick, b.tile));
        lifecycle_bursts.sort_by_key(|b| (b.despawn_tick, b.tile, b.sequence));
        let required_per_tile = match case {
            UrbanCase::MixedPeak => [
                ("crossed_tile_completed".into(), 1),
                ("red_wait_then_crossed".into(), 1),
                ("park_or_leave".into(), 1),
            ]
            .into(),
            UrbanCase::GarageEgress => [
                ("garage_exit_0".into(), 1),
                ("garage_exit_1".into(), 1),
                ("safe_leave_rejection".into(), 1),
                ("retried_leave_success".into(), 1),
            ]
            .into(),
            UrbanCase::GarageIngress => [
                ("explicit_park".into(), 1),
                ("virtual_park".into(), 1),
                ("exclusive_rejection".into(), 1),
                ("full_rejection".into(), 1),
            ]
            .into(),
            UrbanCase::WaitingRelease => [
                ("waiting_entry".into(), 1),
                ("waiting_capacity_rejection".into(), 1),
                ("waiting_storage_rejection".into(), 1),
                ("waiting_release".into(), 1),
            ]
            .into(),
            UrbanCase::PermissiveLeft => [
                ("permissive_no_grant".into(), 1),
                ("permissive_grant".into(), 1),
                ("permissive_pass".into(), 1),
            ]
            .into(),
            UrbanCase::UncontrolledYield => [
                ("mainline_pass".into(), 1),
                ("yield_wait".into(), 1),
                ("yield_pass".into(), 1),
            ]
            .into(),
            UrbanCase::BoundaryBurst => [
                ("phase_change".into(), 1),
                ("lifecycle_success".into(), 1),
                ("safe_rejection".into(), 1),
                ("retry_success".into(), 1),
                ("before_boundary_command".into(), 1),
                ("after_boundary_command".into(), 1),
            ]
            .into(),
        };
        Ok(Self {
            version: "urban-demand-v2".into(),
            case: case.as_str().into(),
            seed: 544,
            scale: catalog.scale.clone(),
            dt: artifacts.dt,
            cycle_ticks: cycle,
            tiles: artifacts.tiles,
            individuals: artifacts.individuals,
            manifest_digest: artifacts.manifest_digest.clone(),
            files: artifacts.files.clone(),
            window,
            required_per_tile,
            initial_counts: LifecycleCounts {
                active: active_per_tile * artifacts.tiles,
                parked: (1_000 - active_per_tile) * artifacts.tiles,
                completed: 0,
            },
            max_attempts: 8,
            retry_ticks: 4 * quantum,
            initial,
            departures,
            leaves,
            arrivals,
            reservation_rejections,
            role_departures,
            boundary_windows,
            lifecycle_bursts,
        })
    }

    pub fn write(&self, path: &Path) -> Result<String> {
        use std::io::Write;
        let bytes = toml::to_string_pretty(self)?.into_bytes();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&bytes)?;
        Ok(sha256(&bytes))
    }
    pub fn read(path: &Path) -> Result<Self> {
        Ok(toml::from_str(&fs::read_to_string(path)?)?)
    }
    pub fn validate(&self, artifacts: &Artifacts) -> Result<()> {
        let case = self.case.parse()?;
        if self != &Self::for_case(artifacts, case, self.window.clone())? {
            return Err(invalid(
                "plan differs from the frozen expansion or source artifacts",
            ));
        }
        Ok(())
    }
}

fn profile(slot: u32) -> &'static str {
    match slot % 10 {
        0..=2 => "compact",
        3..=8 => "car",
        _ => "van",
    }
}
