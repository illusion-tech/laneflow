use std::{collections::BTreeMap, fs, path::Path};

use crate::{Artifacts, Result, artifacts::FileDigest, invalid, sha256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Window {
    pub purpose: String,
    pub warm_up_ticks: u64,
    pub observation_ticks: u64,
}

impl Window {
    pub fn correctness(artifacts: &Artifacts) -> Result<Self> {
        let cycle = cycle_ticks(artifacts)?;
        Ok(Self {
            purpose: "correctness".into(),
            warm_up_ticks: cycle,
            observation_ticks: 2 * cycle,
        })
    }
    pub fn probe(ticks: u64) -> Result<Self> {
        if ticks == 0 {
            return Err(invalid("probe must contain ticks"));
        }
        Ok(Self {
            purpose: "probe".into(),
            warm_up_ticks: 0,
            observation_ticks: ticks,
        })
    }
    pub fn end(&self) -> u64 {
        self.warm_up_ticks + self.observation_ticks
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
    pub max_attempts: u32,
    pub retry_ticks: u64,
    pub initial: Vec<InitialVehicle>,
    pub departures: Vec<DepartureBatch>,
    pub leaves: Vec<ParkingDeparture>,
    pub arrivals: Vec<ParkingArrival>,
}

impl ResolvedPlan {
    /// Expands the accepted MIXED-PEAK rules without advancing a traffic world.
    pub fn mixed(artifacts: &Artifacts, window: Window) -> Result<Self> {
        let catalog = &artifacts.catalog;
        let cycle = cycle_ticks(artifacts)?;
        if window.warm_up_ticks > cycle || window.observation_ticks > 3 * cycle {
            return Err(invalid("window exceeds this finite validation slice"));
        }
        if window.purpose == "correctness" && window != Window::correctness(artifacts)?
            || !matches!(window.purpose.as_str(), "probe" | "correctness")
            || window.end() == 0
        {
            return Err(invalid("unsupported run window"));
        }
        let quantum = 528 / artifacts.dt;
        let mut initial = Vec::new();
        let mut departures = Vec::new();
        let mut leaves = Vec::new();
        let mut arrivals = Vec::new();
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
            for (slot, suffix, role) in [
                (741, "c08.bay1", "explicit-arrival"),
                (743, "c09.mixed", "virtual-arrival"),
            ] {
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
            // Roles own their initial slot and the finite forward space on their entry arm.
            // Fill all remaining individuals from the same ordered 814-position table.
            let mut positions = (0..11).flat_map(|layer| {
                let fronts = &arrival_fronts;
                arms.iter().filter_map(move |edge| {
                    let position = 7_000 + 8_500 * layer;
                    fronts
                        .get(edge.as_str())
                        .is_none_or(|front| position < *front)
                        .then_some((*edge, position))
                })
            });
            for slot in 0..750_u32 {
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
                let (route, occurrence) = choices[0];
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
            let mut next_slot = 750;
            for target in &targets {
                let count = if target.kind == "explicit" {
                    u32::from(target.key.ends_with(".bay0"))
                } else if target.capacity == 100 {
                    12
                } else if target.capacity == 1_000 {
                    120
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
                .filter(|r| r.key.starts_with(&prefix) && r.key.ends_with(".cross.e"))
                .collect();
            let west: Vec<_> = catalog
                .routes
                .iter()
                .filter(|r| r.key.starts_with(&prefix) && r.key.ends_with(".cross.w"))
                .collect();
            if east.is_empty() || west.is_empty() {
                return Err(invalid("missing directional routes"));
            }
            let mut east = east;
            let mut west = west;
            east.sort_by_key(|r| &r.key);
            west.sort_by_key(|r| &r.key);
            for group in 0..75_u32 {
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
            let garage = targets
                .iter()
                .find(|p| p.kind == "virtual" && p.capacity == 1_000)
                .ok_or_else(|| invalid("missing garage"))?;
            for i in 0..20_u32 {
                let eastbound = i % 10 < 7;
                let exit = garage
                    .exits
                    .iter()
                    .position(|a| a.edge.ends_with(if eastbound { ".w.in" } else { ".e.in" }))
                    .ok_or_else(|| invalid("garage lacks a directional exit"))?;
                let due = window.warm_up_ticks + u64::from(i / 10) * 8 * quantum;
                if due < window.end() {
                    leaves.push(ParkingDeparture {
                        due_tick: due,
                        slot: tile * 1_000 + 880 + i,
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
        Ok(Self {
            version: "urban-demand-v2".into(),
            case: "MIXED-PEAK".into(),
            seed: 544,
            scale: catalog.scale.clone(),
            dt: artifacts.dt,
            cycle_ticks: cycle,
            tiles: artifacts.tiles,
            individuals: artifacts.individuals,
            manifest_digest: artifacts.manifest_digest.clone(),
            files: artifacts.files.clone(),
            window,
            required_per_tile: [
                ("crossed_tile_completed".into(), 1),
                ("red_wait_then_crossed".into(), 1),
                ("park_or_leave".into(), 1),
            ]
            .into(),
            max_attempts: 8,
            retry_ticks: 4 * quantum,
            initial,
            departures,
            leaves,
            arrivals,
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
        if self != &Self::mixed(artifacts, self.window.clone())? {
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
