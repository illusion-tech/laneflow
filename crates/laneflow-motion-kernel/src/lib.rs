//! 纯数值列内核。CPU/OS 检测和切片范围证明限定在本 crate（ADR 0031）。
//! 不拥有车辆、路线或资源；无近似倒数、FMA 或 fast-math。

mod hold;
mod projection;
#[cfg(target_arch = "x86_64")]
mod x86;
pub use hold::{
    HOLD_TARGETS, HoldCommit, HoldKind, HoldState, NO_HOLD_TARGET, delta_ms, hold_stop_feasibility,
    stop_distance_um,
};
pub use projection::{
    BoundaryInput, BoundaryOutput, LimitInput, LimitOutput, max_next_speed_for_decel,
    speed_down_constraint_holds,
};

/// 标量预览/摆放与向量后端共享同一纯数值定义；不包含停止点或资源状态。
pub struct ProposalInput {
    pub speed_m_s: f32,
    pub desired_m_s: f32,
    pub leader_m: Option<f32>,
    /// 前车已提交速度（m/s），只用于 IDM 期望间距的接近速度项；无前车时不读。
    pub leader_speed_m_s: f32,
    pub min_gap_m: f32,
    pub time_headway: f32,
    pub max_accel: f32,
    pub comfort_decel: f32,
    pub emergency_decel: f32,
    pub delta_s: f32,
}

/// 返回尚未量化、尚未叠加停止约束的 `(travel_m, speed_m_s)`；非法数值返回 None。
pub fn raw_proposal(input: &ProposalInput) -> Option<(f32, f32)> {
    let speed = input.speed_m_s;
    let desired = input.desired_m_s;
    let delta_s = input.delta_s;
    if !speed.is_finite() || !desired.is_finite() || delta_s <= 0.0 {
        return None;
    }
    if input.max_accel <= 0.0 || input.comfort_decel <= 0.0 || input.emergency_decel <= 0.0 {
        return None;
    }
    if input.leader_m.is_some_and(|gap| gap <= 0.0) {
        return Some((0.0, 0.0));
    }
    let squared = (speed / desired).max(0.0).powi(2);
    let speed_term = if desired <= 0.0 {
        1.0
    } else {
        squared * squared
    };
    let gap_term = input.leader_m.map_or(0.0, |gap| {
        let s_star = desired_gap(
            input.min_gap_m,
            speed,
            input.time_headway,
            input.leader_speed_m_s,
            input.max_accel,
            input.comfort_decel,
        );
        (s_star / gap).max(0.0).powi(2)
    });
    // vehicle-following §9：舒适加速度夹到 `[-b, a]`（上界由 `1 - 项 ≤ 1` 自然成立）。
    let accel = (input.max_accel * (1.0 - speed_term - gap_term)).max(-input.comfort_decel);
    let next_speed = (speed + accel * delta_s).max(0.0).min(desired.max(0.0));
    let travel = ((speed + next_speed) * 0.5 * delta_s).max(0.0);
    (travel.is_finite() && next_speed.is_finite()).then_some((travel, next_speed))
}

/// vehicle-following §9 的 IDM 期望动态间距
/// `s* = s0 + max(0, v·T + v·Δv/(2√(ab)))`；向量后端逐式同序。
fn desired_gap(
    min_gap: f32,
    speed: f32,
    headway: f32,
    leader_speed: f32,
    max_accel: f32,
    comfort: f32,
) -> f32 {
    let approach = speed * (speed - leader_speed) / (2.0 * (max_accel * comfort).sqrt());
    min_gap + (speed * headway + approach).max(0.0)
}

/// 砖墙安全速度上界的安全方向收缩：抵消 f32 求根舍入，使上界不高于数学正根。
pub const SAFE_SPEED_SHRINK: f32 = 1.0 - 1.0 / 1_048_576.0;

/// vehicle-following §10.2 砖墙包络：`0.5(v+u)Δt + u²/(2b) ≤ s_a` 的最大非负 `u`，
/// 稳定求根形式；`C ≥ 0` 时为零。`room` 非有限（无前车）时不施加上界。
pub fn safe_speed(speed: f32, emergency: f32, delta_s: f32, room: f32) -> f32 {
    if !room.is_finite() {
        return f32::INFINITY;
    }
    let b_dt = emergency * delta_s;
    let rhs = 2.0 * emergency * room;
    let c = emergency * speed * delta_s - rhs;
    if c.is_nan() || c >= 0.0 {
        return 0.0;
    }
    let discriminant = b_dt * b_dt - 4.0 * c;
    ((-2.0 * c) / (b_dt + discriminant.sqrt())) * SAFE_SPEED_SHRINK
}

/// 受支持的数值执行后端；选择不进入交通身份或快照。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Scalar,
    Avx2,
    Avx512,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Full,
    Direct,
    Proposal,
    Project,
    FloatProject,
    Quantize,
}

/// 只在构造时选择 ISA，热循环按整块分发。
#[derive(Clone, Copy, Debug)]
pub struct Kernel {
    backend: Backend,
    motion: MotionEntry,
    motion_untracked: MotionEntry,
    edge: EdgeEntry,
    limit: LimitEntry,
    boundary: BoundaryEntry,
}

