//! 受保护转向走廊黄灯放行后的清空判据（#1496，#1446 工作包 2）与样本诊断（#1498）。
//!
//! 判据只读每个提交边界的公开车辆状态与已提交灯态，不调用 Runtime 的门决策、
//! 困境区豁免或停止求解器：
//!
//! - **个体清空**：车辆以拍初为黄灯的组提交 `fromEdge → 内部边` 跨越时，记录该组
//!   本轮黄灯起点 `t_y`、跨越边界 `t_e` 与车尾完全离开内部边的边界 `t_c`，要求
//!   `t_c − t_y ≤ Y + R`。`Y` 是该组本轮黄灯时长，`R` 是其后到同一控制器下一个组
//!   转绿的全红时长，均取自已提交灯态时间线。
//! - **相位交接清空**：同一控制器上一个组转绿的边界，前一 active set（此前最后一个
//!   非红组）的内部边上没有任何车辆车身。
//!
//! 另外拒绝以拍初红灯提交的跨越。
//!
//! **诊断**只附加事实，不参与判定：每个黄灯期跨越样本另外保存黄灯起点时的运动
//! 状态、跨越前后两个提交边界、从黄灯起点到车尾清空（或窗口结束）的逐拍轨迹，
//! 并由 [`analyze`] 给出带依据的候选原因。候选原因可以同时成立多项，也可以为空
//! （暂无法确定）；它们是对可观测事实的推断，不是 Runtime 实际起作用的约束。

use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::hash::Hash;

use laneflow_static_contract::SignalAspect;

/// 黄灯起点时只跟踪到停止线不超过该距离的车辆：约为 3 s 黄灯内按主路限速可达
/// 距离的两倍，更远的车辆不可能在本轮黄灯内跨越。
pub const APPROACH_WINDOW_MM: u64 = 100_000;
/// 相邻两行轨迹的平均加速度不高于该值（取负）时记为一次减速拍。
pub const DECEL_THRESHOLD_MM_S2: i64 = 500;

/// 路线上的一次机动路径出现：`from_index` 是 fromEdge 在路线中的下标，内部边紧随其后。
#[derive(Clone, Copy, Debug)]
pub struct PathOccurrence {
    pub path: u32,
    pub group: usize,
    pub from_index: u32,
    /// 内部边在路线坐标中的区间 `[start_mm, end_mm)`；`start_mm` 即停止线位置。
    pub start_mm: u64,
    pub end_mm: u64,
}

/// 一条路线的坐标布局：各边起点前缀和与机动路径出现。
#[derive(Clone, Debug, Default)]
pub struct RouteLayout {
    pub prefix_mm: Vec<u64>,
    pub occurrences: Vec<PathOccurrence>,
}

/// 前方最近车辆：按车道边连接求得的车距（前车车尾到本车前杠）与其速度。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Leader {
    pub gap_mm: u64,
    pub speed_mm_s: u32,
}

/// 一个提交边界上的一辆 Active 车辆。
#[derive(Clone, Copy, Debug)]
pub struct VehicleSample<K, R> {
    pub key: K,
    pub route: R,
    pub edge_index: u32,
    pub progress_mm: u32,
    pub length_mm: u32,
    pub speed_mm_s: u32,
    /// 前杠所在边的限速。
    pub limit_mm_s: u32,
    pub leader: Option<Leader>,
}

/// 一个提交边界的公开观测。
pub struct Frame<'a, K, R> {
    pub time_ms: u64,
    /// 按信号组序号排列的已提交灯态。
    pub aspects: &'a [SignalAspect],
    pub vehicles: &'a [VehicleSample<K, R>],
}

/// 一个提交边界上的运动状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kinematics {
    pub time_ms: u64,
    pub edge_index: u32,
    pub progress_mm: u32,
    /// 前杠的路线坐标。
    pub front_mm: u64,
    pub speed_mm_s: u32,
}

