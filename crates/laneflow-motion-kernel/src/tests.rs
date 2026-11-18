use super::*;

const N: usize = 129;

struct Fixture {
    enabled: [bool; N],
    speed: [u32; N],
    desired: [u32; N],
    progress: [u32; N],
    carry: [u16; N],
    room: [u32; N],
    stop_cap: [u32; N],
    cap_dominates: [bool; N],
    limit: [u32; N],
    leader: [f32; N],
    has_leader: [bool; N],
    min_gap: [f32; N],
    headway: [f32; N],
    accel: [f32; N],
    comfort: [f32; N],
    emergency: [f32; N],
    stop: [f32; N],
    end: [f32; N],
    envelope: [f32; N],
    has_proposal: [bool; N],
    proposal_speed: [f32; N],
    proposal_travel: [f32; N],
}

impl Fixture {
    fn new(seed: u64) -> Self {
        let mut f = Self {
            enabled: [true; N],
            speed: [0; N],
            desired: [0; N],
            progress: [0; N],
            carry: [0; N],
            room: [u32::MAX; N],
            stop_cap: [0; N],
            cap_dominates: [false; N],
            limit: [u32::MAX; N],
            leader: [f32::INFINITY; N],
            has_leader: [false; N],
            min_gap: [2.0; N],
            headway: [1.5; N],
            accel: [2.0; N],
            comfort: [3.0; N],
            emergency: [8.0; N],
            stop: [f32::INFINITY; N],
            end: [f32::INFINITY; N],
            envelope: [f32::INFINITY; N],
            has_proposal: [false; N],
            proposal_speed: [0.0; N],
            proposal_travel: [0.0; N],
        };
        let mut random = seed;
        for row in 0..N {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            f.speed[row] = (random >> 32) as u32;
            f.desired[row] = if row % 17 == 0 {
                0
            } else {
                ((random >> 7) as u32).max(1)
            };
            f.progress[row] = (random as u32) % 1_000_000;
            f.carry[row] = (random % 1_000) as u16;
            f.enabled[row] = row % 11 != 0;
            f.has_leader[row] = row % 3 != 0;
            f.leader[row] = match row % 5 {
                0 => -1.0,
                1 => 0.0,
                2 => 0.001,
                _ => (random % 1_000_000) as f32 / 1_000.0,
            };
            f.stop[row] = match row % 7 {
                0 => 0.0,
                1 => 0.001,
                _ => f32::INFINITY,
            };
            f.room[row] = match row % 9 {
                0 => 0,
                1 => 1,
                _ => u32::MAX,
            };
            if row % 2 == 0 {
                f.stop_cap[row] = 9_550;
            }
            // F3'：耗尽上界主导通道（过渡舒适衰减）随机穿插，验证三后端
            // 与标量在同一主导语义下逐位一致。
            f.cap_dominates[row] = row % 6 == 0;
            f.end[row] = match row % 13 {
                0 => 1.0,
                _ => f32::INFINITY,
            };
            f.envelope[row] = (random % 9_000_000) as f32 / 1_000.0;
            f.has_proposal[row] = row % 4 == 0;
            f.proposal_speed[row] = (random % 50_000) as f32 / 1_000.0;
            f.proposal_travel[row] = (random % 10_000) as f32 / 1_000_000.0;
        }
        f
    }

    fn input(&self, n: usize) -> Input<'_> {
        Input {
            enabled: &self.enabled[..n],
            speed_mm_s: &self.speed[..n],
            desired_mm_s: &self.desired[..n],
            progress_mm: &self.progress[..n],
            carry_um: &self.carry[..n],
            hard_room_mm: &self.room[..n],
            exhausted_speed_cap_mm_s: &self.stop_cap[..n],
            exhausted_cap_dominates: &self.cap_dominates[..n],
            committed_limit_mm_s: &self.limit[..n],
            leader_m: &self.leader[..n],
            has_leader: &self.has_leader[..n],
            min_gap_m: &self.min_gap[..n],
            time_headway: &self.headway[..n],
            max_accel: &self.accel[..n],
            comfort_decel: &self.comfort[..n],
            emergency_decel: &self.emergency[..n],
            stop_m: &self.stop[..n],
            route_end_m: &self.end[..n],
            envelope_m: &self.envelope[..n],
            has_proposal: &self.has_proposal[..n],
            proposal_speed_m_s: &self.proposal_speed[..n],
            proposal_travel_m: &self.proposal_travel[..n],
        }
    }
}