type MotionEntry = unsafe fn(&Input<'_>, &mut Columns<'_>, f32, Phase, &mut Stats) -> usize;
type EdgeEntry = unsafe fn(&EdgeInput<'_>, &mut EdgeOutput<'_>, &mut EdgeStats) -> usize;
type LimitEntry = unsafe fn(&LimitInput<'_>, &mut LimitOutput<'_>, f32) -> usize;
type BoundaryEntry = unsafe fn(&BoundaryInput<'_>, &mut BoundaryOutput<'_>, f32) -> usize;
fn scalar_motion(_: &Input<'_>, _: &mut Columns<'_>, _: f32, _: Phase, _: &mut Stats) -> usize {
    0
}
fn scalar_edge(_: &EdgeInput<'_>, _: &mut EdgeOutput<'_>, _: &mut EdgeStats) -> usize {
    0
}
fn scalar_limit(_: &LimitInput<'_>, _: &mut LimitOutput<'_>, _: f32) -> usize {
    0
}
fn scalar_boundary(_: &BoundaryInput<'_>, _: &mut BoundaryOutput<'_>, _: f32) -> usize {
    0
}

/// 所有字段均属于同一连续物理行范围。复杂路线的降速约束由精确控制路径处理。
pub struct Input<'a> {
    pub enabled: &'a [bool],
    pub speed_mm_s: &'a [u32],
    pub desired_mm_s: &'a [u32],
    pub progress_mm: &'a [u32],
    pub carry_um: &'a [u16],
    pub hard_room_mm: &'a [u32],
    /// 仅在硬空间耗尽时叠加到投影速度的上限；零表示停稳。
    pub exhausted_speed_cap_mm_s: &'a [u32],
    /// 耗尽拍速度由耗尽上界直接主导（过渡预约的舒适衰减），不与候选投影
    /// 取 min。只在完成阶段消费，不参与候选形成（#1272 F3' 复审修）。
    pub exhausted_cap_dominates: &'a [bool],
    pub committed_limit_mm_s: &'a [u32],
    pub leader_m: &'a [f32],
    pub has_leader: &'a [bool],
    /// 前车已提交速度（m/s）；无前车行任意有限值。
    pub leader_speed_m_s: &'a [f32],
    /// vehicle-following §10.2 整数停车界（米，已含安全方向收缩）；无前车为 `+∞`。
    pub safe_room_m: &'a [f32],
    /// 本拍开始时有前车且跟车可行：行程微米与已提交速度向零取整（整数几何 §5）。
    pub safe_quantize: &'a [bool],
    pub min_gap_m: &'a [f32],
    pub time_headway: &'a [f32],
    pub max_accel: &'a [f32],
    pub comfort_decel: &'a [f32],
    pub emergency_decel: &'a [f32],
    /// 三列上界只取有限值或 `+∞`，不得为 NaN：标量 `f32::min` 忽略单个 NaN，
    /// x86 min/max 按操作数位置传播 NaN，两类后端结果会分叉。Runtime 由整数毫米
    /// 与正时间步导出这三列，满足该约定。
    pub stop_m: &'a [f32],
    pub route_end_m: &'a [f32],
    /// 整数几何 §5.1 停车可行性保持的目标距离（mm，与 `stop_m` / `route_end_m`
    /// 同源的整数值）；[`NO_HOLD_TARGET`] 表示没有该类目标。
    pub stop_mm: &'a [u32],
    pub route_end_mm: &'a [u32],
    pub envelope_m: &'a [f32],
    pub has_proposal: &'a [bool],
    pub proposal_speed_m_s: &'a [f32],
    pub proposal_travel_m: &'a [f32],
}

/// 直接写入下一运动列所需的数值；路线行走和稀疏控制副作用由调用方消费。
pub struct Output<'a> {
    pub speed_mm_s: &'a mut [u32],
    pub progress_mm: &'a mut [u32],
    pub carry_um: &'a mut [u16],
    pub travel_mm: &'a mut [u32],
    pub travel_m: &'a mut [f32],
    pub proposal_speed_m_s: &'a mut [f32],
    pub proposal_travel_m: &'a mut [f32],
    pub exhausted: &'a mut [bool],
    pub valid: &'a mut [bool],
    /// 只在实际有降速消费者的提案屏障物化制动查询窗。
    pub window_m: Option<&'a mut [f32]>,
    /// 诊断：整数几何 §5.1 改写后的提交。只写入经标量完成原语且有改写的行，
    /// 其余行保持调用方预置值；不参与求值。
    pub hold: Option<&'a mut [Option<HoldCommit>]>,
}

/// 无后续浮点屏障的常规路径只发布这些列，不要求分配或写入中间提案列。
pub struct DirectOutput<'a> {
    pub speed_mm_s: &'a mut [u32],
    pub progress_mm: &'a mut [u32],
    pub carry_um: &'a mut [u16],
    pub travel_mm: &'a mut [u32],
    pub exhausted: &'a mut [bool],
    pub valid: &'a mut [bool],
    /// 诊断：同 [`Output::hold`]。
    pub hold: Option<&'a mut [Option<HoldCommit>]>,
}

struct Floats<'a> {
    travel_m: &'a mut [f32],
    proposal_speed_m_s: &'a mut [f32],
    proposal_travel_m: &'a mut [f32],
    window_m: Option<&'a mut [f32]>,
}

/// 后端只借用调用者实际要求的输出；Direct 没有浮点列消费者。
struct Columns<'a> {
    speed_mm_s: &'a mut [u32],
    progress_mm: &'a mut [u32],
    carry_um: &'a mut [u16],
    travel_mm: &'a mut [u32],
    exhausted: &'a mut [bool],
    valid: &'a mut [bool],
    floats: Option<Floats<'a>>,
    hold: Option<&'a mut [Option<HoldCommit>]>,
}

