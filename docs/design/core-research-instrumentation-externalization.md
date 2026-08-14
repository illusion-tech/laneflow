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
  反向挂载 `tests/support/` 夹具（8 个文件 4,762 行），tests 不依赖 benches。
- `occupancy.rs` 原指控内容已大部分不存在，现仅余 `retained_bytes` 与
  `set_max_vehicle_length_for_research` 两个 `#[cfg(test)]` seam。

## 2. 仪器分类与目的

调研结论：上述内容不是单一性质的技术债，而是四种目的不同、生命周期状态不同的
机制的混合体。外移方案必须按类分治，统一迁出会误伤长期基础设施。

| 类别           | 内容                                                                                                                                                                                                        | 目的                                                                                                                                                   | 证据链 / 契约                                                                                             | 生命周期                                                                                                                                                           |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| A 语义研究原型 | 4 个 `*_research_tests.rs` 模块（#204/#207/#210/#212）、`CoreWorld.reduced_rate_research` 字段、`reduced_rate_motion_for_research` Active 分叉、`longitudinal.rs` / `occupancy.rs` 的 `*_for_research` seam | 验证替代架构（事件归并、分区 occupancy、选择性读取、降频更新）能否与单线程 production oracle 精确等价、或在显式保真度预算内有界等价（P4 降频采用后者） | `core-runtime-scalability-audit.md` §6 P1–P4 证据档案                                                     | 全部 G4 收官，结论均为"采纳为研究证据，拒绝为生产架构"；已登记为未来工作的证据根（#220 merge contract ← P1，halo/dependency 约束 ← P2，#218 semantic oracle ← P4） |
| B 性能归因计时 | 六段纳秒计时（occupancy、longitudinal 总计、longitudinal proposal、projection、post-longitudinal、research-commit）与 Criterion release 取证                                                                | whole-step 机制归因，产出 audit D5 归因表                                                                                                              | baseline §10.1 优化队列（#216–#219 靶点数字来源）；baseline §8.4 契约"coarse stage timing 只用于机制归因" | 活跃：#216–#218 的取证入口；audit §6 已记录一次内嵌计时生命周期 bug 导致整轮归因作废重跑                                                                           |
| C 故障注入     | `step_failure_after_vehicle`、`replace_failure_after_prepare`                                                                                                                                               | failed-step 失败原子性产品护栏                                                                                                                         | baseline §7.1 零容忍硬不变量、§4.3 逐字点名、W3/W4 产品 Gate 指定仪器；#186 原子 replace 契约             | 长期产品门禁，非研究遗留                                                                                                                                           |
| D 保留内存账本 | `retained_bytes` 系列、`CompleteRetainedComponents` 20 组件穷尽解构账本                                                                                                                                     | 产品认证容量证据 + 常规 PR 回归门禁                                                                                                                    | baseline §5/§8.3/§11；`../reference/rust-code-style.md` §5 常规 PR smoke                                  | 长期测试基础设施，非研究遗留                                                                                                                                       |

另有一项组织问题：benches → `tests/support/` 的单向 `#[path]` 耦合（夹具共享
没有正式落点）。

## 3. 冻结决策

原则：生产路径与生产类型只保留两类正式接缝——仪器探针边界（instrumentation
probe boundary，承接 B 类）与测试支持接缝（test-support seam，承接 C 类故障
注入）；保留内存账本保持 crate-private 内聚；研究原型按访问需求分流——四项
研究均依赖 crate-private 访问或同 crate 编译期守卫，保留为 crate 内常规
`#[cfg(test)]` 模块（P1/P2/P3）或降级为 provenance（P4）；本切片不向生产
crate 外迁出研究模块，不新增面向研究访问的公共表面。

- **A 类 → 按访问需求逐项冻结落点**。四项研究的落点由访问需求决定，全部摘除
  `#[path]` 挂载，生产模块树只保留常规模块声明：