#[derive(Debug, PartialEq)]
struct Results {
    speed: [u32; N],
    progress: [u32; N],
    carry: [u16; N],
    travel: [u32; N],
    meters: [f32; N],
    proposal_speed: [f32; N],
    proposal_travel: [f32; N],
    exhausted: [bool; N],
    valid: [bool; N],
}
impl Results {
    fn new() -> Self {
        Self {
            speed: [42; N],
            progress: [42; N],
            carry: [42; N],
            travel: [42; N],
            meters: [42.0; N],
            proposal_speed: [42.0; N],
            proposal_travel: [42.0; N],
            exhausted: [false; N],
            valid: [false; N],
        }
    }
    fn output(&mut self, n: usize) -> Output<'_> {
        Output {
            speed_mm_s: &mut self.speed[..n],
            progress_mm: &mut self.progress[..n],
            carry_um: &mut self.carry[..n],
            travel_mm: &mut self.travel[..n],
            travel_m: &mut self.meters[..n],
            proposal_speed_m_s: &mut self.proposal_speed[..n],
            proposal_travel_m: &mut self.proposal_travel[..n],
            exhausted: &mut self.exhausted[..n],
            valid: &mut self.valid[..n],
            window_m: None,
        }
    }

    fn direct_output(&mut self, n: usize) -> DirectOutput<'_> {
        DirectOutput {
            speed_mm_s: &mut self.speed[..n],
            progress_mm: &mut self.progress[..n],
            carry_um: &mut self.carry[..n],
            travel_mm: &mut self.travel[..n],
            exhausted: &mut self.exhausted[..n],
            valid: &mut self.valid[..n],
        }
    }

    fn assert_motion_eq(&self, reference: &Self) {
        assert_eq!(self.speed, reference.speed);
        assert_eq!(self.progress, reference.progress);
        assert_eq!(self.carry, reference.carry);
        assert_eq!(self.travel, reference.travel);
        assert_eq!(self.exhausted, reference.exhausted);
        assert_eq!(self.valid, reference.valid);
        assert_eq!(self.meters, [42.0; N], "Direct has no float consumer");
        assert_eq!(self.proposal_speed, [42.0; N]);
        assert_eq!(self.proposal_travel, [42.0; N]);
    }
}

fn compare(fixture: &Fixture, n: usize, delta_s: f32) {
    let input = fixture.input(n);
    let mut reference = Results::new();
    let reference_stats = Kernel::for_backend(Backend::Scalar)
        .unwrap()
        .run(&input, &mut reference.output(n), delta_s)
        .unwrap();
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut plain = Results::new();
        kernel
            .run_direct(&input, &mut plain.direct_output(n), delta_s)
            .unwrap();
        plain.assert_motion_eq(&reference);
        let mut tracked = Results::new();
        let stats = kernel
            .run_direct_with_stats(&input, &mut tracked.direct_output(n), delta_s)
            .unwrap();
        tracked.assert_motion_eq(&reference);
        assert_eq!(plain, tracked);
        assert_eq!(
            stats.active_lanes,
            input.enabled.iter().filter(|&&v| v).count()
        );
        assert_eq!(stats.vector_lanes + stats.scalar_tail_lanes, n);
        assert_proposal_counts(
            stats,
            reference_stats.proposal_lanes_computed,
            reference_stats.proposal_lanes_reused,
        );
    }
    for backend in [Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut actual = Results::new();
        let stats = kernel.run(&input, &mut actual.output(n), delta_s).unwrap();
        assert_eq!(stats.vector_lanes + stats.scalar_tail_lanes, n);
        assert_proposal_counts(
            stats,
            reference_stats.proposal_lanes_computed,
            reference_stats.proposal_lanes_reused,
        );
        assert_eq!(
            actual, reference,
            "backend {backend:?}, n {n}, dt {delta_s}"
        );
    }
}

fn assert_proposal_counts(stats: Stats, computed: usize, reused: usize) {
    assert_eq!(stats.proposal_lanes_computed, computed);
    assert_eq!(stats.proposal_lanes_reused, reused);
    assert_eq!(
        stats.proposal_lanes_computed + stats.proposal_lanes_reused,
        stats.active_lanes,
    );
}