/// 前方最近停止线（下一机动路径出现的 fromEdge 终点）与其所属组的灯态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextStop {
    pub distance_mm: u64,
    pub aspect: SignalAspect,
}

/// 轨迹行所处阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// 黄灯起点到跨越前一个边界。
    Approach,
    /// 跨越边界到车尾清空。
    Inside,
}

/// 一个提交边界上的诊断事实。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceRow {
    pub time_ms: u64,
    pub phase: Phase,
    pub edge_index: u32,
    pub front_mm: u64,
    pub rear_mm: u64,
    pub speed_mm_s: u32,
    pub limit_mm_s: u32,
    pub leader: Option<Leader>,
    pub next_stop: Option<NextStop>,
}

/// 一个黄灯期跨越样本；`t_c` 与 `budget_ms` 在可判定后补齐。
#[derive(Clone)]
pub struct YellowSample<K, R> {
    pub key: K,
    pub route: R,
    pub path: u32,
    pub group: usize,
    pub from_index: u32,
    pub t_y: u64,
    pub t_e: u64,
    pub t_c: Option<u64>,
    /// `Y + R`：该组本轮黄灯起点到同控制器下一组转绿的时长。
    pub budget_ms: Option<u64>,
    pub length_mm: u32,
    pub start_mm: u64,
    pub end_mm: u64,
    /// 黄灯起点边界上的状态；车辆当时不在跟踪窗口内时为 `None`。
    pub at_yellow: Option<Kinematics>,
    /// 跨越前一个提交边界（前杠仍在 fromEdge）。
    pub before: Kinematics,
    /// 跨越后的提交边界 `t_e`（前杠可能已进入内部边一段距离）。
    pub after: Kinematics,
    /// 从黄灯起点（或首次观测）到 `t_c`（或窗口结束）的逐拍事实。
    pub trace: Vec<TraceRow>,
}

impl<K, R> YellowSample<K, R> {
    pub fn elapsed_ms(&self) -> Option<u64> {
        self.t_c.map(|t_c| t_c - self.t_y)
    }

    pub fn is_late(&self) -> bool {
        matches!((self.elapsed_ms(), self.budget_ms), (Some(e), Some(b)) if e > b)
    }

    pub fn is_unresolved(&self) -> bool {
        self.t_c.is_none() || self.budget_ms.is_none()
    }
}

/// 失败输出省略逐拍轨迹（另行写出），只保留样本身份、边界与跨越前后状态。
impl<K: Debug, R: Debug> Debug for YellowSample<K, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("YellowSample")
            .field("key", &self.key)
            .field("route", &self.route)
            .field("path", &self.path)
            .field("group", &self.group)
            .field("from_index", &self.from_index)
            .field("t_y", &self.t_y)
            .field("t_e", &self.t_e)
            .field("t_c", &self.t_c)
            .field("budget_ms", &self.budget_ms)
            .field("before", &self.before)
            .field("after", &self.after)
            .field("trace_rows", &self.trace.len())
            .finish()
    }
}

#[derive(Clone, Debug)]
#[expect(dead_code, reason = "部分字段只经 Debug 输出为失败事实")]
pub enum Violation<K, R> {
    /// `t_c − t_y > Y + R`。
    LateClearance(YellowSample<K, R>),
    /// 以拍初红灯提交的跨越。
    RedEntry {
        key: K,
        route: R,
        path: u32,
        group: usize,
        from_index: u32,
        t_e: u64,
    },
    /// 同控制器下一组转绿时，前一 active set 的内部边仍有车身。
    HandoffOccupied {
        time_ms: u64,
        green_group: usize,
        previous_group: usize,
        key: K,
        route: R,
        path: u32,
        edge_index: u32,
        progress_mm: u32,
        rear_mm: u64,
        front_mm: u64,
        start_mm: u64,
        end_mm: u64,
    },
    /// 车辆在车尾离开内部边前消失（不应发生：路终在出口边之后）。
    LostBeforeClear(YellowSample<K, R>),
}