impl Output<'_> {
    fn columns(&mut self) -> Columns<'_> {
        Columns {
            speed_mm_s: self.speed_mm_s,
            progress_mm: self.progress_mm,
            carry_um: self.carry_um,
            travel_mm: self.travel_mm,
            exhausted: self.exhausted,
            valid: self.valid,
            floats: Some(Floats {
                travel_m: self.travel_m,
                proposal_speed_m_s: self.proposal_speed_m_s,
                proposal_travel_m: self.proposal_travel_m,
                window_m: self.window_m.as_deref_mut(),
            }),
            hold: self.hold.as_deref_mut(),
        }
    }
}

impl DirectOutput<'_> {
    fn columns(&mut self) -> Columns<'_> {
        Columns {
            speed_mm_s: self.speed_mm_s,
            progress_mm: self.progress_mm,
            carry_um: self.carry_um,
            travel_mm: self.travel_mm,
            exhausted: self.exhausted,
            valid: self.valid,
            floats: None,
            hold: self.hold.as_deref_mut(),
        }
    }
}

/// 路线行走的一轮静态查询结果；调用方只为仍在行走的通道读取当前实际 occurrence。
pub struct EdgeInput<'a> {
    pub edge_length_mm: &'a [u32],
    pub can_hop: &'a [bool],
    pub has_next: &'a [bool],
    pub carry_um: &'a [u16],
}

/// 原位推进真实 next 列；active 掩码允许任意多轮实际跨边，无一跳假设。
pub struct EdgeOutput<'a> {
    pub route_cursor: &'a mut [u32],
    pub progress_mm: &'a mut [u32],
    pub remaining_mm: &'a mut [u32],
    pub active: &'a mut [bool],
    pub valid: &'a mut [bool],
}

/// 路线行走实际执行的整数向量通道与 hop 数。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EdgeStats {
    pub active_lanes: usize,
    pub vector_lanes: usize,
    pub scalar_tail_lanes: usize,
    pub hops: usize,
}

/// 运动工作量：向量和尾部按物理通道计数，提案来源只统计启用行。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    pub active_lanes: usize,
    pub vector_lanes: usize,
    pub scalar_tail_lanes: usize,
    /// 本次使用新计算提案的启用行数，与复用行互斥；不是 SIMD 满宽算术次数。
    pub proposal_lanes_computed: usize,
    /// 本次使用既有提案的启用行数，包含 Project/FloatProject；Quantize 不计提案来源。
    pub proposal_lanes_reused: usize,
    pub integer_vector_lanes: usize,
}

impl Stats {
    fn record_proposal_source(&mut self, enabled: bool, reused: bool) {
        if !enabled {
            return;
        }
        if reused {
            self.proposal_lanes_reused += 1;
        } else {
            self.proposal_lanes_computed += 1;
        }
    }
}

/// 输入或输出列的长度不一致；任何输出写入前拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LengthMismatch;
impl std::fmt::Display for LengthMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("motion column lengths differ")
    }
}
impl std::error::Error for LengthMismatch {}

impl Kernel {
    /// CPU 与操作系统共同支持时选择 AVX-512，其次 AVX2，否则标量。
    #[must_use]
    pub fn detect() -> Self {
        Self::for_backend(Backend::Avx512)
            .or_else(|| Self::for_backend(Backend::Avx2))
            .unwrap_or_else(|| {
                Self::for_backend(Backend::Scalar).expect("scalar backend is portable")
            })
    }

    /// 强制受支持后端，用于同布局差异验证；不支持时返回 `None`。
    #[must_use]
    pub fn for_backend(backend: Backend) -> Option<Self> {
        let supported = match backend {
            Backend::Scalar => true,
            #[cfg(target_arch = "x86_64")]
            Backend::Avx2 => std::is_x86_feature_detected!("avx2"),
            #[cfg(target_arch = "x86_64")]
            Backend::Avx512 => {
                std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx2")
            }
            #[cfg(not(target_arch = "x86_64"))]
            Backend::Avx2 | Backend::Avx512 => false,
        };
        if !supported {
            return None;
        }
        let (motion, edge, limit, boundary): (MotionEntry, EdgeEntry, LimitEntry, BoundaryEntry) =
            match backend {
                Backend::Scalar => (scalar_motion, scalar_edge, scalar_limit, scalar_boundary),
                #[cfg(target_arch = "x86_64")]
                Backend::Avx2 => (
                    x86::avx2::<true>,
                    x86::advance2,
                    x86::limit2,
                    x86::boundary2,
                ),
                #[cfg(target_arch = "x86_64")]
                Backend::Avx512 => (
                    x86::avx512::<true>,
                    x86::advance512,
                    x86::limit512,
                    x86::boundary512,
                ),
                #[cfg(not(target_arch = "x86_64"))]
                Backend::Avx2 | Backend::Avx512 => {
                    unreachable!("unsupported backend rejected before table construction")
                }
            };
        Some(Self {
            backend,
            motion,
            motion_untracked: match backend {
                Backend::Scalar => scalar_motion,
                #[cfg(target_arch = "x86_64")]
                Backend::Avx2 => x86::avx2::<false>,
                #[cfg(target_arch = "x86_64")]
                Backend::Avx512 => x86::avx512::<false>,
                #[cfg(not(target_arch = "x86_64"))]
                Backend::Avx2 | Backend::Avx512 => unreachable!("unsupported backend"),
            },
            edge,
            limit,
            boundary,
        })
    }

    #[must_use]
    pub const fn backend(self) -> Backend {
        self.backend
    }

    /// 同布局求解、投影、量化、carry 和同边进度推进。
    ///
    /// # Errors
    /// 任一列长度不同则返回 `LengthMismatch`，全部输出保持原值。
    pub fn run(
        self,
        input: &Input<'_>,
        output: &mut Output<'_>,
        delta_s: f32,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate(input, output, delta_s, Phase::Full)
    }