#[test]
fn exhausted_cap_composes_with_candidate_floor_and_only_applies_at_the_boundary() {
    // 同一批含 floor 抬升、上界更紧、停稳分支、较低限速及未耗尽行；17 行覆盖向量尾。
    // #1272：候选形成把低于紧急下界 F 的提案抬到 F；耗尽拍 committed = min(归约速度, 上界)。
    // F3' 复审修：dominant 通道（过渡舒适衰减）耗尽拍 = min(cap, limit)——
    // cap 取代投影与紧急下界（r_p5 钉版 1000 → 550），且只在耗尽拍生效。
    // (speed_mm_s, proposal_m_s, cap, limit, room, dominant, expected)
    let cases = [
        (10_000, 10.0, u32::MAX, u32::MAX, 0, false, 10_000),
        (10_000, 9.0, u32::MAX, u32::MAX, 0, false, 9_600),
        (10_000, 10.0, 9_550, u32::MAX, 0, false, 9_550),
        (400, 0.0, u32::MAX, u32::MAX, 100, false, 0),
        (10_000, 10.0, 9_550, 6_000, 0, false, 6_000),
        (10_000, 10.0, u32::MAX, u32::MAX, u32::MAX, false, 10_000),
        (1_000, 1.0, 550, u32::MAX, 0, true, 550),
        (1_000, 1.0, 550, 400, 0, true, 400),
        (1_000, 10.0, 550, u32::MAX, u32::MAX, true, 10_000),
    ];
    let mut f = Fixture::new(1_199);
    for row in 0..17 {
        let (speed, proposal, cap, limit, room, dominant, _) = cases[row % cases.len()];
        f.enabled[row] = true;
        f.has_proposal[row] = true;
        f.has_leader[row] = false;
        f.speed[row] = speed;
        f.comfort[row] = 2.0;
        f.emergency[row] = 4.0;
        f.stop[row] = f32::INFINITY;
        f.end[row] = f32::INFINITY;
        f.envelope[row] = f32::INFINITY;
        f.proposal_travel[row] = 1.0;
        f.proposal_speed[row] = proposal;
        f.room[row] = room;
        f.stop_cap[row] = cap;
        f.cap_dominates[row] = dominant;
        f.limit[row] = limit;
    }
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut result = Results::new();
        kernel
            .run_direct(&f.input(17), &mut result.direct_output(17), 0.1)
            .unwrap();
        for row in 0..17 {
            assert!(result.valid[row]);
            assert_eq!(
                result.speed[row],
                cases[row % cases.len()].6,
                "{backend:?}, row {row}"
            );
        }
    }
}

#[test]
fn mixed_cached_proposals_are_counted_once_per_enabled_lane() {
    let mut fixture = Fixture::new(814);
    fixture.enabled.fill(true);
    fixture.speed.fill(5_000);
    fixture.desired.fill(15_000);
    fixture.has_leader.fill(false);
    fixture.has_proposal = std::array::from_fn(|row| row % 2 == 0);
    fixture.proposal_speed.fill(6.0);
    fixture.proposal_travel.fill(0.1815);
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut plain = Results::new();
        kernel
            .run_direct(&fixture.input(16), &mut plain.direct_output(16), 0.033)
            .unwrap();
        let mut tracked = Results::new();
        let stats = kernel
            .run_direct_with_stats(&fixture.input(16), &mut tracked.direct_output(16), 0.033)
            .unwrap();
        assert_eq!(plain, tracked);
        assert_proposal_counts(stats, 8, 8);
    }
}

