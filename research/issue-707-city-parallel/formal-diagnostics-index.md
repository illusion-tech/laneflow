# #707 D1/D2 与诊断证据索引

2026-09-21。本文件补充原 WP A/B/C pilot 交付；不关闭 #707，不调整正式预算。
来源根目录：`E:/projects/laneflow-evidence/issue-707/4de40e04/`。
源码身份：`4de40e045398e4b010b2aa36522afc02a4094c4d`。

## 正式 r1 的已完成范围

| 批次 | 暖机 / 观察拍数（每臂） | w1 / w4 Core p95 | 原预算 | 语义 / 预算结论 |
| --- | --- | --- | --- | --- |
| D1 10k | 30624 / 61248 | 7.89 / 5.33ms | 2ms @16ms 步长 | performance-match / 两臂未通过 |
| D2 100k | 14848 / 29696 | 100.92 / 89.35ms | 16ms @33ms 步长 | performance-match / 两臂未通过 |

数据来自 `formal/d1-batch-summary.md`、`formal/d2-batch-summary.md` 与其指向的
`formal/{10k,100k}-w{1,4}-r1/measurements.toml` 观察窗；不是含暖机的 diagnostics
分位数。运行结束与研究/认证完成是不同状态。r2/r3 及同 worker 三轮聚合未完成。

D2 的 Active p50/p95/max 为 45468/53656/55372；这是总个体 100k 的混合生命周期
场景，不是稳定 100k Active。w4 Core p95 改善，同时 command p95 为 219.80ms
（w1 148.91ms）、observation p95 为 109.10ms（w1 88.10ms）；增量原因尚未分离。
不能将三项 p95 相加，不能将命令与观测变化归因成 worker 的必然效果。

## 来源定位

| 项目 | 根目录下路径 | 用途 |
| --- | --- | --- |
| D1/D2 起跑核查 | `manifest/d0-d1-precheck.toml`、`manifest/d0-d2-precheck.toml` | 硬件/电源/源码/二进制/输入前置条件 |
| 完成与样本封套 | `formal/{scale}-w{workers}-r1/result.json`、`measurements.toml`、`diagnostics.json` | 完成拍数、观察窗、测量身份与原始样本 |
| 原始语义轨迹 | 同目录 `ticks.jsonl`、`commands.jsonl`、`events.jsonl` | 逐拍、命令、事件顺序核验 |
| 跨 worker 语义对照 | `comparisons/10k-r1-w1-w4.json`、`comparisons/100k-r1-w1-w4.json` | performance-match，不是三轮性能认证 |
| 原始计划 | `plans/10k-performance.toml`、`plans/100k-performance.toml` | 冻结输入，短测仍读取原计划 |
| D2 分段诊断 | `diagnostics/d2-offline-decomposition.md` | 后段成本变化，解释范围依原文 |
| L1 | `diagnostics/wpr/l1-findings.md`、`l1-w4-prefix512.etl` | 早期机制筛查 |
| L3 | `diagnostics/wpr/l3-w4-meta.txt`、`l3-w4-early.etl`、`l3-w4-late.etl` | 同进程早晚采样，裁剪边界见 [L3 摘要](l3-findings.md) |

原始摘要和逐文件 SHA-256 仍以证据封套及文件为准；本索引不声称重新执行正式测量。
内存测量未提供，不能填 0。四基线分解、稳定 Active、资源成本及完整重复协议仍需
后续补足。先做[分层短测](short-profile-plan.md)，取得可消除成本后再扩大正式复验。