#[derive(Clone, Copy, Debug)]
struct Open {
    occurrence: usize,
    sample: Option<usize>,
}

struct GroupClock {
    controller: usize,
    yellow_since: Option<u64>,
    /// 本轮黄灯起点时位于跟踪窗口内、尚未跨越的车辆及其逐拍事实。
    approach: Option<HashMap<usize, Vec<TraceRow>>>,
}

/// 逐边界判据状态机。
pub struct ClearanceCheck<K, R> {
    layouts: HashMap<R, RouteLayout>,
    groups: Vec<GroupClock>,
    /// 每个控制器此前最后一个非红组。
    last_active: Vec<Option<usize>>,
    previous_aspects: Vec<SignalAspect>,
    previous: HashMap<K, (R, Kinematics)>,
    open: HashMap<K, Open>,
    /// 跟踪中的车辆键 → 稠密下标（`GroupClock::approach` 以下标索引）。
    keys: HashMap<K, usize>,
    /// 黄灯起点等待“下一组转绿”的样本下标，按组。
    awaiting_budget: Vec<Vec<usize>>,
    pub samples: Vec<YellowSample<K, R>>,
    pub violations: Vec<Violation<K, R>>,
    pub traversals: HashMap<u32, u64>,
    pub handoffs: u64,
    pub frames: u64,
}