#[test]
fn proposal_source_counts_match_across_masks_phases_and_tails() {
    for n in [0, 1, 7, 8, 9, 15, 16, 17, 31, 32, 33, 127, 128, 129] {
        for mask in 0..6 {
            let mut fixture = Fixture::new(814);
            for row in 0..N {
                let (enabled, cached) = match mask {
                    0 => (true, false),
                    1 => (true, true),
                    2 => (true, row % 2 == 0),
                    3 => (row % 5 != 0, row % 2 == 0),
                    4 => (false, row % 2 == 0),
                    5 => (row % 2 == 0, row % 2 == 0),
                    _ => unreachable!(),
                };
                fixture.enabled[row] = enabled;
                fixture.has_proposal[row] = cached;
            }
            let input = fixture.input(n);
            let active = input.enabled.iter().filter(|&&enabled| enabled).count();
            let cached = input
                .enabled
                .iter()
                .zip(input.has_proposal)
                .filter(|&(&enabled, &cached)| enabled && cached)
                .count();
            for phase in [
                Phase::Direct,
                Phase::Full,
                Phase::Proposal,
                Phase::Project,
                Phase::FloatProject,
                Phase::Quantize,
            ] {
                let run = |kernel: Kernel| {
                    let mut result = Results::new();
                    let stats = match phase {
                        Phase::Direct => kernel.run_direct_with_stats(
                            &input,
                            &mut result.direct_output(n),
                            0.033,
                        ),
                        Phase::Full => kernel.run(&input, &mut result.output(n), 0.033),
                        Phase::Proposal => kernel.proposal(&input, &mut result.output(n), 0.033),
                        Phase::Project => kernel.project(&input, &mut result.output(n), 0.033),
                        Phase::FloatProject => {
                            kernel.float_projection(&input, &mut result.output(n), 0.033)
                        }
                        Phase::Quantize => kernel.quantize(&input, &mut result.output(n), 0.033),
                    }
                    .unwrap();
                    (stats, result)
                };
                let (_, reference) = run(Kernel::for_backend(Backend::Scalar).unwrap());
                for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
                    let Some(kernel) = Kernel::for_backend(backend) else {
                        continue;
                    };
                    let (stats, result) = run(kernel);
                    assert_eq!(result, reference, "backend {backend:?}, n {n}, mask {mask}");
                    assert_eq!(stats.active_lanes, active);
                    if phase == Phase::Quantize {
                        assert_eq!(stats.proposal_lanes_computed, 0);
                        assert_eq!(stats.proposal_lanes_reused, 0);
                    } else if matches!(phase, Phase::Project | Phase::FloatProject) {
                        assert_proposal_counts(stats, 0, active);
                    } else {
                        assert_proposal_counts(stats, active - cached, cached);
                    }
                    if phase == Phase::Direct {
                        let mut plain = Results::new();
                        kernel
                            .run_direct(&input, &mut plain.direct_output(n), 0.033)
                            .unwrap();
                        assert_eq!(plain, result);
                    }
                }
            }
        }
    }
}

#[test]
fn direct_output_rejects_short_columns_before_any_write() {
    let fixture = Fixture::new(17);
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut result = Results::new();
        let before = Results::new();
        let mut output = result.direct_output(N);
        output.valid = &mut output.valid[..N - 1];
        assert_eq!(
            kernel.run_direct(&fixture.input(N), &mut output, 0.033),
            Err(LengthMismatch)
        );
        assert_eq!(result, before);
        let mut input = fixture.input(N);
        input.exhausted_speed_cap_mm_s = &fixture.stop_cap[..N - 1];
        assert_eq!(
            kernel.run_direct(&input, &mut result.direct_output(N), 0.033),
            Err(LengthMismatch)
        );
        assert_eq!(result, before);
    }
}

#[test]
fn direct_skips_closed_groups_and_reuses_only_enabled_proposals() {
    let mut fixture = Fixture::new(18);
    fixture.enabled.fill(false);
    fixture.enabled[17] = true;
    fixture.has_proposal.fill(false);
    fixture.has_proposal[17] = true;
    fixture.accel[17] = 2.0;
    fixture.proposal_speed[17] = 3.0;
    fixture.proposal_travel[17] = 0.015;
    for backend in [Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut result = Results::new();
        result.valid.fill(true);
        let stats = kernel
            .run_direct_with_stats(&fixture.input(N), &mut result.direct_output(N), 0.033)
            .unwrap();
        assert_eq!(stats.proposal_lanes_computed, 0);
        assert_eq!(stats.proposal_lanes_reused, 1);
        assert_eq!(result.valid, std::array::from_fn(|row| row == 17));
    }
    compare(&fixture, N, 0.033);
}

#[test]
fn complete_columns_match_for_unsigned_range_sparse_lanes_and_every_tail() {
    println!("process backend: {:?}", Kernel::detect().backend());
    for seed in 1..=64 {
        let fixture = Fixture::new(seed);
        for n in [0, 1, 7, 8, 9, 15, 16, 17, 63, 64, 127, 128, 129] {
            for delta_s in [0.016, 0.033, 0.1] {
                compare(&fixture, n, delta_s);
            }
        }
    }
}

