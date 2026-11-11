# 当前能力与限制

**文档状态**: Active<br>
**最后核对**: 2026-11-11，基线
[`2d0c07070fbe6543f8597b0093322ab301881d5b`][capabilities-commit]<br>
**适用范围**: 道路机动车特化的 `TrafficWorld`，以及按源码集成的宿主

本页是当前实现与限制的阅读入口，不替代所链专题合同，也不扩大公开支持承诺。
当前可运行执行域是道路机动车 `TrafficWorld`。设计被接受、实现已存在、历史验收通过
和产品认证是不同事实。具体格式版本、容量与执行阈值以所链合同和代码常量为准；
计划进度请查 GitHub。

## 阅读方式与状态

每条先给出实现或设计状态，再写使用条件。两层状态不必互斥：

- **实现 / 设计**：`已实现`，或 `已接受但未实现`。已接受只说明设计输入，不是实现回执。尚未冻结的目标不标成已接受。
- **证据**：`已验收且绑定基线` 必须写明提交和覆盖范围。没有这种绑定的结论保持 `未认证`。历史验收集中在文末。

明确不支持、已知缺口和保守拒绝是补充说明，不另排一条状态流水线。后继缺口只按已登记、尚无关闭修复证明来引用。
首次 Alpha 的范围、稳定性、平台、MSRV 和 feature 支持矩阵不由本页决定
（[#1327][issue-1327]、[#1329][issue-1329]）。

## 当前能力

### 编译与加载

**已实现**：正式来源经过编译和后发射检查，得到受检 LFCA，再构建共享修订。发布加载先验证 LFCP 绑定。

- **条件与边界**：检查、绑定或资源限额失败时停在当前阶段。Runtime 不解析 JSON，也不直接安装 LFCA。
  道路编辑来源和可移植对象各有自己的版本轴。P100 初始配置档的已知单次发射限额只描述该档
  （[#1288][issue-1288]），不能当成所有档的最大路网规模。按容量推导限额仍是建议设计
  （[#1296][issue-1296]）。本页不复制该数字，也不另立格式版本表。这不是容量或性能认证。
- **依据**：[后发射检查 §4][post-emission-api]、
  [§5][post-emission-order]、
  [§7][post-emission-lifecycle]、
  [§8][post-emission-lfcp]；
  [共享静态路网 §3][shared-static-network-admission]。格式细节见
  [`portable-canonical-artifact.md`][portable-artifact]。
  [`check_portable_candidate`][mod-rs] 只是检查委托，不是完整检查实现。

### 车辆行为与路权

**已实现**：固定步进下的道路机动车跟驰、固定时制信号，以及 Waiting 与 Conflict 的联合裁决。世界安装时绑定完整路权策略。

- **条件与边界**：生成和替换要满足路线后缀的静态准入和放置条件。停止约束仍可能在一拍内把速度收到零
  （[#1272][issue-1272]）。感应信号和自适应信号不是当前能力。
  舒适制动和物理平顺没有据此通过。
- **依据**：[路权策略 §3][right-of-way-signals]、
  [§5][right-of-way-binding]、
  [§6][right-of-way-lifecycle]；
  [跟车 §6][vehicle-following-leader]、
  [§8][vehicle-following-longitudinal]、
  [§10.2][vehicle-following-emergency]；[信号 §1][signal-system-goals]、
  [§5][signal-system-timing]；
  [横断面与准入 §6.5][cross-section-access-runtime]。生成与替换的拒绝见
  [`SpawnError` / `ReplaceError`][error-rs]。

### 停车

**已实现**：静态设施、显式泊位、虚拟容量和锚点由共享修订拥有；预约、占用和生命周期由世界拥有。

- **条件与边界**：reserve、park、leave，以及 cancel、rebind、despawn，受状态、精确绑定、锚点、路线匹配和安全插入约束。
  没有位姿的停驻车辆仍然保留身份。容量耗尽、锚点不属于设施、入口已经越过、尚未到达、离场重叠或后车不安全时可以拒绝。
  这些命令路径没有带来性能认证。
- **依据**：[停车 §2][parking-system-boundary]、[§4][parking-system-resources]、
  [§5][parking-system-commands]、[§5.7][parking-system-errors]、
  [§8][parking-system-spatial]。拒绝种类见
  [`ParkingError`][error-rs]。

### 固定预构造换道

**已实现**：只执行已经编制好的固定过渡材料。合同中的名称是「固定预制过渡」。

- **条件与边界**：路线编译会拒绝一部分互不可见的共横向过渡段；同向相邻或纵向分开的组合也可能被保守拒绝。
  已登记且尚无关闭修复证明的缺口有三项：平行边直行车对过渡仍可能不可见
  （[#1271][issue-1271]）；未使用的公共后继可能绕过现有注册拒绝
  （[#1321][issue-1321]）；循环路线可能把近端制动边界选错
  （[#1322][issue-1322]）。回归 `straight_vehicle_on_parallel_edge_is_a_registered_gap`
  记录的是 #1271 的缺口。#1321 与 #1322 的反例目前只登记在 Issue 中。这些拒绝没有构成完整安全保证，任意位置变道也不是这项能力。
- **依据**：[最小换道合同 §3][lane-change-contract-a1]、
  [§5][lane-change-contract-a3]、
  [§6][lane-change-contract-a6]、
  [§9][lane-change-contract-a5]、
  [§10][lane-change-contract-limits]。缺口回归见
  [`transition_visibility.rs`][transition-visibility-rs]。

### 快照与恢复

**已实现**：可以按固定步长、修订和来源绑定保存并恢复世界。执行配置由宿主另外提供。

- **条件与边界**：恢复核对语义容量和完整性，容量不能缩小。活动过渡，以及当前格式不能忠实表达的过渡预约或历史，以
  `UnsupportedTransitionState` 结构化拒绝，世界可以继续运行。同修订的完整过渡持久化尚未交付
  （[#1200][issue-1200]）。最小换道合同 §9 中的版本升级是后续格式设计。生产版本以
  `SNAPSHOT_FORMAT_VERSION`、`RUNTIME_STATE_VERSION` 和 `RUNTIME_STATE_DIGEST_VERSION` 为准，本页不另立版本表。
- **依据**：[快照 §2][runtime-snapshot-versions]、
  [§5][runtime-snapshot-restore]、[§6][runtime-snapshot-replay]、
  [§9][runtime-snapshot-g2]。常量与过渡准入门见
  [`snapshot.rs`][snapshot-rs]；摘要版本常量在 `snapshot_digest.rs`。

### 修订切换

**已实现**：候选在事务里追赶。只有成功提交才发布目标修订和切换事件。

- **条件与边界**：映射、预算或等价证明失败时，旧世界继续，没有部分发布。过渡相关迁移的拒绝比「当前有车正在换道」更宽，
  受结构化拒绝限制，详见合同与 [#1217][issue-1217]。
  含过渡资源的各切换模式没有在这里逐项归纳，触发范围以合同和该 Issue 为准。
- **依据**：[切换 §3][revision-cutover-strategy]、
  [§4][revision-cutover-state-machine]、
  [§7][revision-cutover-promotion]、
  [§8][revision-cutover-faults]；
  [快照 §7][runtime-snapshot-migration]。迁移拒绝见
  [`cutover_migration.rs`][cutover-migration-rs] 的
  `CutoverError::UnsupportedTransitionState`。

### 观测

**已实现**：从已提交状态导出 full、delta 或 partition。session 绑定世界、世代、修订和选择。

- **条件与边界**：新 session 的第一次导出必须是 full。绑定失效、选择非法，以及容量、算术或分配失败，都返回 `ObservationError`。
  不同逻辑时点的分区不能拼成一批。这不是交通工程分析产品。
- **依据**：[观测与 Routing §2][routing-integration-observation]、
  [§3][routing-integration-cost]、
  [§5][routing-integration-cutover]。入口见
  [`open_observation_export`][observation-rs]。

### Routing 接入

**已实现**：宿主用观测构造动态成本并生成候选，Runtime 验证后注册。固定步进里不做全图寻路。Routing 算法由宿主拥有。

- **条件与边界**：成本、世界、修订和模型身份，有效窗，稳定边身份与连通性，以及路线和出现记录容量，都参与准入。摘要不授予拓扑信任。
  过期、来自未来、模型或修订错配都会拒绝。设计 §7.1 的旧 Routing G2 fixture 仍登记为不能按该入口执行
  （[#1288][issue-1288]），尚无关闭修复证明；注册接缝本身仍是已实现能力。
  §7.2 与 §7.3 是另外的负载和范围。城市负载的验收边界见文末。
- **依据**：[观测与 Routing §1][routing-integration-boundary]、
  [§4][routing-integration-register]、
  [§4.1][routing-integration-capacity]、
  [§7.1][routing-integration-descriptive]、
  [§7.2][routing-integration-deterministic]、
  [§7.3][routing-integration-wall-clock]。
  注册见 [`register_candidate_route`][routing-rs]。出现记录容量的字段是
  `WorldConfig::route_edge_occurrence_capacity`。

### 单世界并行

**已实现**：非零 worker 配置受后端上限约束。单个 worker 在调用线程执行；多个 worker 由该世界独占的池执行。

- **条件与边界**：P4 仍按规范串行。同步 `step` 返回前收拢任务。超出支持范围、资源预留失败、线程创建失败或计划失败都会拒绝，
  不会静默减少 worker。worker 数不进入快照或逻辑摘要。同一环境下的并行等价，不推出跨版本或跨平台的全部一致。城市性能未认证。
  上限以 `MAX_SUPPORTED_WORKERS` 为准，本页不抄数字。
- **依据**：[执行配置 §2][execution-config-api]、
  [§4.3][execution-config-errors]、
  [§6][execution-config-format]；
  [并行 §1][parallel-execution-equivalence]、
  [§3][parallel-execution-phases]、
  [§4][parallel-execution-interfaces]、
  [§6][parallel-execution-p4]。上限与校验见
  [`config.rs`][config-rs] 的 `MAX_SUPPORTED_WORKERS` 和
  `ExecutionConfig::validate_supported`。

### Spatial

**已实现，且可选**：`SpatialSession` 绑定共享修订，消费 Runtime 已提交的位姿来源，不裁决交通路权。

- **条件与边界**：修订没有 Spatial 组件时，`SpatialSession::bind` 返回 `Ok(None)`。有 Spatial 组件但缺少 lane-pose 时，绑定失败
  （`MissingLanePose`）。几何使用有界的 canonical `f32`。交通进度和距离仍以整数毫米为正本。
- **依据**：[空间几何 §1.1][spatial-geometry-delivered]、
  [§6][spatial-geometry-length]、[§7][spatial-geometry-sampling]；
  [Adapter API][adapter-api]；[架构 §3][architecture-crates]。绑定见
  [`session.rs`][session-rs]。

### Bevy 接入

**已实现**：当前参考适配器是 Bevy 0.20。宿主负责 schedule、生命周期和实体表现。这一实现不构成对更多引擎的支持承诺。

- **条件与边界**：提供 Spatial 时，必须与世界满足同一根 `Arc::ptr_eq`，失配则拒绝。批量提取完成之后才发布 Transform。示例规模、
  截图和有界性能报告不是城市认证。平台、MSRV 和 feature 的公开支持范围不在本页给定
  （[#1329][issue-1329]）。
- **依据**：[Bevy §2][bevy-adapter-toolchain]、
  [§3][bevy-adapter-session]、
  [§4][bevy-adapter-schedule]、[§5][bevy-adapter-entities]、
  [§6][bevy-adapter-frame]、
  [§7][bevy-adapter-extract]、
  [§16][bevy-adapter-cutover]；
  [`laneflow-bevy` README][laneflow-bevy-readme]。会话构造见
  [`LaneFlowSession::new`][session-rs-2]。

## 已接受但未实现与明确边界

- 时变准入的运行时求值已接受但未实现。`FacilityBand` 作为 `AccessRule` 目标也已接受但未实现；编译器当前拒绝这一目标
  （`AccessCapability::FacilityBandTarget`）。本页不把 `TimeWindows` 写成现行生产拒绝守卫。见
  [横断面与准入 §6.2][cross-section-access-target]、
  [§6.5][cross-section-access-runtime] 和
  [`hir/access.rs`][access-rs]。
- 任意位置变道是后续能力目标。固定预制过渡不是该目标的等价实现。提前制动
  （[#1272][issue-1272]）仍待设计与实施；立项本身不是已接受的设计，也不是已经完成的行为。
- 非机动车、行人、轨道执行域，感应或自适应信号，以及其他引擎的正式适配，都不是现行能力。`ParticipantClass` 只是准入分类，
  不表示这些执行域已经实现。
- Routing 算法、出行需求、出发计划和最终表现由宿主拥有。LaneFlow 不提供专业交通工程分析。这是职责边界。

## 历史验收与认证边界

v0.10 的 [#305][issue-305] 综合结论与
[#306][issue-306] 独立审阅绑定 `c0045f8f`，不是当前 main 的新认证。
LF-SYNTH 和城市负载的实际覆盖及授权范围见 [路线图验收摘要][roadmap-v010] 与原始回执；performance、
resource 均为未认证，城市 Routing 获授权豁免，LuST 五轴未验证。关闭任务或里程碑不改变这些边界。历史性能和资源测量只能按原提交、
输入、硬件与执行配置解释，不能升级为当前能力承诺。

原始回执：[#305 综合结论][issue-305-summary]、
[维护者裁定与更正][issue-305-correction]、
[#306 最终复核][issue-306-review]。

下面两项只限制核查或证据的区分能力。它们是已登记、尚无关闭修复证明的限制，不构成新的复现结论，也不撤销已经绑定基线的验收：

- [#1318][issue-1318]：L1 核查标签尚未绑定完整输入身份。
  这个缺口限制标签能区分的输入范围。本页不据此宣称历史正式记录挂错输入，也不把 clean 标签提升成任意参数都已核查。
- [#1323][issue-1323]：LF-SYNTH 换道事件证据里，多簇过渡资源身份可能被折成同一个常量。
  这个缺口限制证据能否分出真实过渡身份。本页不据此宣称 Runtime 发错事件，也不否定已有的 F1、F2、F3 回执。

[capabilities-commit]: https://github.com/illusion-tech/laneflow/commit/2d0c07070fbe6543f8597b0093322ab301881d5b
[issue-1327]: https://github.com/illusion-tech/laneflow/issues/1327
[issue-1329]: https://github.com/illusion-tech/laneflow/issues/1329
[issue-1288]: https://github.com/illusion-tech/laneflow/issues/1288
[issue-1296]: https://github.com/illusion-tech/laneflow/issues/1296
[post-emission-api]: ../design/compiler-post-emission-check-and-minimal-publication-closure.md#4-公共检查-api
[post-emission-order]: ../design/compiler-post-emission-check-and-minimal-publication-closure.md#5-检查顺序
[post-emission-lifecycle]: ../design/compiler-post-emission-check-and-minimal-publication-closure.md#7-compiler-候选与消费生命周期
[post-emission-lfcp]: ../design/compiler-post-emission-check-and-minimal-publication-closure.md#8-lfcp-v2
[shared-static-network-admission]: ../design/shared-static-network.md#3-构建输入与-admission
[portable-artifact]: ../design/portable-canonical-artifact.md
[mod-rs]: ../../crates/laneflow-compiler/src/portable_publication/mod.rs
[issue-1272]: https://github.com/illusion-tech/laneflow/issues/1272
[right-of-way-signals]: ../design/traffic-runtime-right-of-way-policy.md#3-灯态解释与禁令
[right-of-way-binding]: ../design/traffic-runtime-right-of-way-policy.md#5-世界和场景绑定
[right-of-way-lifecycle]: ../design/traffic-runtime-right-of-way-policy.md#6-仲裁持久状态与生命周期
[vehicle-following-leader]: ../design/vehicle-following.md#6-leader-detection-与-route-relative-distance
[vehicle-following-longitudinal]: ../design/vehicle-following.md#8-longitudinal-constraints
[vehicle-following-emergency]: ../design/vehicle-following.md#102-emergency-safe-speed
[signal-system-goals]: ../design/signal-system.md#1-目标状态与非目标
[signal-system-timing]: ../design/signal-system.md#5-fixed-tick-phase-timing
[cross-section-access-runtime]: ../design/cross-section-access.md#65-运行时集成边界
[error-rs]: ../../crates/laneflow-runtime/src/kernel/error.rs
[parking-system-boundary]: ../design/parking-system.md#2-产品边界
[parking-system-resources]: ../design/parking-system.md#4-运行时资源与唯一规范依据
[parking-system-commands]: ../design/parking-system.md#5-生命周期命令
[parking-system-errors]: ../design/parking-system.md#57-幂等和错误分类
[parking-system-spatial]: ../design/parking-system.md#8-spatial-与-adapter
[issue-1271]: https://github.com/illusion-tech/laneflow/issues/1271
[issue-1321]: https://github.com/illusion-tech/laneflow/issues/1321
[issue-1322]: https://github.com/illusion-tech/laneflow/issues/1322
[lane-change-contract-a1]: ../design/lane-change-minimal-contract.md#3-承载形式裁定-a1-回写
[lane-change-contract-a3]: ../design/lane-change-minimal-contract.md#5-同拍联合求解与预约约束裁定-a3a4-回写核心
[lane-change-contract-a6]: ../design/lane-change-minimal-contract.md#6-能力检查裁定-a6-选项-3-回写
[lane-change-contract-a5]: ../design/lane-change-minimal-contract.md#9-持久化与版本轴裁定-a5规范定义见-snapshot-衔接注记
[lane-change-contract-limits]: ../design/lane-change-minimal-contract.md#10-排除能力限制与缺口
[transition-visibility-rs]: ../../crates/laneflow-runtime/src/kernel/tests/transition_visibility.rs
[issue-1200]: https://github.com/illusion-tech/laneflow/issues/1200
[runtime-snapshot-versions]: ../design/traffic-runtime-snapshot.md#2-版本轴与绑定集
[runtime-snapshot-restore]: ../design/traffic-runtime-snapshot.md#5-保存与恢复
[runtime-snapshot-replay]: ../design/traffic-runtime-snapshot.md#6-回放与确定性状态摘要
[runtime-snapshot-g2]: ../design/traffic-runtime-snapshot.md#9-runtime-snapshot-v7-的-g2-边界与必测义务
[snapshot-rs]: ../../crates/laneflow-runtime/src/admin/snapshot.rs
[issue-1217]: https://github.com/illusion-tech/laneflow/issues/1217
[revision-cutover-strategy]: ../design/traffic-runtime-revision-cutover.md#3-迁移策略封闭种类选择器
[revision-cutover-state-machine]: ../design/traffic-runtime-revision-cutover.md#4-状态机
[revision-cutover-promotion]: ../design/traffic-runtime-revision-cutover.md#7-原子晋升与共存
[revision-cutover-faults]: ../design/traffic-runtime-revision-cutover.md#8-失败关闭故障映射
[runtime-snapshot-migration]: ../design/traffic-runtime-snapshot.md#7-跨修订迁移入口
[cutover-migration-rs]: ../../crates/laneflow-runtime/src/admin/cutover_migration.rs
[routing-integration-observation]: ../design/traffic-observation-and-routing-integration.md#2-已提交交通观测
[routing-integration-cost]: ../design/traffic-observation-and-routing-integration.md#3-动态成本快照
[routing-integration-cutover]: ../design/traffic-observation-and-routing-integration.md#5-与-302-切换快照和回放的接缝
[observation-rs]: ../../crates/laneflow-runtime/src/facade/observation.rs
[routing-integration-boundary]: ../design/traffic-observation-and-routing-integration.md#1-产品选择与正本边界
[routing-integration-register]: ../design/traffic-observation-and-routing-integration.md#4-候选路线注册
[routing-integration-capacity]: ../design/traffic-observation-and-routing-integration.md#41-统一路线边出现记录容量
[routing-integration-descriptive]: ../design/traffic-observation-and-routing-integration.md#71-lf-routing-g2-linear-v1-首轮描述性证据
[routing-integration-deterministic]: ../design/traffic-observation-and-routing-integration.md#72-lf-routing-candidate-lfsynth-v1-首轮确定性证据1267
[routing-integration-wall-clock]: ../design/traffic-observation-and-routing-integration.md#73-lf-routing-candidate-lfsynth-v1-墙钟与-tick-间隔干扰1279
[routing-rs]: ../../crates/laneflow-runtime/src/facade/routing.rs
[execution-config-api]: ../design/traffic-runtime-execution-config.md#2-公开-api
[execution-config-errors]: ../design/traffic-runtime-execution-config.md#43-错误类型
[execution-config-format]: ../design/traffic-runtime-execution-config.md#6-格式与版本轴
[parallel-execution-equivalence]: ../design/traffic-runtime-parallel-execution.md#1-等价范围
[parallel-execution-phases]: ../design/traffic-runtime-parallel-execution.md#3-首版阶段图与写入者
[parallel-execution-interfaces]: ../design/traffic-runtime-parallel-execution.md#4-独立计算与规范消费接口
[parallel-execution-p4]: ../design/traffic-runtime-parallel-execution.md#6-p4-规范串行联合裁决
[config-rs]: ../../crates/laneflow-runtime/src/kernel/config.rs
[spatial-geometry-delivered]: ../design/spatial-geometry.md#11-291300-共享空间路网已交付
[spatial-geometry-length]: ../design/spatial-geometry.md#6-长度正本与绑定
[spatial-geometry-sampling]: ../design/spatial-geometry.md#7-采样语义
[adapter-api]: ../design/adapter-api.md
[architecture-crates]: ../architecture.md#3-crate-依赖与所有权
[session-rs]: ../../crates/laneflow-spatial/src/session.rs
[bevy-adapter-toolchain]: ../design/bevy-reference-adapter.md#2-版本工具链与依赖边界
[bevy-adapter-session]: ../design/bevy-reference-adapter.md#3-最终裁决职责与-session-边界
[bevy-adapter-schedule]: ../design/bevy-reference-adapter.md#4-fixed-schedule
[bevy-adapter-entities]: ../design/bevy-reference-adapter.md#5-vehicle-与-entity-映射
[bevy-adapter-frame]: ../design/bevy-reference-adapter.md#6-canonical-frame-与-bevy-transform
[bevy-adapter-extract]: ../design/bevy-reference-adapter.md#7-批量提取与原子-transform-提交
[bevy-adapter-cutover]: ../design/bevy-reference-adapter.md#16-修订绑定生命周期与维护暂停式切换534
[laneflow-bevy-readme]: ../../crates/laneflow-bevy/README.md
[session-rs-2]: ../../crates/laneflow-bevy/src/session.rs
[cross-section-access-target]: ../design/cross-section-access.md#62-target-平面与展开
[access-rs]: ../../crates/laneflow-compiler/src/hir/access.rs
[issue-305]: https://github.com/illusion-tech/laneflow/issues/305
[issue-306]: https://github.com/illusion-tech/laneflow/issues/306
[roadmap-v010]: ../roadmap.md#v010-验收结论
[issue-305-summary]: https://github.com/illusion-tech/laneflow/issues/305#issuecomment-6707563062
[issue-305-correction]: https://github.com/illusion-tech/laneflow/issues/305#issuecomment-6708446002
[issue-306-review]: https://github.com/illusion-tech/laneflow/issues/306#issuecomment-6729858464
[issue-1318]: https://github.com/illusion-tech/laneflow/issues/1318
[issue-1323]: https://github.com/illusion-tech/laneflow/issues/1323