    /// 提案、安全投影和量化在同一数值子批内完成，直接发布整数运动列。
    /// 普通入口使用编译期无统计后端；不生成逐行诊断计数。
    ///
    /// # Errors
    /// 任一列长度不同则返回 `LengthMismatch`，全部输出保持原值。
    pub fn run_direct(
        self,
        input: &Input<'_>,
        output: &mut DirectOutput<'_>,
        delta_s: f32,
    ) -> Result<(), LengthMismatch> {
        self.calculate_columns::<false>(input, &mut output.columns(), delta_s, Phase::Direct)
            .map(|_| ())
    }

    /// 与常规直接路径共用运算，诊断和差异测试才收集实际通道统计。
    ///
    /// # Errors
    /// 任一列长度不同则返回 `LengthMismatch`，全部输出保持原值。
    pub fn run_direct_with_stats(
        self,
        input: &Input<'_>,
        output: &mut DirectOutput<'_>,
        delta_s: f32,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate_columns::<true>(input, &mut output.columns(), delta_s, Phase::Direct)
    }

    /// 仅输出未量化 IIDM 提案，供实际降速 occurrence 工作集消费。
    ///
    /// # Errors
    /// 列长度不同时全部输出保持原值并返回 `LengthMismatch`。
    pub fn proposal(
        self,
        input: &Input<'_>,
        output: &mut Output<'_>,
        delta_s: f32,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate(input, output, delta_s, Phase::Proposal)
    }

    /// 投影调用方提供的提案；不再执行 IIDM，仍执行安全裁剪、量化及整数推进。
    ///
    /// # Errors
    /// 列长度不同时全部输出保持原值并返回 `LengthMismatch`。
    pub fn project(
        self,
        input: &Input<'_>,
        output: &mut Output<'_>,
        delta_s: f32,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate(input, output, delta_s, Phase::Project)
    }

    /// 制动约束之后重新积分并安全裁剪，量化留到实际边界约束之后。
    ///
    /// # Errors
    /// 列长度不同时全部输出保持原值并返回 `LengthMismatch`。
    pub fn float_projection(
        self,
        input: &Input<'_>,
        output: &mut Output<'_>,
        delta_s: f32,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate(input, output, delta_s, Phase::FloatProject)
    }

    /// 对已经过完整安全投影的输出旅行列执行 ties-even 量化、硬空间和 carry。
    ///
    /// # Errors
    /// 列长度不同时全部输出保持原值并返回 `LengthMismatch`。
    pub fn quantize(
        self,
        input: &Input<'_>,
        output: &mut Output<'_>,
        delta_s: f32,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate(input, output, delta_s, Phase::Quantize)
    }

    fn calculate(
        self,
        input: &Input<'_>,
        output: &mut Output<'_>,
        delta_s: f32,
        phase: Phase,
    ) -> Result<Stats, LengthMismatch> {
        self.calculate_columns::<true>(input, &mut output.columns(), delta_s, phase)
    }

    fn calculate_columns<const TRACK: bool>(
        self,
        input: &Input<'_>,
        output: &mut Columns<'_>,
        delta_s: f32,
        phase: Phase,
    ) -> Result<Stats, LengthMismatch> {
        let n = input.enabled.len();
        let lengths = [
            input.speed_mm_s.len(),
            input.desired_mm_s.len(),
            input.progress_mm.len(),
            input.carry_um.len(),
            input.hard_room_mm.len(),
            input.exhausted_speed_cap_mm_s.len(),
            input.exhausted_cap_dominates.len(),
            input.committed_limit_mm_s.len(),
            input.leader_m.len(),
            input.has_leader.len(),
            input.leader_speed_m_s.len(),
            input.safe_room_m.len(),
            input.safe_quantize.len(),
            input.min_gap_m.len(),
            input.time_headway.len(),
            input.max_accel.len(),
            input.comfort_decel.len(),
            input.emergency_decel.len(),
            input.stop_m.len(),
            input.route_end_m.len(),
            input.stop_mm.len(),
            input.route_end_mm.len(),
            input.envelope_m.len(),
            input.has_proposal.len(),
            input.proposal_speed_m_s.len(),
            input.proposal_travel_m.len(),
            output.speed_mm_s.len(),
            output.progress_mm.len(),
            output.carry_um.len(),
            output.travel_mm.len(),
            output.exhausted.len(),
            output.valid.len(),
        ];
        if lengths.iter().any(|&len| len != n)
            || output.hold.as_ref().is_some_and(|column| column.len() != n)
            || output.floats.as_ref().is_some_and(|floats| {
                floats.travel_m.len() != n
                    || floats.proposal_speed_m_s.len() != n
                    || floats.proposal_travel_m.len() != n
                    || floats
                        .window_m
                        .as_ref()
                        .is_some_and(|column| column.len() != n)
            })
        {
            return Err(LengthMismatch);
        }
        debug_assert!(
            [input.stop_m, input.route_end_m, input.envelope_m]
                .iter()
                .all(|column| column.iter().all(|value| !value.is_nan())),
            "motion bounds must not be NaN"
        );
        let mut stats = Stats {
            active_lanes: if TRACK {
                input.enabled.iter().filter(|&&x| x).count()
            } else {
                0
            },
            ..Stats::default()
        };
        // SAFETY: 唯一构造入口已经验证 CPU/OS 并绑定整个后端函数表；
        // 上方验证所有列等长，输出由互斥 &mut 借用提供。标量表返回 0，
        // 下方同布局尾部循环完成全部工作。不在热循环重查 ISA 或分派四车批次。
        let motion = if TRACK {
            self.motion
        } else {
            self.motion_untracked
        };
        let vector_end = unsafe { motion(input, output, delta_s, phase, &mut stats) };

        if TRACK {
            stats.vector_lanes = vector_end;
            stats.scalar_tail_lanes = n - vector_end;
        }
        for row in vector_end..n {
            scalar_row::<TRACK>(input, output, row, delta_s, phase, &mut stats);
        }
        Ok(stats)
    }