impl<K, R> ClearanceCheck<K, R>
where
    K: Copy + Eq + Hash + Debug,
    R: Copy + Eq + Hash + Debug,
{
    /// `group_controllers[g]` 是组 `g` 的控制器序号。
    pub fn new(group_controllers: &[usize]) -> Self {
        let controllers = group_controllers.iter().copied().max().map_or(0, |m| m + 1);
        Self {
            layouts: HashMap::new(),
            groups: group_controllers
                .iter()
                .map(|&controller| GroupClock {
                    controller,
                    yellow_since: None,
                    approach: None,
                })
                .collect(),
            last_active: vec![None; controllers],
            previous_aspects: Vec::new(),
            previous: HashMap::new(),
            open: HashMap::new(),
            keys: HashMap::new(),
            awaiting_budget: vec![Vec::new(); group_controllers.len()],
            samples: Vec::new(),
            violations: Vec::new(),
            traversals: HashMap::new(),
            handoffs: 0,
            frames: 0,
        }
    }

    pub fn has_layout(&self, route: R) -> bool {
        self.layouts.contains_key(&route)
    }

    pub fn add_layout(&mut self, route: R, layout: RouteLayout) {
        self.layouts.insert(route, layout);
    }

    fn front_mm(&self, vehicle: &VehicleSample<K, R>) -> u64 {
        self.layouts[&vehicle.route].prefix_mm[vehicle.edge_index as usize]
            + u64::from(vehicle.progress_mm)
    }

    fn kinematics(&self, time_ms: u64, vehicle: &VehicleSample<K, R>) -> Kinematics {
        Kinematics {
            time_ms,
            edge_index: vehicle.edge_index,
            progress_mm: vehicle.progress_mm,
            front_mm: self.front_mm(vehicle),
            speed_mm_s: vehicle.speed_mm_s,
        }
    }

    /// 前杠不晚于其停止线的下一个机动路径出现。
    fn next_occurrence(&self, vehicle: &VehicleSample<K, R>) -> Option<PathOccurrence> {
        let front = self.front_mm(vehicle);
        self.layouts[&vehicle.route]
            .occurrences
            .iter()
            .copied()
            .find(|occurrence| {
                vehicle.edge_index <= occurrence.from_index && front <= occurrence.start_mm
            })
    }

    fn row(
        &self,
        frame: &Frame<'_, K, R>,
        vehicle: &VehicleSample<K, R>,
        phase: Phase,
    ) -> TraceRow {
        let front = self.front_mm(vehicle);
        TraceRow {
            time_ms: frame.time_ms,
            phase,
            edge_index: vehicle.edge_index,
            front_mm: front,
            rear_mm: front.saturating_sub(u64::from(vehicle.length_mm)),
            speed_mm_s: vehicle.speed_mm_s,
            limit_mm_s: vehicle.limit_mm_s,
            leader: vehicle.leader,
            next_stop: self.next_occurrence(vehicle).map(|occurrence| NextStop {
                distance_mm: occurrence.start_mm - front,
                aspect: frame.aspects[occurrence.group],
            }),
        }
    }

    fn dense(&mut self, key: K) -> usize {
        let next = self.keys.len();
        *self.keys.entry(key).or_insert(next)
    }

    /// 消费一个提交边界。首个边界只建立基线。
    pub fn observe(&mut self, frame: &Frame<'_, K, R>) {
        self.frames += 1;
        let first = self.previous_aspects.is_empty();
        if first {
            self.previous_aspects = frame.aspects.to_vec();
            for (group, aspect) in frame.aspects.iter().enumerate() {
                if *aspect != SignalAspect::Red {
                    self.last_active[self.groups[group].controller] = Some(group);
                }
                if *aspect == SignalAspect::Yellow {
                    // 起始即为黄灯：起点未知，不作样本来源。
                    self.groups[group].yellow_since = None;
                }
            }
        }
        // 1. 跨越：拍初（上一边界）灯态决定许可。
        for vehicle in frame.vehicles {
            let Some(&(route, before)) = self.previous.get(&vehicle.key) else {
                continue;
            };
            if route != vehicle.route || vehicle.edge_index <= before.edge_index {
                continue;
            }
            let occurrences = self.layouts[&vehicle.route].occurrences.clone();
            for (index, occurrence) in occurrences.iter().enumerate() {
                if before.edge_index > occurrence.from_index
                    || vehicle.edge_index <= occurrence.from_index
                {
                    continue;
                }
                *self.traversals.entry(occurrence.path).or_default() += 1;
                let aspect = self.previous_aspects[occurrence.group];
                let sample = match aspect {
                    SignalAspect::Green => None,
                    SignalAspect::Red => {
                        self.violations.push(Violation::RedEntry {
                            key: vehicle.key,
                            route: vehicle.route,
                            path: occurrence.path,
                            group: occurrence.group,
                            from_index: occurrence.from_index,
                            t_e: frame.time_ms,
                        });
                        None
                    }
                    SignalAspect::Yellow => match self.groups[occurrence.group].yellow_since {
                        None => None,
                        Some(t_y) => {
                            let dense = self.dense(vehicle.key);
                            let approach = self.groups[occurrence.group]
                                .approach
                                .as_mut()
                                .and_then(|map| map.remove(&dense))
                                .unwrap_or_default();
                            let at_yellow =
                                approach
                                    .first()
                                    .filter(|row| row.time_ms == t_y)
                                    .map(|row| Kinematics {
                                        time_ms: row.time_ms,
                                        edge_index: row.edge_index,
                                        progress_mm: u32::try_from(
                                            row.front_mm
                                                - self.layouts[&vehicle.route].prefix_mm
                                                    [row.edge_index as usize],
                                        )
                                        .expect("progress fits"),
                                        front_mm: row.front_mm,
                                        speed_mm_s: row.speed_mm_s,
                                    });
                            self.samples.push(YellowSample {
                                key: vehicle.key,
                                route: vehicle.route,
                                path: occurrence.path,
                                group: occurrence.group,
                                from_index: occurrence.from_index,
                                t_y,
                                t_e: frame.time_ms,
                                t_c: None,
                                budget_ms: None,
                                length_mm: vehicle.length_mm,
                                start_mm: occurrence.start_mm,
                                end_mm: occurrence.end_mm,
                                at_yellow,
                                before,
                                after: self.kinematics(frame.time_ms, vehicle),
                                trace: approach,
                            });
                            let sample = self.samples.len() - 1;
                            self.awaiting_budget[occurrence.group].push(sample);
                            Some(sample)
                        }
                    },
                    other => panic!("走廊判据未定义的灯态 {other:?}"),
                };
                self.open.insert(
                    vehicle.key,
                    Open {
                        occurrence: index,
                        sample,
                    },
                );
            }
        }
        // 1b. 仍在接近的跟踪车辆追加本边界事实。
        for vehicle in frame.vehicles {
            let Some(&dense) = self.keys.get(&vehicle.key) else {
                continue;
            };
            let row = self.row(frame, vehicle, Phase::Approach);
            for group in &mut self.groups {
                if let Some(rows) = group.approach.as_mut().and_then(|map| map.get_mut(&dense)) {
                    rows.push(row);
                }
            }
        }
        // 2. 车尾清空：首个车尾不早于内部边终点的边界即 t_c。
        let mut present: HashMap<K, ()> = HashMap::with_capacity(frame.vehicles.len());
        for vehicle in frame.vehicles {
            present.insert(vehicle.key, ());
            let Some(open) = self.open.get(&vehicle.key).copied() else {
                continue;
            };
            let occurrence = self.layouts[&vehicle.route].occurrences[open.occurrence];
            if let Some(sample) = open.sample {
                let row = self.row(frame, vehicle, Phase::Inside);
                self.samples[sample].trace.push(row);
            }
            let rear = self
                .front_mm(vehicle)
                .saturating_sub(u64::from(vehicle.length_mm));
            if rear >= occurrence.end_mm {
                if let Some(sample) = open.sample {
                    self.samples[sample].t_c = Some(frame.time_ms);
                    self.finish(sample);
                }
                self.open.remove(&vehicle.key);
            }
        }
        let lost: Vec<K> = self
            .open
            .keys()
            .copied()
            .filter(|key| !present.contains_key(key))
            .collect();
        for key in lost {
            if let Some(Open {
                sample: Some(sample),
                ..
            }) = self.open.remove(&key)
            {
                self.violations
                    .push(Violation::LostBeforeClear(self.samples[sample].clone()));
            }
        }
        // 3. 灯态边沿：黄灯起点、转绿交接与 Y + R 预算。
        if !first {
            for (group, aspect) in frame.aspects.iter().copied().enumerate() {
                let before = self.previous_aspects[group];
                if aspect == before {
                    continue;
                }
                let controller = self.groups[group].controller;
                if before == SignalAspect::Yellow {
                    // 拍初黄灯的跨越已在本边界判定；之后的跨越不再是黄灯样本。
                    self.groups[group].approach = None;
                }
                if aspect == SignalAspect::Yellow {
                    self.groups[group].yellow_since = Some(frame.time_ms);
                    self.start_approach(frame, group);
                }
                if aspect == SignalAspect::Green {
                    if let Some(previous) = self.last_active[controller]
                        && previous != group
                    {
                        self.handoffs += 1;
                        self.check_handoff(frame, group, previous);
                        let waiting = std::mem::take(&mut self.awaiting_budget[previous]);
                        for sample in waiting {
                            let t_y = self.samples[sample].t_y;
                            self.samples[sample].budget_ms = Some(frame.time_ms - t_y);
                            self.finish(sample);
                        }
                    }
                    self.last_active[controller] = Some(group);
                }
            }
        }
        self.previous_aspects.clear();
        self.previous_aspects.extend_from_slice(frame.aspects);
        self.previous.clear();
        for vehicle in frame.vehicles {
            let kinematics = self.kinematics(frame.time_ms, vehicle);
            self.previous
                .insert(vehicle.key, (vehicle.route, kinematics));
        }
    }

    /// 黄灯起点：记录跟踪窗口内、下一停止线属于该组的车辆。
    fn start_approach(&mut self, frame: &Frame<'_, K, R>, group: usize) {
        let mut map = HashMap::new();
        for vehicle in frame.vehicles {
            let Some(occurrence) = self.next_occurrence(vehicle) else {
                continue;
            };
            if occurrence.group != group
                || occurrence.start_mm - self.front_mm(vehicle) > APPROACH_WINDOW_MM
            {
                continue;
            }
            let row = self.row(frame, vehicle, Phase::Approach);
            let dense = self.dense(vehicle.key);
            map.insert(dense, vec![row]);
        }
        self.groups[group].approach = Some(map);
    }

    fn check_handoff(&mut self, frame: &Frame<'_, K, R>, green: usize, previous: usize) {
        for vehicle in frame.vehicles {
            let layout = &self.layouts[&vehicle.route];
            let front =
                layout.prefix_mm[vehicle.edge_index as usize] + u64::from(vehicle.progress_mm);
            let rear = front.saturating_sub(u64::from(vehicle.length_mm));
            for occurrence in &layout.occurrences {
                if occurrence.group == previous
                    && rear < occurrence.end_mm
                    && front > occurrence.start_mm
                {
                    self.violations.push(Violation::HandoffOccupied {
                        time_ms: frame.time_ms,
                        green_group: green,
                        previous_group: previous,
                        key: vehicle.key,
                        route: vehicle.route,
                        path: occurrence.path,
                        edge_index: vehicle.edge_index,
                        progress_mm: vehicle.progress_mm,
                        rear_mm: rear,
                        front_mm: front,
                        start_mm: occurrence.start_mm,
                        end_mm: occurrence.end_mm,
                    });
                }
            }
        }
    }

    /// `t_c` 与预算都已知时判定。
    fn finish(&mut self, sample: usize) {
        let s = &self.samples[sample];
        if let (Some(t_c), Some(budget)) = (s.t_c, s.budget_ms)
            && t_c - s.t_y > budget
        {
            self.violations.push(Violation::LateClearance(s.clone()));
        }
    }

    /// 运行结束：仍未清空或未取得预算的样本数（窗口截断，不计为违例）。
    pub fn unresolved(&self) -> usize {
        self.samples.iter().filter(|s| s.is_unresolved()).count()
    }
}

