# 未武装迁移日志的有限生产对照

本切片对应 #618 / PR #619，只前移 P7 车辆提交中的日志存在性判断：未武装时不做
日志专用状态比较、路线定位或 `VehicleDelta` 构造。状态写回、空 tick、资源增量、
溢出后的世界推进及切换合同保持不变；不修改公开 API、格式、依赖或状态布局。

`from_state` 消费的不变量由受检路线与 P6 Waiting/Conflict 派生保障，不新增全量验证。
依据为 `traffic-runtime-phase-protocol.md`、`traffic-runtime-revision-cutover.md` 和
ADR 0020/0021/0025；G1 N/A，无需修改长期设计。

## 结论

**未证明稳定的整步提速，不按性能收益采纳或入队。** 原 CPU 画像中约 1.52% 的
叶栈权重不是本补丁的加速比。下面是 12 对独立进程轮次的整步平均时间配对变化：

| 输入（车辆数 / 边数） | 日志   | 时间变化 | 描述性 95% 区间  |
| --------------------- | ------ | -------- | ---------------- |
| 1000 / 256            | 未武装 | +1.93%   | [-0.98%, +4.78%] |
| 10000 / 256           | 未武装 | +0.73%   | [-4.04%, +6.20%] |
| 10000 / 16            | 未武装 | -1.72%   | [-4.11%, +0.93%] |
| 10000 / 256           | 武装   | -0.12%   | [-4.70%, +4.73%] |

负数表示更快；所有区间跨零，不能据此宣称性能等效或排除小幅回归。候选只保留供
私有实现清理评审，不追加扩测或开启其他热点；#219 等父任务未因此完成。

## 验证与方法

- 新回归先在旧逻辑失败；修复后未武装/武装/解除武装的变化 tick 物化次数为 0/1/0。
  空 tick 和溢出路径保持；Waiting/Conflict 各 64 拍武装/未武装快照与三类输出相等。
- Runtime release 测试：449 passed、0 failed、29 ignored；手动 A/B 与分配入口另跑。
  release check、fmt、Runtime 架构检查和生产库严格 Clippy 通过。
  全测试目标严格 Clippy 的五项既有告警已在 #617 原树复现，未混入无关清理。
- 四个道路输入使用 100 ms 步长，车辆始终为 Individual / Active / Intent，
  Presented / Aggregate 为 0。每世界暖机 32 + 8 拍，再观测固定 64 拍。
- 每进程另有两个预定排除但保留的暖机窗口；正式窗口为 128 个（1000 车辆）或
  16 个（10000 车辆）。共 96 进程、270336 拍正式样本，另保留 12288 拍暖机样本。
  AB/BA 平衡、输入顺序轮转，不剔除轮次；计时不含建世界、校验、摘要与输出。
- 主指标为独立进程平均时长的配对比值几何平均；以 12 对轮次整体 bootstrap
  10000 次（seed 618），不把逐拍视为独立实验。全部摘要与日志预算校验通过。
- 分配另用独立二进制：两版六组现有输入均暖态零分配/重分配，摘要及日志字节数一致。
- 同机 Rust 1.98.0、默认 release、Windows MSVC；AMD Ryzen 9 9955HX，平衡电源计划，
  未锁频/绑核，无 ETW 或同时 Cargo 构建。结论不构成跨机器保证；未跑 100k 矩阵。

## 复现

基线 `6ae9cddc03714a487903cb4a4a36ce5a6e315f4e`，
候选 `e45e48cd561817db5af1123a7dda7e0acd27c08a`；两者的共同测试入口一致。
分别构建并将 EXE/PDB 保存在不同目录，然后运行：

```powershell
cargo +1.98.0 test --release --locked --offline -p laneflow-runtime --test runtime_profile_evidence --test runtime_profile_allocation --no-run
pwsh -NoLogo -NoProfile -NonInteractive -File research/issue-618-unarmed-journal/run-ab.ps1 -Baseline '<baseline-exe>' -Candidate '<candidate-exe>' -OutputDirectory 'target/issue-618/new-run'
python research/issue-618-unarmed-journal/analyze.py 'target/issue-618/new-run' 'target/issue-618/new-analysis'
```

原始进程日志、逐拍样本、完整统计、源码/二进制指纹和测试日志属于生成制品，不入库。
本次原件保留在运行机器的 `target/issue-618/`，既有导出副本在本目录被忽略的
`evidence/` 中；从其他机器的新 checkout 复算历史批次需要先取得这份本地制品。
分析器拒绝缺失进程或不一致窗口；已用完整原始批次验证可重复计算。
