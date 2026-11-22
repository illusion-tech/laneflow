//! 受保护转向走廊黄灯放行后的清空判据（#1496，#1446 工作包 2）。
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

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use laneflow_static_contract::SignalAspect;

/// 路线上的一次机动路径出现：`from_index` 是 fromEdge 在路线中的下标，内部边紧随其后。
#[derive(Clone, Copy, Debug)]
pub struct PathOccurrence {
    pub path: u32,
    pub group: usize,
    pub from_index: u32,
    /// 内部边在路线坐标中的区间 `[start_mm, end_mm)`。
    pub start_mm: u64,
    pub end_mm: u64,
}

/// 一条路线的坐标布局：各边起点前缀和与机动路径出现。
#[derive(Clone, Debug, Default)]
pub struct RouteLayout {
    pub prefix_mm: Vec<u64>,
    pub occurrences: Vec<PathOccurrence>,
}

/// 一个提交边界上的一辆 Active 车辆。
#[derive(Clone, Copy, Debug)]
pub struct VehicleSample<K, R> {
    pub key: K,
    pub route: R,
    pub edge_index: u32,
    pub progress_mm: u32,
    pub length_mm: u32,
}

/// 一个提交边界的公开观测。
pub struct Frame<'a, K, R> {
    pub time_ms: u64,
    /// 按信号组序号排列的已提交灯态。
    pub aspects: &'a [SignalAspect],
    pub vehicles: &'a [VehicleSample<K, R>],
}

/// 一个黄灯期跨越样本；`t_c` 与 `budget_ms` 在可判定后补齐。
#[derive(Clone, Debug)]
#[expect(dead_code, reason = "部分字段只经 Debug 输出为失败事实")]
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
}

impl<K, R> YellowSample<K, R> {
    pub fn elapsed_ms(&self) -> Option<u64> {
        self.t_c.map(|t_c| t_c - self.t_y)
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
}

/// 逐边界判据状态机。
pub struct ClearanceCheck<K, R> {
    layouts: HashMap<R, RouteLayout>,
    groups: Vec<GroupClock>,
    /// 每个控制器此前最后一个非红组。
    last_active: Vec<Option<usize>>,
    previous_aspects: Vec<SignalAspect>,
    previous_edges: HashMap<K, (R, u32)>,
    open: HashMap<K, Open>,
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
                })
                .collect(),
            last_active: vec![None; controllers],
            previous_aspects: Vec::new(),
            previous_edges: HashMap::new(),
            open: HashMap::new(),
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
            let Some(&(route, previous)) = self.previous_edges.get(&vehicle.key) else {
                continue;
            };
            if route != vehicle.route || vehicle.edge_index <= previous {
                continue;
            }
            let layout = &self.layouts[&vehicle.route];
            for (index, occurrence) in layout.occurrences.iter().enumerate() {
                if previous > occurrence.from_index || vehicle.edge_index <= occurrence.from_index {
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
                    SignalAspect::Yellow => self.groups[occurrence.group].yellow_since.map(|t_y| {
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
                        });
                        let sample = self.samples.len() - 1;
                        self.awaiting_budget[occurrence.group].push(sample);
                        sample
                    }),
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
        // 2. 车尾清空：首个车尾不早于内部边终点的边界即 t_c。
        let mut present: HashMap<K, ()> = HashMap::with_capacity(frame.vehicles.len());
        for vehicle in frame.vehicles {
            present.insert(vehicle.key, ());
            let Some(open) = self.open.get(&vehicle.key).copied() else {
                continue;
            };
            let occurrence = self.layouts[&vehicle.route].occurrences[open.occurrence];
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
                if aspect == SignalAspect::Yellow {
                    self.groups[group].yellow_since = Some(frame.time_ms);
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
        self.previous_edges.clear();
        for vehicle in frame.vehicles {
            self.previous_edges
                .insert(vehicle.key, (vehicle.route, vehicle.edge_index));
        }
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
        self.samples
            .iter()
            .filter(|s| s.t_c.is_none() || s.budget_ms.is_none())
            .count()
    }
}