| 模块                      | 访问需求                                                                                                                                                                                                                                                     | 落点                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1 事件归并（#204）       | 顺序与信号状态可从公开 API 派生，但穷尽映射守卫依赖同 crate match：`CoreEvent` 为 `#[non_exhaustive]`，外置会迫使研究映射加通配臂而静默吞掉新 variant                                                                                                        | crate 内常规 `#[cfg(test)]` 模块，穷尽守卫原样保留                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| P2 分区 occupancy（#207） | 构造与变更 crate-private `OccupancyScratch` / `Occupant` 候选（`begin` / `insert` / `sort_edges`），并调用私有 oracle 方法（`leader_horizon` / `find_leader`）；只读快照不覆盖这些候选构建操作                                                               | crate 内常规 `#[cfg(test)]` 模块，等价断言零损失、零新公共表面                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| P3 选择性读取（#210）     | stable-hash 候选选择与 slot 索引 delta 追踪依赖 crate-private `VehicleHandle::index` / `generation`；pose 映射对 `#[non_exhaustive] VehicleStatus` 的穷尽匹配守卫依赖同 crate 编译，外置会迫使替换候选算法并加通配臂，改变已记录证据语义并静默吞掉新 variant | crate 内常规 `#[cfg(test)]` 模块，等价断言与穷尽守卫零损失、零新公共表面                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| P4 降频（#212）           | 替换生产运动路径并内嵌事务 cache，无法外置为独立 crate 的等价可运行形态                                                                                                                                                                                      | provenance 降级：保留源码、证据与最后可运行 commit 记录；语义 oracle 由 #218 在新边界重建；降级同时覆盖整个 D5 reduced-rate 对比的可运行复现——D5 表 H1/C4 行的全部列（whole/occupancy/proposal/projection/longitudinal/post/research-commit）都由 `release_100k_three_round_step_stage_diagnostics` 经 `world_for_performance_case` / `enable_reduced_rate` 构造并读自 `CoreWorld.reduced_rate_research`，kernel 诊断命令（`reduced-rate-ablation` controller-only 与 motion benchmark 输入）依赖 P4 独占的 `evaluate_controller_intent_for_research`；P4 状态机、Active 分叉与这些 seam 移除后 H1/C4 workload 本身不复存在，生产路径探针无法保全该对比或其复现命令。降级已记录于 #380 评论，后续由既有 Issue #218 承担 |

  crate 内保留的研究/白盒模块（P1、P2、P3，以及行为白盒 `occupancy_tests` 与
  D 类 `retained_memory_tests`）一律改为常规 `#[cfg(test)]` 模块声明，不再经
  `#[path]` 挂入；生产类型与热路径不含其任何字段、分支或签名引用。
  `CoreWorld` 的 `reduced_rate_research` 字段与 `reduced_rate_motion_for_research`
  Active 分叉随之移除。`*_for_research` seam 按使用者分流：仅服务 P4 的
  `evaluate_controller_intent_for_research` 与
  `compute_motion_from_controller_intent_for_research` 随降级移除；P2 依赖的
  4 个 `#[cfg(test)] pub(crate)` seam（`LongitudinalMotion::leader_for_research`
  / `float_bits_for_research`、`LongitudinalScratch::reset_geometry_projection_for_research`、
  `OccupancyScratch::set_max_vehicle_length_for_research`）保留——其底层字段为
  兄弟模块私有，crate 内任何单一测试落点都无法直接访问，移除将使 P2 等价
  断言不可运行；这 4 个 seam 本身即编译期排除、crate-private、零公共表面的
  测试支持形态，不违背本节任何冻结约束。#216 / #218 重启时的 oracle 重建
  不得再次嵌回生产路径；未来确需外置的研究 harness 所需访问边界在各自
  Issue 的 G1 冻结，本方案不预设。
- **B 类 → 仪器探针边界**。生产路径持有空操作（no-op）默认的探针，六段计时
  与 `ReducedRateMetrics` 指标收集收敛为探针方法；Criterion 取证迁移为探针
  实现。探针契约完整保留六段方法（含 research-commit，供未来 #218 等研究
  harness 注入探针实现时恢复测量）；D5 的 reduced-rate 对比（H1/C4 行全部
  列）与 kernel 诊断命令的可运行复现随 P4 降级整体暂停（见 A 类降级记录），
  生产路径探针在 P0 workload 上的五段计时（occupancy、longitudinal 总计
  / proposal / projection、post-longitudinal）复现能力完整保留。默认发布
  构建零成本、零行为变更；audit D5 已记录数值按历史证据语义保留，暂停范围
  与探针替代边界由实施 PR 回写 `core-runtime-scalability-audit.md`。探针边界同时消除已发生过的内嵌计时
  生命周期 bug 事故面。探针的形状级决策在此冻结：以 method-generic 入口承载
  （泛型 / unit 空操作类型，空操作实现整体编译消除），不参数化 `CoreWorld`，
  现有 `CoreWorld` 与 `step` 公开签名保持不变；不采用在热路径保留运行期分支
  的 `Option<&mut dyn …>` 形态。
- **C 类 → 正式测试支持接缝**。故障注入检查点必须编译期门控（crate feature /
  `cfg`），不得进入默认发布构建——这是 G1 冻结约束，不留待 G2 取舍；
  `#[doc(hidden)]` 只允许用于公共 setter 的文档可见性控制，不构成构建排除。
  baseline §4.3/§7.1 点名的 W3/W4 产品 Gate 验证路径保持不变。
- **D 类 → 内聚为常规 `cfg(test)` 模块**。保留内存账本维持 crate-private、
  `cfg(test)`（`../reference/rust-code-style.md` §5 规则保持不变），不新增任何
  公共表面或 crate feature；外移收敛为去 `#[path]` 化——`retained_bytes` 计账
  方法、`CompleteRetainedComponents` 与对应测试迁入按常规模块声明的
  `#[cfg(test)]` 子模块，生产函数与热路径不再持有这些定义。穷尽解构设计
  （新增字段编译失败强制分类）与常规 PR smoke、一万/十万 matrix 验证语义不变。
