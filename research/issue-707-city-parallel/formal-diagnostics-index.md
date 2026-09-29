# #707 D1/D2 与诊断证据索引

2026-09-21。本文件补充原 WP A/B/C pilot 交付；不关闭 #707，不调整正式预算。
来源根目录：`E:/projects/laneflow-evidence/issue-707/4de40e04/`。
源码身份：`4de40e045398e4b010b2aa36522afc02a4094c4d`。

仓库内的 [`formal-r1-evidence.json`](evidence/formal-r1-evidence.json)把四轮
execution ID、完整 plan/result/measurements/diagnostics 摘要、观察窗统计和
`result.json` 内全部大文件身份绑定为一条机器可读证据链；两份现场核查原始副本、
四份 diagnostics、两份跨 worker comparison 与 D1/D2 四轮电源边界原始记录也
提交在 `evidence/`。两份现场核查保持与外部来源逐字节一致，后验计划重放和电源
边界分析不回写原始副本。冻结 performance 计划的直接重放见
[`frozen-plan-replay.json`](evidence/frozen-plan-replay.json)。WPR trace、外部完整导出、
提交的隐私筛选紧凑报告与 `tar.zst` 容量实测见
[`wpr-trace-identities.json`](evidence/wpr-trace-identities.json)；M21 校准审计的可重建
源码、构建、输入、运行配置和四份结果见
[`calibration-audit-manifest.json`](evidence/calibration-audit-manifest.json)。M2 屏障跳过
的源码补丁、十次运行树身份与逐拍紧凑报告见
[`motion-barrier-evidence-manifest.json`](evidence/motion-barrier-evidence-manifest.json)。
原始逐拍文件和 ETL 当前只在作者机器的证据根现存，没有项目控制的持久链接，按未
长期留存处理；仓库不复制多 GB 日志。

## 正式 r1 的已完成范围

| 批次    | 暖机 / 观察拍数（每臂） | w1 / w4 Core p95 | 原预算          | 语义 / 预算结论                |
| ------- | ----------------------- | ---------------- | --------------- | ------------------------------ |
| D1 10k  | 30624 / 61248           | 7.89 / 5.33ms    | 2ms @16ms 步长  | performance-match / 两臂未通过 |
| D2 100k | 14848 / 29696           | 100.92 / 89.35ms | 16ms @33ms 步长 | performance-match / 两臂未通过 |

数据来自 `formal/d1-batch-summary.md`、`formal/d2-batch-summary.md` 与其指向的
`formal/{10k,100k}-w{1,4}-r1/measurements.toml` 观察窗；不是含暖机的 diagnostics
分位数。运行结束与研究/认证完成是不同状态。r2/r3 及同 worker 三轮聚合未完成。

D2 的 Active p50/p95/max 为 45468/53656/55372；这是总个体 100k 的混合生命周期
场景，不是稳定 100k Active。w4 Core p95 改善，同时 command p95 为 219.80ms
（w1 148.91ms）、observation p95 为 109.10ms（w1 88.10ms）；增量原因尚未分离。
不能将三项 p95 相加，不能将命令与观测变化归因成 worker 的必然效果。

D1 与 D2 的 w1/w4 均在单轮开始、结束边界记录
`Win32_Battery BatteryStatus=2`、电量 100%；LF 规范化提交副本为
`evidence/formal-power-{10k,100k}-w{1,4}.txt`，机器记录分别保留原始日志与提交
副本的身份及解析结果。这证明八个边界均接通 AC 且未放电；它不是运行中每一时刻
的连续遥测，也不把单轮 r1 扩大为最终性能认证。

D1 现场核查时间为 23:49:39，早于 w1 起点 23:50:58 共 79 秒，可作为起跑核查。
D2 现场核查时间为 03:05:00，晚于 w1 起点 03:01:13 共 227 秒，只能归类为运行中
观察，不能证明 D2 启动时的环境、负载和电源方案前置条件。D2 两臂完成、语义对照
及边界电源记录仍为有效单轮观察；缺失的启动时刻现场核查限制已进入机器记录，最终
认证保持开放。

## 来源定位

