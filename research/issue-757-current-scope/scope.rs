//! #757 隔离研究补丁：本拍工作范围。无跨拍名单或第二份交通权威。
use super::{phase::StepReadView, tick::MotionReach};
use crate::VehicleState;
use laneflow_static_network::BoundedDistance;
use std::{cell::Cell, sync::LazyLock};

pub(crate) struct Scope {
    pub(crate) p2: bool,
    pub(crate) p3: bool,
}

#[cfg(test)]
mod tests {
    use crate::{TickInput, VehicleSpawnInput};

    #[test]
    fn scope_boundaries_and_unknown_motion_are_conservative() {
        let world = crate::kernel::waiting::tests::multi_gate_world(1);
        let handle = world.state.committed.live_order[0];
        let view = world.state.read_view();
        let mut state = *view.vehicle_state(handle).unwrap();
        let route = view.compiled_route(state.route).unwrap();
        let gate = route.gate_hops[0];
        state.route_edge_index = gate;
        let length = world.traffic().lane_lengths_millimetres()[route.edges[gate as usize].index()];
        state.speed_mm_s = 0;
        state.progress_mm = length - 1;
        assert!(view.scope_near_gate(&state, 0.1, false));
        state.progress_mm = 0;
        assert!(!view.scope_near_gate(&state, 0.1, false));
        state.speed_mm_s = 100_001;
        assert!(view.scope_near_gate(&state, 0.1, false));
        state.speed_mm_s = 0;
        assert!(view.scope_near_gate(&state, f32::NAN, false));
        state.route_edge_index = gate + 1;
        assert!(view.scope_near_gate(&state, 0.1, false));
        state.route_edge_index = route.edges.len() as u32 - 1;
        state.progress_mm = 1;
        assert!(!view.scope_near_gate(&state, 0.1, false));
    }

    #[test]
    fn scope_membership_remains_a_participant() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        let handle = world.state.committed.live_order[0];
        let mut observed = false;
        for _ in 0..100 {
            world.step(TickInput::new(100)).unwrap();
            let mut state = *world.state.vehicle_state(handle).unwrap();
            if state.waiting_membership.is_some() {
                state.speed_mm_s = 0;
                assert!(world.state.read_view().scope_near_gate(&state, 0.1, true));
                observed = true;
                break;
            }
        }
        assert!(observed, "fixture must enter waiting");
    }

    #[test]
    fn scope_new_generation_is_read_in_the_same_tick() {
        let mut world = crate::kernel::waiting::tests::multi_gate_world(1);
        let old = world.state.committed.live_order[0];
        let state = *world.state.vehicle_state(old).unwrap();
        world.despawn_vehicle(old).unwrap();
        let new = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    state.profile,
                    state.route,
                    state.route_edge_index,
                    state.progress_mm,
                    state.speed_mm_s,
                )
                .with_open_entrance(),
            )
            .unwrap();
        assert_ne!(old, new);
        let view = world.state.read_view();
        assert!(view.vehicle_state(old).is_none());
        assert!(view.scope_near_gate(view.vehicle_state(new).unwrap(), 0.1, false));
        world.step(TickInput::new(100)).unwrap();
    }
}

pub(crate) static MODE: LazyLock<Scope> =
    LazyLock::new(
        || match std::env::var("LF757_SCOPE").as_deref().unwrap_or("all") {
            "all" => Scope {
                p2: false,
                p3: false,
            },
            "p3" => Scope {
                p2: false,
                p3: true,
            },
            "waiting" => Scope {
                p2: true,
                p3: false,
            },
            "both" => Scope { p2: true, p3: true },
            _ => panic!("invalid LF757_SCOPE"),
        },
    );

// 诊断只运行一个 worker；计数不共享原子，不进入普通 release。
thread_local! { static COUNTS: Cell<[u64; 9]> = const { Cell::new([0; 9]) }; }
#[inline]
pub(crate) fn count(index: usize, amount: usize) {
    #[cfg(feature = "scope-counts")]
    COUNTS.with(|cell| {
        let mut value = cell.get();
        value[index] += amount as u64;
        cell.set(value);
    });
    #[cfg(not(feature = "scope-counts"))]
    let _ = (index, amount);
}
pub(crate) fn take() -> [u64; 9] {
    COUNTS.with(|cell| cell.replace([0; 9]))
}

impl StepReadView<'_> {
    pub(crate) fn scope_near_gate(self, state: &VehicleState, delta_s: f32, waiting: bool) -> bool {
        if waiting && state.waiting_membership.is_some() {
            return true;
        }
        let Some(route) = self.compiled_route(state.route) else {
            return true;
        };
        let cursor = if state.progress_mm == 0 && state.carry_um == 0 {
            state.route_edge_index.saturating_sub(1)
        } else {
            state.route_edge_index
        };
        let hop = if waiting {
            route
                .waiting
                .get(route.waiting.partition_point(|o| o.entry_hop < cursor))
                .map(|o| o.entry_hop)
        } else {
            route
                .gate_hops
                .get(route.gate_hops.partition_point(|h| *h < cursor))
                .copied()
        };
        let Some(hop) = hop else { return false };
        if hop < state.route_edge_index {
            return true;
        }
        let Some(profile) = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
        else {
            return true;
        };
        let Some(reach) = MotionReach::from_tick(state.speed_mm_s, profile.max_accel(), delta_s)
        else {
            return true;
        };
        match super::tables::distance_to_occurrence_start(
            &route.occurrence_segments,
            &route.occurrence_offsets,
            &route.segment_totals,
            state.route_edge_index as usize,
            state.progress_mm,
            hop as usize + 1,
        ) {
            Some(BoundedDistance::Finite(mm)) => !reach.excludes(mm),
            // 不把超出有限距离表示的结果当作经过证明的不可达。
            Some(BoundedDistance::BeyondFinite) | None => true,
        }
    }
}
