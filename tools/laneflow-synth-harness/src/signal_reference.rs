//! W 域六相位信号参考：由冻结六阶段公式独立计算拍初预期灯色，供两条观测
//! 路径（普通与 burst）构造拍初事实。灯色事实交给共享包
//! `laneflow_traffic_verification::signal` 的独立许可判据（D1 严格不等式）
//! 裁决；本模块只负责 W 的域适配（相位公式与调度偏移），不重复持有判据。

use laneflow_static_contract::SignalAspect;

use crate::{cycle_ms, durations};

/// 冻结六阶段的调度偏移：W1/W2/W3 按 cell 索引错峰，W4 用同步信号边界。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SignalSchedule {
    /// `(cell * 997) % cycle`。
    CellOffset,
    /// `cycle - dt`。
    Synchronized,
}

/// 由冻结六阶段公式独立计算 `(cell, route)` 在 `tick` 拍初的预期灯色。
/// 两条观测路径（普通与 burst）共用本式，不调用 Runtime 的信号解释器。
pub(crate) fn aspect(
    schedule: SignalSchedule,
    cell: u32,
    route: u32,
    tick: u64,
    dt: u32,
) -> SignalAspect {
    let cycle = cycle_ms(dt);
    let offset = match schedule {
        SignalSchedule::CellOffset => u64::from(cell) * 997 % cycle,
        SignalSchedule::Synchronized => cycle - u64::from(dt),
    };
    let mut position = (tick * u64::from(dt) + offset) % cycle;
    let phase = durations(dt)
        .into_iter()
        .position(|duration| {
            if position < duration {
                true
            } else {
                position -= duration;
                false
            }
        })
        .expect("positive cycle");
    match (phase, route % 2) {
        (0, 0) | (3, 1) => SignalAspect::Green,
        (1, 0) | (4, 1) => SignalAspect::Yellow,
        _ => SignalAspect::Red,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_offset_phase_boundaries_match_frozen_six_phase_plan() {
        // 16 ms 与 33 ms 两档。相位时长按步长向上取整，边界 tick 从取整后的
        // durations 推出：green 最后一拍 position < durations[0]，yellow 首拍
        // position == durations[0]。W4 同步 offset 下 warm 拍为 Red、warm+1
        // 偶 route 转 Green。
        for dt in [16, 33] {
            let cycle = cycle_ms(dt);
            let warm = 4 * cycle / u64::from(dt);
            let first_phase = 30_000_u64.div_ceil(u64::from(dt)) * u64::from(dt);
            let green_last = (first_phase - 1) / u64::from(dt);
            let yellow_first = first_phase.div_ceil(u64::from(dt));
            assert_eq!(
                aspect(SignalSchedule::CellOffset, 0, 0, green_last, dt),
                SignalAspect::Green
            );
            assert_eq!(
                aspect(SignalSchedule::CellOffset, 0, 0, yellow_first, dt),
                SignalAspect::Yellow
            );
            for cell in [0_u32, 99, 999] {
                for route in 0..4_u32 {
                    assert_eq!(
                        aspect(SignalSchedule::Synchronized, cell, route, warm, dt),
                        SignalAspect::Red,
                        "dt={dt} cell={cell} route={route}"
                    );
                    assert_eq!(
                        aspect(SignalSchedule::Synchronized, cell, route, warm + 1, dt),
                        if route % 2 == 0 {
                            SignalAspect::Green
                        } else {
                            SignalAspect::Red
                        }
                    );
                }
            }
        }
    }
}
