# LF-SYNTH 合同校准与小样本验证

日期：2026-10-29。关联 #1148，供 #539 的 LF-SYNTH 输入交付线使用。

## 结论与边界

修订后的 `LF-SYNTH-v2` 交通输入在 1/2 cell、16/33 ms 下通过现行生产准入与运行。
本切片只交付合同和 Rust 测试：没有正式规模生成器、manifest 编码、Spatial 几何、
表现选择、外层帧调度、产品计时或五轴认证。一万/十万仍为 `Product TBD / Uncertified`。

基线提交为 `445dcf16c604d9db96507aeb0e440b15b9ab8197`。测试源位于
[`lf_synth_contract.rs`](../../crates/laneflow-runtime/src/kernel/tests/lf_synth_contract.rs)
及其 [`fixture.rs`](../../crates/laneflow-runtime/src/kernel/tests/lf_synth_contract/fixture.rs)。
验收代码身份以包含本报告的提交为准；没有使用未提交的生产补丁。

样本通过 `SyntheticModuleBuilder → Compiler → LFCA/后发射检查 → SharedNetworkRevision
→ TrafficWorld::install` 构造，每 cell 保留 4 路线、8 边、4 Gate、1 信号控制器、
100 个车辆槽位及 100 个实体停车位。采用单 worker、显式 `ProtectedGroup` 策略。
Spatial 被显式省略，样本 provenance 中的固定摘要仅用于测试安装，不是正式输入文件的
认证摘要。正式生成器必须另行冻结规范编码、stable order 和输入摘要测试向量。

## 合同修订

- 每阶段按 `ceil(nominal_ms / delta_ms) * delta_ms` 离散化，周期分别为
  58032/58113 ms；W4 offset 分别为 58016/58080 ms。W1–W3 的 cell offset
  按实际周期取模。Runtime 本身不取整。
- 密集车队保留 6.5 m 间距，初速改为 0。旧 1 m/s 输入的第二辆车在升序生成时
  被 `UnsafeFollower` 拒绝，逆序被 `UnsafeLeader` 拒绝；失败状态不变。
- lifecycle probe 改为路线终点 10260 m、零速生成；新 handle 初态为 Active，
  下一成功拍自然成为 Completed，下一 boundary 执行 atomic replacement。
  旧“距终点 0.1 m、13.9 m/s”被 `StopConstraintUnsatisfiable` 拒绝。
- 停车映射到 `spawn_parked_vehicle`、`reserve_parking`、`park_vehicle`、
  `leave_parking`；完成观测由宿主比较状态生成，不虚构 Runtime 完成事件 API。
- 失败注入使用现有 test-only `StepFailpoint::AfterTransitions`；它是整拍提交前
  失败点。失败不回滚已成功的前置命令，也不发布本拍状态或事件；重试只重做 step。
- 输入身份包含 workload、scale、case、fixed delta 和 seed；输入文件的 SHA-256
  与运行时 `deterministic_state_digest` 分别使用，符合 ADR 0033 的用途划分。

长期权威为 [`core-runtime-performance-baseline.md` §4](../../docs/design/core-runtime-performance-baseline.md#4-canonical-workload)。
旧 ID 仅由 Git 历史保存，不交付兼容路径。

## 复现与结果

```powershell
cargo +1.99.1 test --release --locked -p laneflow-runtime --lib lf_synth_contract -- --nocapture
```

5 项测试全部通过，其中完整运行矩阵为 4 cases × 2 cell 数 × 2 固定步长。
信号检查通过 stable ID 查回每个 group，对每条成功 step 的已提交相位按独立时间公式核对，
覆盖 cell 1 的非步长整数倍 offset 和 W4 同步 offset。

| case | cell 数 | 16 ms 成功拍数 | 33 ms 成功拍数 | 核验结果                                                                                     |
| ---- | ------- | -------------: | -------------: | -------------------------------------------------------------------------------------------- |
| W1   | 1、2    |          43524 |          21132 | 每 cell 始终 75 Active / 25 Parked，无 Completed；停车绑定与命令游标不变                     |
| W2   | 1、2    |          43524 |          21132 | 每 cell 始终 100 Active，无 Completed；命令游标不变                                          |
| W3   | 1、2    |          43524 |          21132 | 无预热到达；每 cell 恰好一次到达；单拍 Parked barrier 后下一 boundary 离场；生命周期替换通过 |
| W4   | 1、2    |          14510 |           7046 | B-1 完成，B0 同步绿灯与替换/离场，B1 指定车辆跨 Gate；最终每 cell 76 Active / 24 Parked      |

W3 parking commit 的 observation index 为 8492（16 ms）和 4108（33 ms），从 0 开始；
这是本样本的实际回执，不是后继 harness 的硬编码输入。1/2 cell 的结果相同。
W4 的 B0/B1 为世界成功拍序号 14509/14510 或 7045/7046。

W3/W4 均使用独立安装并从初始化开始重放全部输入的第二个世界：每拍比较 step outcome
与事件批次，在失败边界及末尾比较规范快照、运行时摘要、handle 和信号批次。
失败前后的规范快照、摘要、已发布事件和命令游标相同；retry 不重复前置命令，并与
无注入世界得到相同结果。此处只报告语义结果，不使用测试耗时推导产品性能。

补充回归：`cargo +1.99.1 test --release --locked -p laneflow-runtime --lib` 通过
638 项、按既有标记忽略 20 项；去掉 `--release` 的 5 项合同测试也全部通过。
`cargo +1.99.1 clippy --locked -p laneflow-runtime --all-targets -- -D warnings`
通过。未在本地运行全 workspace 测试，跨 crate 的完整
检查由 PR CI 执行；本次没有修改生产 Runtime 代码或依赖。

## 后继交付

本 PR 合入后可据此开展正式生成器：100/1000 cells、完整 Spatial 几何、规范输入文件、
manifest/digest 和独立重复生成逐字节比较。执行 harness、表现/外层帧与五轴认证仍需
独立切片；#539 不因这组小样本通过而关闭。