    /// 运行一轮带工作掩码的实际路线行走，保留边界余量与拒绝过门语义。
    ///
    /// # Errors
    /// 列长度不同时返回 `LengthMismatch`，全部输出保持原值。
    pub fn advance(
        self,
        input: &EdgeInput<'_>,
        output: &mut EdgeOutput<'_>,
    ) -> Result<EdgeStats, LengthMismatch> {
        let n = output.active.len();
        if [
            input.edge_length_mm.len(),
            input.can_hop.len(),
            input.has_next.len(),
            input.carry_um.len(),
            output.route_cursor.len(),
            output.progress_mm.len(),
            output.remaining_mm.len(),
            output.valid.len(),
        ]
        .iter()
        .any(|&len| len != n)
        {
            return Err(LengthMismatch);
        }
        let mut stats = EdgeStats {
            active_lanes: output.active.iter().filter(|&&x| x).count(),
            ..EdgeStats::default()
        };
        // SAFETY: 唯一构造入口已经验证 CPU/OS 并绑定整个后端函数表；
        // 上方验证所有列等长，输出由互斥 &mut 借用提供。标量表返回 0，
        // 下方同布局尾部循环完成全部工作。不在热循环重查 ISA 或分派四车批次。
        let vector_end = unsafe { (self.edge)(input, output, &mut stats) };

        stats.vector_lanes = vector_end;
        stats.scalar_tail_lanes = n - vector_end;
        for row in vector_end..n {
            if !output.active[row] {
                continue;
            }
            let length = input.edge_length_mm[row];
            let leftover = length.saturating_sub(output.progress_mm[row]);
            let remaining = output.remaining_mm[row];
            if remaining < leftover {
                output.progress_mm[row] = output.progress_mm[row].saturating_add(remaining);
                output.remaining_mm[row] = 0;
                output.active[row] = false;
            } else if !input.can_hop[row] || !input.has_next[row] {
                output.progress_mm[row] = length;
                output.remaining_mm[row] = 0;
                output.active[row] = false;
            } else if let Some(cursor) = output.route_cursor[row].checked_add(1) {
                output.route_cursor[row] = cursor;
                output.progress_mm[row] = 0;
                output.remaining_mm[row] = remaining - leftover;
                output.active[row] = output.remaining_mm[row] > 0 || input.carry_um[row] > 0;
                stats.hops += 1;
            } else {
                output.valid[row] = false;
                output.active[row] = false;
            }
        }
        Ok(stats)
    }

    /// 对实际工作掩码施加一轮精确制动约束，保持原 f32 运算次序。
    ///
    /// # Errors
    /// 列长度不同时返回 `LengthMismatch`，不修改提案速度。
    pub fn limit(
        self,
        input: &LimitInput<'_>,
        output: &mut LimitOutput<'_>,
        delta_s: f32,
    ) -> Result<EdgeStats, LengthMismatch> {
        let n = output.active.len();
        if [
            input.speed_mm_s.len(),
            input.distance_mm.len(),
            input.limit_mm_s.len(),
            input.window_m.len(),
            input.comfort_decel.len(),
            input.emergency_decel.len(),
            output.next_speed_m_s.len(),
        ]
        .iter()
        .any(|&len| len != n)
        {
            return Err(LengthMismatch);
        }
        let mut stats = EdgeStats {
            active_lanes: output.active.iter().filter(|&&x| x).count(),
            ..EdgeStats::default()
        };
        // SAFETY: 唯一构造入口已经验证 CPU/OS 并绑定整个后端函数表；
        // 上方验证所有列等长，输出由互斥 &mut 借用提供。标量表返回 0，
        // 下方同布局尾部循环完成全部工作。不在热循环重查 ISA 或分派四车批次。
        let vector_end = unsafe { (self.limit)(input, output, delta_s) };

        stats.vector_lanes = vector_end;
        stats.scalar_tail_lanes = n - vector_end;
        for row in vector_end..n {
            projection::scalar(input, output, row, delta_s);
        }
        Ok(stats)
    }

    /// 应用一轮实际降速边界裁剪，远于本拍旅行的通道退出工作掩码。
    ///
    /// # Errors
    /// 列长度不同时返回 `LengthMismatch`，保持全部输出。
    pub fn boundary(
        self,
        input: &BoundaryInput<'_>,
        output: &mut BoundaryOutput<'_>,
        delta_s: f32,
    ) -> Result<EdgeStats, LengthMismatch> {
        let n = output.active.len();
        if [
            input.speed_mm_s.len(),
            input.distance_mm.len(),
            input.limit_mm_s.len(),
            input.next_speed_m_s.len(),
            output.travel_m.len(),
        ]
        .iter()
        .any(|&len| len != n)
        {
            return Err(LengthMismatch);
        }
        let mut stats = EdgeStats {
            active_lanes: output.active.iter().filter(|&&x| x).count(),
            ..EdgeStats::default()
        };
        // SAFETY: 唯一构造入口已经验证 CPU/OS 并绑定整个后端函数表；
        // 上方验证所有列等长，输出由互斥 &mut 借用提供。标量表返回 0，
        // 下方同布局尾部循环完成全部工作。不在热循环重查 ISA 或分派四车批次。
        let vector_end = unsafe { (self.boundary)(input, output, delta_s) };

        stats.vector_lanes = vector_end;
        stats.scalar_tail_lanes = n - vector_end;
        for row in vector_end..n {
            projection::boundary_scalar(input, output, row, delta_s);
        }
        Ok(stats)
    }
}