| 项目               | 根目录下路径                                                                        | 用途                                                 |
| ------------------ | ----------------------------------------------------------------------------------- | ---------------------------------------------------- |
| D1/D2 现场核查     | `manifest/d0-d1-precheck.toml`、`manifest/d0-d2-precheck.toml`                      | D1 为起跑前；D2 为首臂开始 227 秒后的运行中观察      |
| 完成与样本封套     | `formal/{scale}-w{workers}-r1/result.json`、`measurements.toml`、`diagnostics.json` | 完成拍数、观察窗、测量身份与原始样本                 |
| 原始语义轨迹       | 同目录 `ticks.jsonl`、`commands.jsonl`、`events.jsonl`                              | 逐拍、命令、事件顺序核验                             |
| 跨 worker 语义对照 | `comparisons/10k-r1-w1-w4.json`、`comparisons/100k-r1-w1-w4.json`                   | performance-match，不是三轮性能认证                  |
| 原始计划           | `plans/10k-performance.toml`、`plans/100k-performance.toml`                         | 冻结输入，短测仍读取原计划                           |
| 冻结计划重放       | 仓库内 `evidence/frozen-plan-replay.json`                                           | correctness/performance 四份均精确命中冻结摘要       |
| D1/D2 电源边界     | 仓库内 `evidence/formal-power-{10k,100k}-w{1,4}.txt`                                | 四轮开始/结束均为 AC、未放电、100%                   |
| D2 分段诊断        | `diagnostics/d2-offline-decomposition.md`                                           | 后段成本变化，解释范围依原文                         |
| L1                 | `diagnostics/wpr/l1-findings.md`、`l1-w4-prefix512.etl`                             | 早期机制筛查                                         |
| L3                 | `diagnostics/wpr/l3-w4-meta.txt`、`l3-w4-early.etl`、`l3-w4-late.etl`               | 同进程早晚采样，裁剪边界见 [L3 摘要](l3-findings.md) |
| WPR 不可变身份     | 仓库内 `evidence/wpr-trace-identities.json`                                         | B1/L3 ETL 与匹配导出的字节数和 SHA-256               |
| WPR 紧凑报告       | 仓库内 `evidence/wpr-b1/`、`evidence/wpr-l3/`                                       | 隐私筛选副本；原始 ETL/全系统导出未长期留存          |
| M2 屏障跳过        | 仓库内 `evidence/motion-barrier-*`                                                  | Rust/Cargo 源码补丁、十次运行树身份及 1,920 拍样本   |
| M21 校准审计       | 仓库内 `evidence/calibration-audit-*`、`evidence/calibration-*.json`                | 五文件源码增量、构建/输入/配置身份及四份结果         |

## ETL `tar.zst` 容量实测

使用 bsdtar 3.8.8 与 libzstd 1.5.7 默认压缩，每份 ETL 单独生成压缩流并通过标准输出
计数；没有落盘或修改制品文件。精确机器记录见
[`wpr-trace-identities.json`](evidence/wpr-trace-identities.json)。

| ETL      | 原始大小                        | `tar.zst` 大小                  | 节省       |
| -------- | ------------------------------- | ------------------------------- | ---------- |
| B1       | 1,311,768,576 B / 1.222 GiB     | 153,978,880 B / 146.846 MiB     | 88.26%     |
| L3 early | 1,154,482,176 B / 1.075 GiB     | 148,490,240 B / 141.611 MiB     | 87.14%     |
| L3 late  | 3,331,325,952 B / 3.103 GiB     | 426,741,760 B / 406.973 MiB     | 87.19%     |
| **合计** | **5,797,576,704 B / 5.399 GiB** | **729,210,880 B / 695.430 MiB** | **87.42%** |

三份 ETL 各自无需分片。结果仅覆盖 ETL；精确 EXE、PDB、manifest 与派生报告会增加
容量。因压缩包未落盘，当前没有压缩包 SHA-256；原 ETL 公开发布仍受隐私复核和 L3
原精确 EXE/PDB 缺失边界约束。

原始摘要和逐文件 SHA-256 仍以证据封套及文件为准；本索引不声称重新执行正式测量。
四轮 `measurements.toml` 均记录了 `Get-Process.PeakWorkingSet64`：10k w1/w4
分别为 239919104 / 239886336 bytes，100k w1/w4 分别为
2336194560 / 2336428032 bytes。`diagnostics.json` 的 `memory_measurement=null`
仅表示该 diagnostics 封套没有第二套内存字段；不能据此写成“未测”。现有数据
只是进程峰值驻留集，不等同资源记账证明。四基线分解、稳定 Active、资源成本及
完整重复协议仍需后续补足。先做[分层短测](short-profile-plan.md)，取得可消除成本
后再扩大正式复验。
