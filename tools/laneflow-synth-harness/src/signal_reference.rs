//! 独立信号许可参考判定：只接收拍初事实（灯色、已提交速度、整数距离、
//! 受检 profile 紧急减速度），不调用 Runtime 的门决策、黄灯困境区豁免或
//! 停止求解器，也不以其结果为输入。判定式与
//! `docs/design/lf-synth-execution.md` 第 1 节（`lf-synth-execution-2`）
//! 一致：绿灯允许；红灯拒绝；黄灯按 D1 困境区严格不等式
//! `v_before² > 2·b_e·d_before` 判定——等号属于可停车，拒绝过门。
//! 采用整数交叉乘积，不复制 Runtime 的 f32 单位转换舍入，不添加 epsilon；
//! 参考式与 Runtime 数值实现的分歧保留为失败并按实现问题处理，不以舍入
//! 对齐抹平。

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

/// 信号侧过门许可的独立证明；许可原因保留供 W4 专项（B1 必须为 Green）
/// 与失败诊断。
// 提交 A 只交付参考模块与边界测试；两 observer 的许可接线在提交 B 落地，
// 届时移除本属性。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrossingPermit {
    Green,
    YellowDilemma,
}

/// 信号侧过门拒绝原因；黄灯可停携带整数比较双方（`v²` 与 `2·b·d`），
/// 供失败信息直接核对。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrossingDenial {
    Red,
    YellowCanStop {
        speed_squared: u128,
        emergency_distance_product: u128,
    },
    UnsupportedAspect,
}

/// 拍初信号许可的纯参考判定。输入全部是事实：拍初已提交灯色与速度、
/// 拍初整数进度到停止线的毫米距离、受检 profile 的紧急减速度（mm/s²）。
/// `SignalAspect` 是 `#[non_exhaustive]`，未知灯色明确拒绝。
#[allow(dead_code)]
pub(crate) fn signal_crossing_permission(
    aspect_before: SignalAspect,
    speed_before_mm_s: u32,
    distance_before_mm: u32,
    emergency_decel_mm_s2: u64,
) -> Result<CrossingPermit, CrossingDenial> {
    match aspect_before {
        SignalAspect::Green => Ok(CrossingPermit::Green),
        SignalAspect::Red => Err(CrossingDenial::Red),
        SignalAspect::Yellow => {
            let v = u128::from(speed_before_mm_s);
            let d = u128::from(distance_before_mm);
            let b = u128::from(emergency_decel_mm_s2);
            let speed_squared = v * v;
            let emergency_distance_product = 2 * b * d;
            if speed_squared > emergency_distance_product {
                Ok(CrossingPermit::YellowDilemma)
            } else {
                Err(CrossingDenial::YellowCanStop {
                    speed_squared,
                    emergency_distance_product,
                })
            }
        }
        _ => Err(CrossingDenial::UnsupportedAspect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMERGENCY: u64 = 6_000;

    #[test]
    fn green_permits_and_red_denies_regardless_of_stopping_ability() {
        assert_eq!(
            signal_crossing_permission(SignalAspect::Green, 0, 0, EMERGENCY),
            Ok(CrossingPermit::Green)
        );
        // 红灯是当前 W 族 ProtectedGroup 输入域内的结论：即使完全不可停车
        // 也拒绝（困境区豁免只作用于黄灯本身）。
        assert_eq!(
            signal_crossing_permission(SignalAspect::Red, 13_900, 0, EMERGENCY),
            Err(CrossingDenial::Red)
        );
    }

    #[test]
    fn yellow_boundary_is_strict_and_equality_denies() {
        // v=6000、b=6000：v²=36_000_000；2bd=12_000·d。d=3001 拒、d=3000
        // 等号拒、d=2999 允许。
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 6_000, 3_001, EMERGENCY),
            Err(CrossingDenial::YellowCanStop {
                speed_squared: 36_000_000,
                emergency_distance_product: 36_012_000,
            })
        );
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 6_000, 3_000, EMERGENCY),
            Err(CrossingDenial::YellowCanStop {
                speed_squared: 36_000_000,
                emergency_distance_product: 36_000_000,
            })
        );
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 6_000, 2_999, EMERGENCY),
            Ok(CrossingPermit::YellowDilemma)
        );
    }

    #[test]
    fn f32_sensitive_boundaries_follow_exact_integer_comparison() {
        // Runtime 的 f32 表达式在这两个状态上与精确整数比较相反（#1455 复核）：
        // 600/30 在 f32 下等号被舍入成大于（允许），精确式拒绝；
        // 4751/1881 在 f32 下差 1 被舍入吞掉（拒绝），精确式允许。
        // 参考判定不复制 Runtime 的舍入来消除差异。
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 600, 30, EMERGENCY),
            Err(CrossingDenial::YellowCanStop {
                speed_squared: 360_000,
                emergency_distance_product: 360_000,
            })
        );
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 4_751, 1_881, EMERGENCY),
            Ok(CrossingPermit::YellowDilemma)
        );
    }

    #[test]
    fn zero_values_do_not_constitute_dilemma() {
        // v=0、d=0：v²=0 不大于 0，不构成困境区。
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 0, 0, EMERGENCY),
            Err(CrossingDenial::YellowCanStop {
                speed_squared: 0,
                emergency_distance_product: 0,
            })
        );
        // 正速度且 d=0（已越过停止线）：按当前 D1 公式判定为允许。
        assert_eq!(
            signal_crossing_permission(SignalAspect::Yellow, 1, 0, EMERGENCY),
            Ok(CrossingPermit::YellowDilemma)
        );
    }

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

    #[test]
    fn permission_uses_tick_start_speed_not_post_step_values() {
        // 构造拍初与拍末速度导出相反结论的状态：许可只看拍初速度。
        let before = 6_000_u32;
        let after = 0_u32;
        let distance = 2_999_u32;
        let permitted =
            signal_crossing_permission(SignalAspect::Yellow, before, distance, EMERGENCY).unwrap();
        assert_eq!(permitted, CrossingPermit::YellowDilemma);
        assert!(
            signal_crossing_permission(SignalAspect::Yellow, after, distance, EMERGENCY).is_err(),
            "拍末速度不得参与判定"
        );
    }
}