// ---------- 原因分析 ----------

/// 分析用的车辆 profile 参数（静态数据）。
#[derive(Clone, Copy, Debug)]
pub struct AnalysisParams {
    pub emergency_decel_mm_s2: u32,
    pub comfort_decel_mm_s2: u32,
    pub min_gap_mm: u32,
    pub time_headway_ms: u32,
}

/// 候选原因及其依据。同一样本可以同时成立多项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Factor {
    /// 从跨越后的实际位置起，即使以内部边限速匀速行驶，车尾清空也超出剩余预算。
    TimingInsufficient { required_ms: u64, remaining_ms: u64 },
    /// 跨越后速度低于内部边限速，按跨越后速度匀速行驶会超出剩余预算，
    /// 而按限速不会；`required_ms` 为 `None` 表示跨越后速度为零。
    SlowEntry {
        entry_mm_s: u32,
        limit_mm_s: u32,
        required_ms: Option<u64>,
        remaining_ms: u64,
    },
    /// 减速拍时速度高于前杠所在边限速：向较低限速减速（例如转弯降速）。
    LimitReduction { ticks: u32 },
    /// 减速拍时前车车距在跟驰范围内（`≤ 2 × (min_gap + v × time_headway)`）。
    LeaderInRange { ticks: u32, min_gap_mm: u64 },
    /// 减速拍时前方停止线为非绿灯，且距离在舒适停车距离两倍加最小间距以内。
    StopLineAhead { ticks: u32, min_distance_mm: u64 },
    /// 减速拍时以上三项都不成立。
    UnattributedDecel { ticks: u32 },
    /// 接近阶段前方停止线为非绿灯、按紧急减速度仍能停在线前（`v²/(2b_e) ≤ d`），
    /// 下一边界速度却没有下降的拍数；`exempt_from_ms` 是豁免判据按公开状态首次
    /// 成立的时刻相对 `t_y` 的偏移。这是可观测事实，不推断 Runtime 内部原因。
    StoppableWithoutBraking {
        ticks: u32,
        comfort_feasible_at_yellow: bool,
        exempt_from_ms: Option<u64>,
    },
}