/// #1272 停止目标速度归约的最严目标距离：denied 停止与有限路终取小。
/// 两列都以 `f32::INFINITY` 表示没有该类目标；`0` 表示已到目标，归耗尽语义。
fn stop_target_distance(input: &Input<'_>, row: usize) -> f32 {
    input.stop_m[row].min(input.route_end_m[row])
}

/// 停止求解是否可能收紧本拍速度（#1272 R6-3 复审修）：目标有效（有限、
/// 正距离）且不严格处于保守制动窗外。窗口与运行时 `speed_drop_window_upper`
/// 同式（`dt·(v+u) + u²/b_c`，u 取本拍可达且不超过期望的速度上界）；
/// 窗口 ≥ `v·dt + v²/(2b_c)`，窗外目标的求解根不低于提案已被抬升的值，
/// `min` 不改变结果——标量与向量同判据，据此跳过两档根式即等价。窗口
/// 证明不了（舒适减速度非正或数值非有限）时按可能参与保守处理。
fn stop_solve_participates(
    speed_mm_s: u32,
    desired_mm_s: u32,
    max_accel: f32,
    comfort: f32,
    delta_s: f32,
    distance_m: f32,
) -> bool {
    if !(distance_m > 0.0 && distance_m < f32::INFINITY) {
        return false;
    }
    let speed = f64::from(speed_mm_s) / 1_000.0;
    let desired = f64::from(desired_mm_s) / 1_000.0;
    let dt = f64::from(delta_s);
    let next = (speed + f64::from(max_accel) * dt).max(0.0).min(desired);
    let upper = next.max(speed.max(0.0));
    let window = if comfort > 0.0 {
        dt * (speed + upper) + upper * upper / f64::from(comfort)
    } else {
        f64::INFINITY
    };
    !window.is_finite() || f64::from(distance_m) < window
}

/// L=0 目标的「舒适 → 紧急 → 具名投影」级联。求解器可行域自带本拍减速上界
/// 与拍内停稳实际减速度定义域；两域皆空（本拍必越过目标）时保持紧急下界
/// 推进，位移由硬空间收到目标，线上速度交给耗尽层按主导源分档（过渡的
/// 舒适衰减仅在实际过渡耗尽时生效——F3 复审修 3）。
fn cap_next_speed_for_stop_target(
    speed: f32,
    next: f32,
    delta_s: f32,
    distance: f32,
    comfort: f32,
    emergency: f32,
) -> f32 {
    let floor_tier = emergency;
    projection::max_next_speed_for_decel(speed, next, delta_s, distance, 0.0, comfort)
        .or_else(|| {
            projection::max_next_speed_for_decel(speed, next, delta_s, distance, 0.0, emergency)
        })
        .unwrap_or_else(|| (speed - floor_tier * delta_s).max(0.0))
}

/// 拍内停稳（终速 0）时按产出该终速的档位取实际停车位移 `v²/(2b)`（舒适档
/// 优先）；两档都停不进目标距离时保留终态梯形，由边界裁剪投影到目标。
/// 未停稳用梯形积分，与 §11.1 的实际加速度积分一致。
fn stop_aware_travel(
    speed: f32,
    next: f32,
    delta_s: f32,
    stop_target: f32,
    comfort: f32,
    emergency: f32,
) -> f32 {
    let trapezoid = ((speed + next) * 0.5 * delta_s).max(0.0);
    if next > 0.0 || speed <= 0.0 {
        return trapezoid;
    }
    let comfort_stop = (speed * speed) / (2.0 * comfort);
    if speed <= comfort * delta_s && comfort_stop <= stop_target {
        return comfort_stop;
    }
    let emergency_stop = (speed * speed) / (2.0 * emergency);
    if speed <= emergency * delta_s && emergency_stop <= stop_target {
        return emergency_stop;
    }
    trapezoid
}