#[test]
fn proposal_reprojection_ties_carry_and_zero_room_match() {
    let mut fixture = Fixture::new(99);
    fixture.has_proposal.fill(true);
    fixture.has_leader.fill(false);
    fixture.envelope.fill(f32::INFINITY);
    fixture.proposal_speed.fill(12.345_5);
    for row in 0..N {
        fixture.proposal_travel[row] = (row as f32 + 0.5) / 1_000_000.0;
        fixture.carry[row] = 999;
    }
    compare(&fixture, N, 0.016);
}

#[test]
fn repeated_masked_edge_steps_match_for_boundaries_carry_blocked_hops_and_unsigned_range() {
    for n in [0, 1, 7, 8, 9, 15, 16, 17, 127, 128, 129] {
        for seed in 0..64_u32 {
            let lengths: [u32; N] = std::array::from_fn(|row| ((row as u32 + seed) * 731) % 10_000);
            let can: [bool; N] = std::array::from_fn(|row| !(row as u32 + seed).is_multiple_of(5));
            let has_next: [bool; N] =
                std::array::from_fn(|row| !(row as u32 + seed).is_multiple_of(7));
            let carry: [u16; N] = std::array::from_fn(|row| if row % 3 == 0 { 999 } else { 0 });
            let initial_cursor: [u32; N] =
                std::array::from_fn(|row| if row % 17 == 0 { u32::MAX } else { row as u32 });
            let initial_progress: [u32; N] =
                std::array::from_fn(|row| lengths[row].saturating_sub((row % 4) as u32));
            let initial_remaining: [u32; N] = std::array::from_fn(|row| {
                if row % 19 == 0 {
                    u32::MAX
                } else {
                    (row as u32 + seed) % 12_345
                }
            });
            let run = |backend| {
                let mut cursor = initial_cursor;
                let mut progress = initial_progress;
                let mut remaining = initial_remaining;
                let mut active: [bool; N] = std::array::from_fn(|row| row % 11 != 0);
                let mut valid = [true; N];
                let kernel = Kernel::for_backend(backend).unwrap();
                for _ in 0..8 {
                    kernel
                        .advance(
                            &EdgeInput {
                                edge_length_mm: &lengths[..n],
                                can_hop: &can[..n],
                                has_next: &has_next[..n],
                                carry_um: &carry[..n],
                            },
                            &mut EdgeOutput {
                                route_cursor: &mut cursor[..n],
                                progress_mm: &mut progress[..n],
                                remaining_mm: &mut remaining[..n],
                                active: &mut active[..n],
                                valid: &mut valid[..n],
                            },
                        )
                        .unwrap();
                }
                (cursor, progress, remaining, active, valid)
            };
            let expected = run(Backend::Scalar);
            for backend in [Backend::Avx2, Backend::Avx512] {
                if Kernel::for_backend(backend).is_some() {
                    assert_eq!(
                        run(backend),
                        expected,
                        "backend={backend:?}, n={n}, seed={seed}"
                    );
                }
            }
        }
    }
}

