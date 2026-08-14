# Core 研究/测试仪器外移设计

**文档状态**: Review<br>
**最后更新**: 2026-08-14<br>
**适用范围**: `laneflow-core` 生产路径中内嵌的研究/测试仪器（Research/Test
Instrumentation）的分类定界、外移落点与仪器边界设计；本文是 #380 的
G1 冻结方案，冻结生效以 #380 Gate Ledger 的 G1 记录为准。本文引入的仪器类
术语以 `../reference/glossary.md` 的中文定义为权威<br>
**关联文档**:

- [`core-runtime-scalability-audit.md`](core-runtime-scalability-audit.md)（P1–P4 研究证据档案）
- [`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)（产品性能基线与优化队列）
- [`core-runtime.md`](core-runtime.md)
- [`../reference/glossary.md`](../reference/glossary.md)（仪器类术语权威定义）
- [`../governance/development-gates.md`](../governance/development-gates.md)
- [Issue #380](https://github.com/illusion-tech/laneflow/issues/380)

## 1. 背景与事实修正

#380 架构审查发现研究/测试仪器深嵌 Core 生产状态机。经 main（`6486c864`）核实，
Issue 核心指控成立，事实细节按当前代码修正：

- `CoreWorld` 持有 3 个 `#[cfg(test)]` 字段（非 4 个）：`reduced_rate_research`、
  `step_failure_after_vehicle`、`replace_failure_after_prepare`。
- `step()`、`rebuild_longitudinal_motions` 与生命周期命令路径散布约 20 处
  `#[cfg(test)]` 研究分支与 5 处 `Instant::now()` 计时钩子；生产函数
  `reduced_rate_motion_for_research` 的签名直接引用测试模块类型
  `reduced_rate_research_tests::ReducedRateResearchState`。
- 6 个研究/白盒测试模块经 `#[path]` 挂入生产模块树，合计 7,811 行；core src
  测试代码占比约 43.6%（编译视图口径）。
- benches 与 tests 之间为单向耦合（非双向）：全部 7 个 bench 经 `#[path]`
  反向挂载 `tests/support/` 夹具（8 个文件 4,836 行），tests 不依赖 benches。
- `occupancy.rs` 原指控内容已大部分不存在，现仅余 `retained_bytes` 与
  `set_max_vehicle_length_for_research` 两个 `#[cfg(test)]` seam。

## 2. 仪器分类与目的

调研结论：上述内容不是单一性质的技术债，而是四种目的不同、生命周期状态不同的
机制的混合体。外移方案必须按类分治，统一迁出会误伤长期基础设施。

| 类别 | 内容 | 目的 | 证据链 / 契约 | 生命周期 |
| --- | --- | --- | --- | --- |
| A 语义研究原型 | 4 个 `*_research_tests.rs` 模块（#204/#207/#210/#212）、`CoreWorld.reduced_rate_research` 字段、`reduced_rate_motion_for_research` Active 分叉、`longitudinal.rs` / `occupancy.rs` 的 `*_for_research` seam | 验证替代架构（事件归并、分区 occupancy、选择性读取、降频更新）能否与单线程 production oracle 逐 bit 精确等价 | `core-runtime-scalability-audit.md` §6 P1–P4 证据档案 | 全部 G4 收官，结论均为"采纳为研究证据，拒绝为生产架构"；已登记为未来工作的证据根（#220 merge contract ← P1，halo/dependency 约束 ← P2，#218 semantic oracle ← P4） |
| B 性能归因计时 | 六段纳秒计时（occupancy、longitudinal 总计、longitudinal proposal、projection、post-longitudinal、research-commit）与 Criterion release 取证 | whole-step 机制归因，产出 audit D5 归因表 | baseline §10.1 优化队列（#216–#219 靶点数字来源）；baseline §8.4 契约"coarse stage timing 只用于机制归因" | 活跃：#216–#218 的取证入口；audit §6 已记录一次内嵌计时生命周期 bug 导致整轮归因作废重跑 |
| C 故障注入 | `step_failure_after_vehicle`、`replace_failure_after_prepare` | failed-step 失败原子性产品护栏 | baseline §7.1 零容忍硬不变量、§4.3 逐字点名、W3/W4 产品 Gate 指定仪器；#186 原子 replace 契约 | 长期产品门禁，非研究遗留 |
| D retained memory 账本 | `retained_bytes` 系列、`CompleteRetainedComponents` 20 组件穷尽解构账本 | 产品认证容量证据 + 常规 PR 回归门禁 | baseline §5/§8.3/§11；`../reference/rust-code-style.md` §5 常规 PR smoke | 长期测试基础设施，非研究遗留 |

另有一项组织问题：benches → `tests/support/` 的单向 `#[path]` 耦合（夹具共享
没有正式落点）。

## 3. 冻结决策

原则：生产路径与生产类型只保留两类正式接缝——仪器探针边界（instrumentation
probe boundary，承接 B 类）与测试支持接缝（test-support seam，承接 C/D 类及
research 访问）；研究原型代码一律移出生产 crate。

- **A 类 → 归档 `research/`**。四个研究模块迁移为 `research/issue-<n>-*` 独立
  workspace 成员包，沿用 #123/#308 既有惯例：`publish = false`、生产 crate 不得
  依赖、复现命令入包 README、研究结论继续以 `docs/design` 档案为准。生产模块树
  摘除全部 `#[path]` 挂载；移除 `CoreWorld.reduced_rate_research` 字段、
  `reduced_rate_motion_for_research` Active 分叉与全部 `*_for_research` seam。
  未来 #218 / #216 重启时在 research 包内重建 oracle，不得再次嵌回生产路径。
  归档的可编译性由 **research 访问边界**承接：编译期门控的只读公共表面，提供
  occupancy/leader oracle 快照、运动浮点位模式快照与 scratch 元数据等价物
  （属测试支持接缝的一部分，具体形状由 G2 定稿）。逐研究的访问需求与处置：
  P1（#204 事件归并）所需顺序与信号状态基本可从公开 API 派生，迁移成本最低；
  P3（#210 选择性读取）依赖公开接口、C 类故障注入接缝与位模式快照；P2（#207
  分区 occupancy）需要 oracle 快照表面；P4（#212 降频）的实验本质是替换生产
  运动路径并内嵌事务 cache，无法外置为独立 crate 的等价可运行形态，按 #380
  允许的降级路径归档为 provenance——保留源码、证据与最后可运行 commit 记录，
  其语义 oracle 由 #218 未来在新边界上重建。该降级项须在 #380 记录并拆后续
  Issue。
- **B 类 → 仪器探针边界**。生产路径持有空操作（no-op）默认的探针：为让默认
  发布构建与无仪器构建等价，探针采用静态分发（泛型 / unit 类型），空操作实现
  整体编译消除；不采用在热路径保留运行期分支的 `Option<&mut dyn …>` 形态——
  这是 G1 冻结约束，不留待 G2 取舍。六段计时与 `ReducedRateMetrics` 指标收集
  收敛为探针方法；Criterion 取证迁移为探针实现。默认发布构建零成本、零行为
  变更；audit D5 归因能力与其复现命令等价物完整保留并回写
  `core-runtime-scalability-audit.md`。探针边界同时消除已发生过的内嵌计时
  生命周期 bug 事故面。
- **C 类 → 正式测试支持接缝**。故障注入检查点必须编译期门控（crate feature /
  `cfg`），不得进入默认发布构建——这是 G1 冻结约束，不留待 G2 取舍；
  `#[doc(hidden)]` 只允许用于公共 setter 的文档可见性控制，不构成构建排除。
  baseline §4.3/§7.1 点名的 W3/W4 产品 Gate 验证路径保持不变。
- **D 类 → 同 C 类**。retained 账本经编译期门控的测试支持接缝获得正式身份；
  穷尽解构设计（新增字段编译失败强制分类）与常规 PR smoke、一万/十万 matrix
  验证语义不变。
- **benches 解耦**。`tests/support/` 抽为独立 support 包（workspace 私有成员包
  或既有 `tests/common` 模式的推广，由 G2 定稿），benches 与 tests 各自正常
  依赖，消除交叉 `#[path]`。

## 4. Core API 影响评估

- 零公开行为变更；tick 确定性语义与全部硬不变量不受影响（#380 非目标保持）。
- 预期新增三类公共表面，均为编译期门控的 API 新增（新增而非既有行为变更）：
  仪器探针边界（探针 trait 与空操作类型）、测试支持接缝（故障注入 setter 与
  retained 账本入口）、research 访问边界（oracle / 位模式 / scratch 元数据
  快照）。在此冻结记录；属实现组织调整，不新增 ADR。
- 三类表面的精确类型清单由 G2 定稿并回本节追加记录（append-only）；超出上述
  范围的公共类型不得静默引入。

## 5. 确定性与验证不变量

- `cargo test --workspace --locked` 全量通过。
- 迁移后的研究测试保留等价断言与可复现证据；`--release --ignored` 取证入口
  在归档包内保持可运行（P4 按 §3 降级路径除外，已在 #380 记录）。
- 既有性能基线与 benches 不因外移失效；tick 路径行为不变，确定性不变量不受
  影响。

## 6. 与后续研究 Issue 的关系

- **#216（occupancy/leader exact path）**：将重新开工（2026-08-14 用户决策）。
  其当前在制品不纳入本方案协调范围；A/B 类外移时直接清理旧挂载点与计时钩子，
  新仪器探针边界由 #216 重启时沿用。
- **#217 / #218**：尚未开工。其未来仪器需求（同类阶段归因计时、#212 semantic
  oracle）由本方案的仪器探针边界与 research 归档包覆盖，不得再次嵌入生产路径。
- **#220**：消费 P1/P2 已冻结的语义约束，不直接使用仪器，不受本方案影响。

## 7. 交付切片建议

按风险递增排列为四个 Related PR 切片（③ 依赖 ② 的探针替代先就位，否则
reduced-rate 证据链断裂）：

1. C/D 类测试支持接缝化：移除 2 个故障注入字段，retained 账本正式化。
2. B 类仪器探针边界：移除 5 处热路径计时钩子与 `ReducedRateMetrics` 内嵌收集。
3. A 类归档 `research/`：先建 research 访问边界，再摘除 `#[path]` 挂载并迁移
   四模块（P4 按 §3 降级路径归档为 provenance），移除 reduced-rate Active 分叉
   与 `*_for_research` seam。
4. `tests/support/` 抽离与 benches 解耦。

## 8. 非目标

沿用 #380：不改变任何公开 Core API 行为与 tick 确定性语义；不删除研究测量能力
本身（P4 降级项按 §3 在 #380 记录并拆后续）；不做 CoreWorld 命令域拆分（由
#381 承担，建议在本 Issue 之后）；不改动 Data/Spatial/Scenario/Bevy crate。