fn scalar_row<const TRACK: bool>(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    row: usize,
    delta_s: f32,
    phase: Phase,
    stats: &mut Stats,
) {
    if phase == Phase::Direct && !input.enabled[row] {
        output.valid[row] = false;
        return;
    }
    if phase == Phase::Quantize {
        let floats = output
            .floats
            .as_ref()
            .expect("quantization follows a float barrier");
        let (rounded_um, rounded_speed) = quantize_row(
            input.safe_quantize[row],
            floats.travel_m[row],
            floats.proposal_speed_m_s[row],
        );
        let valid = motion_values_valid(
            input,
            row,
            delta_s,
            floats.proposal_travel_m[row],
            floats.proposal_speed_m_s[row],
            floats.travel_m[row],
        );
        finish_values(
            input,
            output,
            row,
            delta_s,
            valid,
            rounded_um,
            rounded_speed,
        );
        return;
    }
    let speed = input.speed_mm_s[row] as f32 / 1_000.0;
    let desired = input.desired_mm_s[row] as f32 / 1_000.0;
    let project = matches!(phase, Phase::Project | Phase::FloatProject);
    let reused = input.has_proposal[row] || project;
    if TRACK {
        stats.record_proposal_source(input.enabled[row], reused);
    }
    let (_, raw_speed) = if reused {
        (input.proposal_travel_m[row], input.proposal_speed_m_s[row])
    } else {
        raw_proposal(&ProposalInput {
            speed_m_s: speed,
            desired_m_s: desired,
            leader_m: input.has_leader[row].then_some(input.leader_m[row]),
            leader_speed_m_s: input.leader_speed_m_s[row],
            min_gap_m: input.min_gap_m[row],
            time_headway: input.time_headway[row],
            max_accel: input.max_accel[row],
            comfort_decel: input.comfort_decel[row],
            emergency_decel: input.emergency_decel[row],
            delta_s,
        })
        .unwrap_or((f32::NAN, f32::NAN))
    };
    // #1272 候选形成（仅提案诞生处：Full/Direct/Proposal）：减速下界先于空间
    // 目标施加（raw 低于下界且安全域非空时属正常结果，不进投影分支；过渡预约
    // 用服务/舒适档下界）。目标距离 0 归耗尽语义，不在此归零。Project 读到的
    // 提案已经过归约，重放地板会把已收紧的速度抬回。
    let mut raw_speed = raw_speed;
    let stop_target = stop_target_distance(input, row);
    if !project {
        let floor_tier = input.emergency_decel[row];
        let floor = (speed - floor_tier * delta_s).max(0.0);
        if raw_speed.is_finite() {
            // 整数几何 §5 第 3 步：`max(min(u_raw, V_safe), F)`；无前车时安全上界为 +∞。
            if input.has_leader[row] {
                raw_speed = raw_speed.min(safe_speed(
                    speed,
                    floor_tier,
                    delta_s,
                    input.safe_room_m[row],
                ));
            }
            raw_speed = raw_speed.max(floor);
        }
        // R8 复审修：原始提案有效性贯穿归约——非有限提案（如 +∞ 缓存）
        // 不得进入停止目标 min：有限根会替换无穷值，把非法提案洗成合法
        // 速度（标量与向量后端一致拒绝；向量组内一行进求解也不得带入
        // 同组的非法行）。
        if raw_speed.is_finite()
            && stop_target > 0.0
            && stop_target.is_finite()
            && raw_speed > 0.0
            // R6-3 复审修：有限路终等远目标严格处于保守制动窗外时不进求解
            //（窗外求解根不低于已抬升提案，min 不改变结果；判据与向量
            // 整组跳过同式）。
            && stop_solve_participates(
                input.speed_mm_s[row],
                input.desired_mm_s[row],
                input.max_accel[row],
                input.comfort_decel[row],
                delta_s,
                stop_target,
            )
        {
            raw_speed = raw_speed.min(cap_next_speed_for_stop_target(
                speed,
                raw_speed,
                delta_s,
                stop_target,
                input.comfort_decel[row],
                input.emergency_decel[row],
            ));
        }
    }
    // vehicle-following §10.2：拍内停稳的档位还须停进跟车停车界，起始跟车可行时
    // 选中的停车行程不超过 `s_a`，不依赖后续几何裁剪。
    let stop_room = if input.has_leader[row] {
        stop_target.min(input.safe_room_m[row])
    } else {
        stop_target
    };
    let raw_travel = stop_aware_travel(
        speed,
        raw_speed,
        delta_s,
        stop_room,
        input.comfort_decel[row],
        input.emergency_decel[row],
    );
    if let Some(floats) = output.floats.as_mut() {
        floats.proposal_speed_m_s[row] = raw_speed;
        floats.proposal_travel_m[row] = raw_travel;
    }
    if phase == Phase::Proposal {
        if let Some(window) = output
            .floats
            .as_mut()
            .expect("proposal barrier")
            .window_m
            .as_mut()
        {
            // 已证速度上界取窗：floor 可把候选抬到 desired 之上，取
            // max(v, ·) 覆盖归一化后的共同候选，不随目标收紧缩小。
            let upper = (speed + input.max_accel[row] * delta_s)
                .min(desired)
                .max(speed)
                .max(0.0);
            window[row] = delta_s * (speed + upper) + upper * upper / input.comfort_decel[row];
        }
        output.valid[row] = raw_valid(input, row, delta_s, raw_travel, raw_speed);
        return;
    }
    let mut travel = raw_travel;
    if input.has_leader[row] {
        travel = travel.min((input.leader_m[row] - input.min_gap_m[row]).max(0.0));
    }
    travel = travel
        .min(input.stop_m[row].max(0.0))
        .min(input.route_end_m[row].max(0.0))
        .min(input.envelope_m[row])
        .max(0.0);
    if let Some(floats) = output.floats.as_mut() {
        floats.travel_m[row] = travel;
    }
    if phase == Phase::FloatProject {
        output.valid[row] =
            raw_valid(input, row, delta_s, raw_travel, raw_speed) && travel.is_finite();
        return;
    }
    let (rounded_um, rounded_speed) = quantize_row(input.safe_quantize[row], travel, raw_speed);
    let valid = motion_values_valid(input, row, delta_s, raw_travel, raw_speed, travel);
    finish_values(
        input,
        output,
        row,
        delta_s,
        valid,
        rounded_um,
        rounded_speed,
    );
}

/// 行程微米与速度 mm/s 的量化：默认 round-ties-to-even；跟车安全量化路径向零
/// 取整（非负值即向下取整），向量后端逐通道同式。
fn quantize_row(toward_zero: bool, travel: f32, speed: f32) -> (f64, f64) {
    let um = f64::from(travel) * 1_000_000.0;
    let mm = f64::from(speed.max(0.0)) * 1_000.0;
    if toward_zero {
        (um.floor(), mm.floor())
    } else {
        (um.round_ties_even(), mm.round_ties_even())
    }
}