#[test]
fn speed_limit_solver_and_boundary_clamp_match_for_all_tail_shapes() {
    for n in [0, 1, 7, 8, 9, 15, 16, 17, 63, 127, 128, 129] {
        for seed in 1..=64 {
            let fixture = Fixture::new(seed);
            let distance: [u32; N] = std::array::from_fn(|row| {
                if row % 11 == 0 {
                    0
                } else {
                    (row as u32 * 2_053) % 750_000
                }
            });
            let limits: [u32; N] = std::array::from_fn(|row| (row as u32 * 731) % 100_000);
            let next: [f32; N] =
                std::array::from_fn(|row| fixture.speed[row] as f32 / 1_000.0 + 0.016);
            for delta in [0.001, 0.016, 0.033, 1.0] {
                let run = |backend| {
                    let kernel = Kernel::for_backend(backend).unwrap();
                    let mut next = next;
                    let mut travel: [f32; N] = std::array::from_fn(|row| {
                        (fixture.speed[row] as f32 / 1_000.0 + next[row]) * 0.5 * delta
                    });
                    let mut active = fixture.enabled;
                    kernel
                        .limit(
                            &LimitInput {
                                window_m: &[f32::INFINITY; N][..n],
                                speed_mm_s: &fixture.speed[..n],
                                distance_mm: &distance[..n],
                                limit_mm_s: &limits[..n],
                                comfort_decel: &fixture.comfort[..n],
                                emergency_decel: &fixture.emergency[..n],
                            },
                            &mut LimitOutput {
                                next_speed_m_s: &mut next[..n],
                                active: &mut active[..n],
                            },
                            delta,
                        )
                        .unwrap();
                    kernel
                        .boundary(
                            &BoundaryInput {
                                speed_mm_s: &fixture.speed[..n],
                                distance_mm: &distance[..n],
                                limit_mm_s: &limits[..n],
                                next_speed_m_s: &next[..n],
                            },
                            &mut BoundaryOutput {
                                travel_m: &mut travel[..n],
                                active: &mut active[..n],
                            },
                            delta,
                        )
                        .unwrap();
                    (next.map(f32::to_bits), travel.map(f32::to_bits), active)
                };
                let expected = run(Backend::Scalar);
                for backend in [Backend::Avx2, Backend::Avx512] {
                    if Kernel::for_backend(backend).is_some() {
                        assert_eq!(
                            run(backend),
                            expected,
                            "backend={backend:?}, n={n}, seed={seed}, delta={delta}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn proposal_projection_quantization_pipeline_matches_complete_kernel() {
    let mut fixture = Fixture::new(818);
    fixture.has_proposal.fill(false);
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let input = fixture.input(N);
        let mut expected = Results::new();
        kernel.run(&input, &mut expected.output(N), 0.016).unwrap();
        let mut raw = Results::new();
        kernel.proposal(&input, &mut raw.output(N), 0.016).unwrap();
        let mut projected_input = Fixture::new(818);
        projected_input.has_proposal.fill(true);
        projected_input.proposal_speed = raw.proposal_speed;
        projected_input.proposal_travel = raw.proposal_travel;
        let input = projected_input.input(N);
        let mut actual = Results::new();
        kernel
            .float_projection(&input, &mut actual.output(N), 0.016)
            .unwrap();
        kernel
            .quantize(&input, &mut actual.output(N), 0.016)
            .unwrap();
        assert_eq!(actual, expected, "backend={backend:?}");
    }
}

#[test]
fn length_failure_keeps_every_output_unchanged() {
    let fixture = Fixture::new(7);
    let mut input = fixture.input(N);
    input.proposal_travel_m = &fixture.proposal_travel[..N - 1];
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut result = Results::new();
        assert_eq!(
            kernel.run(&input, &mut result.output(N), 0.016),
            Err(LengthMismatch)
        );
        assert_eq!(result, Results::new());
    }
}

#[test]
fn nonfinite_proposals_and_quantization_overflow_are_rejected() {
    let mut fixture = Fixture::new(8);
    fixture.enabled.fill(true);
    fixture.has_proposal.fill(true);
    fixture.has_leader.fill(false);
    fixture.stop.fill(f32::INFINITY);
    fixture.end.fill(f32::INFINITY);
    fixture.envelope.fill(f32::INFINITY);
    fixture.room.fill(u32::MAX);
    for row in 0..N {
        // #1272 起提案速度是唯一被消费的提案列（位移按一致三元组重算），
        // 非有限拒绝锚定在速度列上。
        fixture.proposal_travel[row] = 1.0;
        fixture.proposal_speed[row] = match row % 3 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            _ => f32::MAX,
        };
    }
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        let mut result = Results::new();
        kernel
            .run(&fixture.input(N), &mut result.output(N), 0.016)
            .unwrap();
        assert!(result.valid.iter().all(|&x| !x));
        let mut direct = Results::new();
        direct.valid.fill(true);
        kernel
            .run_direct(&fixture.input(N), &mut direct.direct_output(N), 0.016)
            .unwrap();
        direct.assert_motion_eq(&result);
    }
}

#[test]
fn direct_preserves_invalid_delta_and_existing_parameter_comparisons() {
    let mut fixture = Fixture::new(19);
    fixture.enabled.fill(true);
    fixture.has_proposal.fill(true);
    fixture.proposal_travel.fill(0.01);
    fixture.proposal_speed.fill(10.0);
    fixture.has_leader.fill(false);
    for row in 0..N {
        match row % 4 {
            0 => fixture.accel[row] = f32::NAN,
            1 => fixture.comfort[row] = f32::NAN,
            2 => fixture.emergency[row] = f32::NAN,
            _ => fixture.accel[row] = 0.0,
        }
    }
    for backend in [Backend::Scalar, Backend::Avx2, Backend::Avx512] {
        let Some(kernel) = Kernel::for_backend(backend) else {
            continue;
        };
        for delta in [0.033, 0.0, -0.033, f32::NAN, f32::INFINITY] {
            let mut reference = Results::new();
            kernel
                .run(&fixture.input(N), &mut reference.output(N), delta)
                .unwrap();
            let mut direct = Results::new();
            kernel
                .run_direct(&fixture.input(N), &mut direct.direct_output(N), delta)
                .unwrap();
            direct.assert_motion_eq(&reference);
        }
    }
}

/// #1272 求解器可行域回归锚：本拍减速上界、拍内停稳实际减速度与安全求根。
#[test]
fn stop_constraint_feasibility_domain_rejects_super_decel_tier_and_root_anchors() {
    // 15 m / 100 ms：舒适档域空（旧实现返回 u≈7.516，一拍 24.84 m/s² 伪舒适），
    // 紧急档可行且本拍无需减速。
    assert!(max_next_speed_for_decel(10.0, 10.0, 0.1, 15.0, 0.0, 2.0).is_none());
    let emergency = max_next_speed_for_decel(10.0, 10.0, 0.1, 15.0, 0.0, 4.0);
    assert_eq!(emergency, Some(10.0));

    // 25 m / 16 ms 舒适边界：任何结果都不得低于该档一拍下界（旧实现因 1.9 µm
    // 代回超界弃根回退 linear=0，产生 625 m/s²）。
    for distance in [24.8_f32, 24.9, 25.0, 25.0001, 25.1, 25.2] {
        if let Some(u) = max_next_speed_for_decel(10.0, 10.0, 0.016, distance, 0.0, 2.0) {
            assert!(u >= 10.0 - 2.0 * 0.016, "u={u} at d={distance}");
        }
    }

    // 拍内停稳实际减速度定义域：真实不可停（紧急停车距离 0.125 m > 0.01 m）
    // 不得被终速判据误判可停；两档皆空进入具名投影。
    assert!(max_next_speed_for_decel(1.0, 1.0, 1.0, 0.01, 0.0, 2.0).is_none());
    assert!(max_next_speed_for_decel(1.0, 1.0, 1.0, 0.01, 0.0, 4.0).is_none());

    // 停稳分支用档位减速度的实际停车位移 v²/(2b)：0.26 m 内 1 m/s 可舒适停稳。
    assert_eq!(
        max_next_speed_for_decel(1.0, 1.0, 1.0, 0.26, 0.0, 2.0),
        Some(0.0)
    );
    assert!(speed_down_constraint_holds(1.0, 0.0, 1.0, 0.26, 0.0, 2.0));
    assert!(!speed_down_constraint_holds(1.0, 0.0, 1.0, 0.24, 0.0, 2.0));

    // 候选低于该档一拍下界时域为空，不抬高候选（级联交由调用方）。
    assert!(max_next_speed_for_decel(10.0, 9.0, 0.1, 100.0, 0.0, 4.0).is_none());
    assert_eq!(
        max_next_speed_for_decel(10.0, 9.7, 0.1, 100.0, 0.0, 4.0),
        Some(9.7)
    );
}

/// F2 回归锚：正根因 f32 舍入微小越界被弃根退到零，导致停在目标前 8 mm
/// 永久不动（current=0、next=0.2、dt=0.1、d=0.008、limit=0、b=2 时根
/// ≈0.1049，代回越界 ~4e-7）。修复后取微降根，三后端一致。
#[test]
fn root_rounding_regression_keeps_feasible_positive_root() {
    let result = max_next_speed_for_decel(0.0, 0.2, 0.1, 0.008, 0.0, 2.0);
    let Some(u) = result else {
        panic!("feasible positive root must not be discarded");
    };
    assert!(
        u > 0.09,
        "shrunk root {u} must stay near the mathematical root 0.1049"
    );
    assert!(speed_down_constraint_holds(0.0, u, 0.1, 0.008, 0.0, 2.0));
}