/// 一个样本的诊断结论。
#[derive(Clone, Debug)]
pub struct Analysis {
    /// 黄灯起点时到停止线的路线距离与紧急停车距离 `v²/(2b_e)`。
    pub distance_to_line_mm: Option<u64>,
    pub stop_distance_mm: Option<u64>,
    /// 黄灯起点时是否满足困境区豁免的能力判据 `v²/(2b_e) > d`（按公开状态复算）。
    pub cannot_stop_at_yellow: Option<bool>,
    /// 跨越后前杠到“车尾离开内部边”所需的剩余行驶距离。
    pub remaining_mm: u64,
    /// 跨越后剩余的 `Y + R` 预算：`budget − (t_e − t_y)`。
    pub remaining_budget_ms: Option<u64>,
    /// 候选原因；为空表示暂无法确定。
    pub factors: Vec<Factor>,
}

fn travel_ms(distance_mm: u64, speed_mm_s: u32) -> Option<u64> {
    (speed_mm_s > 0).then(|| (distance_mm * 1_000).div_ceil(u64::from(speed_mm_s)))
}

/// 由样本的可观测事实推断候选原因。
pub fn analyze<K, R>(sample: &YellowSample<K, R>, params: &AnalysisParams) -> Analysis {
    let distance_to_line_mm = sample
        .at_yellow
        .map(|k| sample.start_mm.saturating_sub(k.front_mm));
    let stop_distance_mm = sample.at_yellow.map(|k| {
        let v = u64::from(k.speed_mm_s);
        (v * v).div_ceil(2 * u64::from(params.emergency_decel_mm_s2.max(1)))
    });
    let cannot_stop_at_yellow = match (stop_distance_mm, distance_to_line_mm) {
        (Some(stop), Some(distance)) => Some(stop > distance),
        _ => None,
    };
    let remaining_mm =
        (sample.end_mm + u64::from(sample.length_mm)).saturating_sub(sample.after.front_mm);
    let remaining_budget_ms = sample
        .budget_ms
        .map(|budget| budget.saturating_sub(sample.t_e - sample.t_y));
    let mut factors = Vec::new();

    let inside_limit = sample
        .trace
        .iter()
        .find(|row| row.phase == Phase::Inside)
        .map(|row| row.limit_mm_s);
    if let (Some(remaining_ms), Some(limit)) = (remaining_budget_ms, inside_limit) {
        let at_limit = travel_ms(remaining_mm, limit);
        let timing = at_limit.is_none_or(|t| t > remaining_ms);
        if timing {
            factors.push(Factor::TimingInsufficient {
                required_ms: at_limit.unwrap_or(u64::MAX),
                remaining_ms,
            });
        }
        let entry = sample.after.speed_mm_s;
        if !timing && entry < limit {
            let at_entry = travel_ms(remaining_mm, entry);
            if at_entry.is_none_or(|t| t > remaining_ms) {
                factors.push(Factor::SlowEntry {
                    entry_mm_s: entry,
                    limit_mm_s: limit,
                    required_ms: at_entry,
                    remaining_ms,
                });
            }
        }
    }

    let (mut limit_ticks, mut leader_ticks, mut stop_ticks, mut other_ticks) = (0, 0, 0, 0);
    let mut min_gap = u64::MAX;
    let mut min_stop = u64::MAX;
    for pair in sample.trace.windows(2) {
        let (r0, r1) = (pair[0], pair[1]);
        let dt = r1.time_ms.saturating_sub(r0.time_ms);
        if dt == 0 {
            continue;
        }
        let accel = (i64::from(r1.speed_mm_s) - i64::from(r0.speed_mm_s)) * 1_000
            / i64::try_from(dt).expect("dt");
        if accel > -DECEL_THRESHOLD_MM_S2 {
            continue;
        }
        let v = u64::from(r0.speed_mm_s);
        let mut attributed = false;
        if r0.speed_mm_s > r1.limit_mm_s {
            limit_ticks += 1;
            attributed = true;
        }
        if let Some(leader) = r0.leader {
            let range =
                2 * (u64::from(params.min_gap_mm) + v * u64::from(params.time_headway_ms) / 1_000);
            if leader.gap_mm <= range {
                leader_ticks += 1;
                min_gap = min_gap.min(leader.gap_mm);
                attributed = true;
            }
        }
        if let Some(stop) = r0.next_stop
            && stop.aspect != SignalAspect::Green
        {
            let comfort = (v * v).div_ceil(2 * u64::from(params.comfort_decel_mm_s2.max(1)));
            if stop.distance_mm <= 2 * comfort + u64::from(params.min_gap_mm) {
                stop_ticks += 1;
                min_stop = min_stop.min(stop.distance_mm);
                attributed = true;
            }
        }
        if !attributed {
            other_ticks += 1;
        }
    }
    if limit_ticks > 0 {
        factors.push(Factor::LimitReduction { ticks: limit_ticks });
    }
    if leader_ticks > 0 {
        factors.push(Factor::LeaderInRange {
            ticks: leader_ticks,
            min_gap_mm: min_gap,
        });
    }
    if stop_ticks > 0 {
        factors.push(Factor::StopLineAhead {
            ticks: stop_ticks,
            min_distance_mm: min_stop,
        });
    }
    if other_ticks > 0 {
        factors.push(Factor::UnattributedDecel { ticks: other_ticks });
    }

    let emergency = u64::from(params.emergency_decel_mm_s2.max(1));
    let mut coasting = 0;
    let mut exempt_from_ms = None;
    let approach: Vec<_> = sample
        .trace
        .iter()
        .take_while(|row| row.phase == Phase::Approach)
        .collect();
    for (index, row) in approach.iter().enumerate() {
        let Some(stop) = row
            .next_stop
            .filter(|stop| stop.aspect != SignalAspect::Green)
        else {
            continue;
        };
        let v = u64::from(row.speed_mm_s);
        // 与 Runtime 判据同形：v²/(2b_e) > d，两边乘 2b_e 后整数比较。
        let cannot_stop = v * v > 2 * emergency * stop.distance_mm;
        if cannot_stop {
            exempt_from_ms.get_or_insert(row.time_ms - sample.t_y);
            continue;
        }
        let next_speed = approach
            .get(index + 1)
            .map_or(sample.after.speed_mm_s, |next| next.speed_mm_s);
        if next_speed >= row.speed_mm_s && row.speed_mm_s > 0 {
            coasting += 1;
        }
    }
    if coasting > 0 {
        let comfort_feasible_at_yellow = sample.at_yellow.is_some_and(|k| {
            let v = u64::from(k.speed_mm_s);
            v * v
                <= 2 * u64::from(params.comfort_decel_mm_s2.max(1))
                    * sample.start_mm.saturating_sub(k.front_mm)
        });
        factors.push(Factor::StoppableWithoutBraking {
            ticks: coasting,
            comfort_feasible_at_yellow,
            exempt_from_ms,
        });
    }
    Analysis {
        distance_to_line_mm,
        stop_distance_mm,
        cannot_stop_at_yellow,
        remaining_mm,
        remaining_budget_ms,
        factors,
    }
}