fn raw_valid(input: &Input<'_>, row: usize, delta_s: f32, travel: f32, speed: f32) -> bool {
    input.enabled[row]
        && delta_s > 0.0
        && delta_s.is_finite()
        && input.max_accel[row] > 0.0
        && input.comfort_decel[row] > 0.0
        && input.emergency_decel[row] > 0.0
        && travel.is_finite()
        && speed.is_finite()
}

fn motion_values_valid(
    input: &Input<'_>,
    row: usize,
    delta_s: f32,
    raw_travel: f32,
    raw_speed: f32,
    travel: f32,
) -> bool {
    // 保留原完成原语的比较顺序与非有限边界；不通过改写 <= 为 > 收紧外部输入。
    !(delta_s <= 0.0
        || !delta_s.is_finite()
        || input.max_accel[row] <= 0.0
        || input.comfort_decel[row] <= 0.0
        || input.emergency_decel[row] <= 0.0
        || !raw_travel.is_finite()
        || !raw_speed.is_finite()
        || !travel.is_finite())
}

/// 整数几何 §5.1：常规候选（量化后、耗尽清零之前）经停车可行性保持改写；
/// 量化值越界或非有限时不改写，由完成原语原有的失败关闭处理。
fn hold_for_row(
    input: &Input<'_>,
    row: usize,
    delta_s: f32,
    rounded_um: f64,
    rounded_speed: f64,
) -> Option<HoldCommit> {
    let targets = [input.stop_mm[row], input.route_end_mm[row]];
    if targets.iter().all(|&target| target == NO_HOLD_TARGET)
        || input.hard_room_mm[row] == 0
        || !(rounded_um.is_finite() && rounded_um >= 0.0 && rounded_um <= u64::MAX as f64)
        || !(rounded_speed.is_finite()
            && rounded_speed >= 0.0
            && rounded_speed <= f64::from(u32::MAX))
    {
        return None;
    }
    hold_stop_feasibility(
        &HoldState {
            speed_mm_s: input.speed_mm_s[row],
            carry_um: input.carry_um[row],
            comfort_decel: input.comfort_decel[row],
            emergency_decel: input.emergency_decel[row],
            delta_ms: delta_ms(delta_s),
        },
        targets,
        rounded_speed as u32,
        rounded_um as u64,
    )
}

fn finish_values(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    row: usize,
    delta_s: f32,
    numeric_valid: bool,
    rounded_um: f64,
    rounded_speed: f64,
) {
    let hold = if input.enabled[row] && numeric_valid {
        hold_for_row(input, row, delta_s, rounded_um, rounded_speed)
    } else {
        None
    };
    finish_held(
        input,
        output,
        row,
        numeric_valid,
        rounded_um,
        rounded_speed,
        hold,
    );
}

/// 完成原语本体；`hold` 是同一行 [`hold_for_row`] 的结果（向量路径复用已求值结果）。
fn finish_held(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    row: usize,
    numeric_valid: bool,
    rounded_um: f64,
    rounded_speed: f64,
    hold: Option<HoldCommit>,
) {
    output.valid[row] = false;
    if !input.enabled[row] {
        return;
    }
    if !numeric_valid {
        return;
    }
    let room = input.hard_room_mm[row];
    let (carry_um, rounded_um, rounded_speed) = match hold {
        Some(commit) => (
            commit.carry_um,
            commit.travel_um as f64,
            f64::from(commit.speed_mm_s),
        ),
        None => (input.carry_um[row], rounded_um, rounded_speed),
    };
    if let Some(column) = output.hold.as_deref_mut() {
        column[row] = hold;
    }
    let projected_speed = (rounded_speed as u32).min(input.committed_limit_mm_s[row]);
    // F3' 复审修：过渡耗尽的「耗尽上界主导」（合同：耗尽拍由该上界主导，
    // 不与候选紧急投影取 min——r_p5 钉版 550 而非 100）；其余停止源维持
    // min(投影, 耗尽上界)。本列只在完成阶段消费，不影响候选形成。
    // 保持义务下的到线优先于其余正常行：零速（整数几何 §5.1）。
    let exhausted_speed = if hold.is_some_and(|commit| commit.arrives) {
        0
    } else if input.exhausted_cap_dominates[row] {
        input.exhausted_speed_cap_mm_s[row].min(input.committed_limit_mm_s[row])
    } else {
        projected_speed.min(input.exhausted_speed_cap_mm_s[row])
    };
    if room == 0 {
        output.travel_mm[row] = 0;
        output.speed_mm_s[row] = exhausted_speed;
        output.carry_um[row] = 0;
        output.progress_mm[row] = input.progress_mm[row];
        output.exhausted[row] = true;
        output.valid[row] = true;
        return;
    }
    if rounded_um < 0.0 || rounded_um > u64::MAX as f64 || !rounded_um.is_finite() {
        return;
    }
    let um = u64::from(carry_um).saturating_add(rounded_um as u64);
    let travel_mm = (um / 1_000).min(u64::from(room)) as u32;
    let exhausted = travel_mm == room;
    if !exhausted && (!rounded_speed.is_finite() || rounded_speed > f64::from(u32::MAX)) {
        return;
    }
    output.travel_mm[row] = travel_mm;
    output.progress_mm[row] = input.progress_mm[row].saturating_add(travel_mm);
    output.speed_mm_s[row] = if exhausted {
        exhausted_speed
    } else {
        projected_speed
    };
    output.carry_um[row] = if exhausted { 0 } else { (um % 1_000) as u16 };
    output.exhausted[row] = exhausted;
    output.valid[row] = true;
}

#[cfg(test)]
mod tests;