- **benches 解耦**。`tests/support/` 抽为独立 support 包（workspace 私有成员包，
  `publish = false`），benches 与 tests 经 dev-dependencies 正常依赖，消除交叉
  `#[path]`。不采用推广既有 `tests/common` 模式的备选：每个 integration test
  与 bench 都是独立 crate，bench 中的 `mod common` 解析到 `benches/` 下而非
  `tests/common/`，该模式只能重新引入 `#[path]` 或复制夹具，无法兑现正常
  依赖。包名与 API 签名由 G2 在上述冻结包络内定稿。

## 4. Core API 影响评估

- 零公开行为变更；tick 确定性语义与全部硬不变量不受影响（#380 非目标保持）。
- 冻结 crate feature 集合：`instrumentation`（B 类探针的研究态实现）与
  `test-support`（C 类故障注入）；默认构建均不启用。D 类保留内存账本保持
  crate-private `cfg(test)`，不占用任何 feature 或公共表面。
- 冻结形状级 API 决策：探针为 method-generic 入口、不参数化 `CoreWorld`；
  `CoreWorld` / `step` 现有公开签名不变；两类公共表面（仪器探针边界与测试
  支持接缝）的可见性最小化，建议 `#[doc(hidden)]`。
- 两类表面的精确类型名与签名清单由 G2 在上述冻结包络内定稿，回本节
  append-only 追加；超出包络的变更（参数化 `CoreWorld`、新增 feature、默认
  启用任一 feature、保留内存账本改公开）属设计变更，须重新 G1，不得静默引入。

## 5. 确定性与验证不变量

- `cargo test --workspace --locked` 全量通过。
- 迁移与内聚后的研究/白盒测试保留等价断言与可复现证据：P1/P2/P3 在 crate 内
  `#[cfg(test)]` 模块中原样可运行（含 P3 的 `--release --ignored` 取证入口）；
  P4 按 §3 降级路径除外——含整个 D5 reduced-rate 对比（H1/C4 行全部列与
  kernel 诊断命令）的可运行复现，已记录于 #380 评论。
- 既有性能基线与 benches 不因外移失效；tick 路径行为不变，确定性不变量不受
  影响。
- 构建级验收（实施 Gate，§3 冻结的两个构建属性由此证明）：
  - 隔离的 `cargo build -p laneflow-core --release`（默认 feature）+ 符号缺席
    检查，证明测试支持接缝不编译进发布二进制；
  - no-op 探针的 codegen 检查（符号 / 反汇编对比），证明空操作实现整体编译
    消除；
  - feature matrix（默认 / `instrumentation` / `test-support` / 全启用）逐一
    构建并测试。
  - 上述验证必须在隔离包上下文进行：`--workspace` 统一构建的 Cargo feature
    统一可能让默认构建实际启用接缝 feature，从而掩盖违背。

## 6. 与后续研究 Issue 的协调契约

本节只记录稳定的依赖与协调契约；各 Issue 的开工 / 重启状态以其 GitHub Issue
为准，不在本文镜像。

- **#216（occupancy/leader exact path）**：其 G2 指名的在制研究模块沿用旧嵌合
  模式；本方案外移 A/B 类时有权直接清理这些挂载点与计时钩子。#216 重启后的
  研究 harness 须沿用仪器探针边界，不得再次嵌入生产路径；如需 crate 外访问
  表面，在其自身 G1 冻结。
- **#217 / #218**：其仪器需求（同类阶段归因计时、#212 semantic oracle 重建）
  由本方案的仪器探针边界与 crate 内研究模块模式覆盖，不得再次嵌入生产路径。
- **#220**：消费 P1/P2 已冻结的语义约束，不直接使用仪器，不受本方案影响。

## 7. 交付切片建议

按风险递增排列为四个 Related PR 切片（③ 依赖 ② 的探针替代先就位，否则
reduced-rate 证据链断裂）：

1. C 类测试支持接缝化 + D 类内聚：移除 2 个故障注入字段（迁入编译期门控
   接缝），保留内存账本迁入常规 `#[cfg(test)]` 模块并去 `#[path]`。
2. B 类仪器探针边界：移除 5 处热路径计时钩子与 `ReducedRateMetrics` 内嵌收集。
3. A 类逐项落点：摘除全部 `#[path]` 挂载；P1/P2/P3 与两个白盒模块
   （`occupancy_tests`、`retained_memory_tests`）改为常规 `#[cfg(test)]` 声明；
   P4 降级为 provenance；移除 reduced-rate Active 分叉与 P4 独占的
   `*_for_research` seam（P2 依赖的 4 个 `#[cfg(test)] pub(crate)` seam 保留）。
4. `tests/support/` 抽离与 benches 解耦。

## 8. 非目标

沿用 #380：不改变任何公开 Core API 行为与 tick 确定性语义；不删除研究测量能力
本身（P4 降级项按 §3 在 #380 记录并拆后续）；不做 CoreWorld 命令域拆分（由
#381 承担，建议在本 Issue 之后）；不改动 Data/Spatial/Scenario/Bevy crate。
