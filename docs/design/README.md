# 设计文档

本目录回答“系统具体怎么做”。`docs/adr/` 回答“为什么这样定”。GitHub 记录当前
任务与评审；这里只保存仍约束实现的长期设计。已关闭任务的完成记录、验证轮次和
截图不进本目录，见 git 历史。

跨领域的当前能力与限制先读[当前能力与限制][capabilities]；规范细节仍以下列专题为准。

## 当前可运行路径

- [`traffic-runtime-shared-consumption.md`][shared-consumption]：
  `TrafficWorld` 安装共享静态路网，Spatial 只 bind 同一根 `Arc`。路线入口是
  `register_route`（ADR 0029）。worker 范围见下方单世界并行设计。
- [`traffic-runtime-phase-protocol.md`][traffic-runtime-phase-protocol]：串行 P0～P8、
  内部状态五分区、窄视图与 `CommitPlan`。该文定义阶段顺序，不单独证明并行执行；
  worker 范围见下方单世界并行设计。
- [`traffic-runtime-module-boundary.md`][traffic-runtime-module-boundary]：仿真内核、
  管理平面与 facade 的私有模块边界、LFSD/LFRS 唯一格式入口，以及 xtask 架构检查。
- [`retire-precompiled-static-route.md`][retire-precompiled-static-route]：路网制品不声明路线；
  场景 catalog 0.4 拥有示例边序列（ADR 0029）。
- [`traffic-runtime-revision-cutover.md`][revision-cutover]：在线修订切换事务、
  切换描述符、封闭迁移策略、迁移增量日志与失败关闭预算。#302 G1 已接受，切换事务已实现。
  含活动过渡的世界拒绝跨修订迁移；该限制不是性能认证。
- [`traffic-runtime-snapshot.md`][runtime-snapshot]：版本化运行时快照容器、保存/恢复合同、
  回放与跨修订迁移入口。#302 G1 已接受。当前已实现 Runtime Snapshot v7
  （LFRS `formatVersion = 7`、runtime state 5、确定性状态摘要版本 8 / BLAKE3-256）。
  活动过渡的保存被结构化拒绝，版本轴 7→8 尚未实施。
- [`traffic-runtime-state-integrity.md`][state-integrity]：运行时状态完整性校验与失真恢复——Runtime 侧只在
  投入使用前与宿主审计时认证共享路网修订（不进 tick 路径），C(T) 由宿主审计重放覆盖；停用 +
  宿主处置的恢复裁决权边界、漏检面、遥测与认证成本口径
  （#955 G1；配套 [ADR 0032][adr-0032]）。
- [`traffic-observation-and-routing-integration.md`][routing-integration]：
  已提交交通观测 full/delta/partition、宿主自有 Routing 成本绑定、候选路线注册与 #302 失效接缝。
  #303 G1 已接受，观测导出与候选路线注册已实现；独立性能验收不在本索引宣称完成。
- [`traffic-runtime-conflict-occurrence.md`][conflict-occurrence]：#283 的路线冲突出现记录、
  route-local 坐标、独立容量与正式 Conflict authority 的活动车辆边界。
- [`traffic-runtime-waiting-zone.md`][traffic-runtime-waiting-zone]：#282 G1 Accepted 的 WaitingZone 本地
  membership/admission/storage/queue 设计，以及当前 LFRS 7、runtime state 5、
  deterministic digest 8 合同；#284 的组合 ledger 与持久化由下列路权文档管理。
- [`shared-static-network.md`][shared-static-network]：从受检 LFCA 构建 `SharedNetworkRevision`。
- [`committed-road-editing-state.md`][committed-editing]：committed `RoadEditingState` 的形状与所属层级、
  `CommittedNetworkSource` 编辑来源变体、`EditableDiffBase` 对应关系、宿主重编译的信任
  边界与快照编辑来源绑定（#1116 G1；实现见 #1119，认证分支与端到端验证见 #529）。
- [`adapter-api.md`][adapter-api]：Runtime / Spatial 与引擎适配器的只读快照、位姿和正本边界。
- [`committed-pose-extraction.md`][committed-pose-extraction]：#681 位姿来源借用、Spatial 缓冲交换与按需提取
  的已接受设计；实现状态以独立任务为准。
- [`portable-canonical-artifact.md`][portable-artifact]：统一 LFCA 5 / LFSM 4 / LFSD 4 /
  LFCP 2、确定性分块与单路网一百万现实混合静态实体容量合同。
- [`compiler-post-emission-check-and-minimal-publication-closure.md`][post-emission]：
  后发射检查与 LFCP v2 最小发布闭合。
- [`compiler-foundation.md`][compiler-foundation]：编译器 crate、Typed AST → HIR → MIR → Canonical LIR、
  合成 DSL 与官方前端接入。
- [`network-compiler.md`][network-compiler]：#291 目标静态编译架构。
- [`road-editing-source-and-geometry-frontend.md`][road-editing]：
  道路编辑 FlatBuffers v4 来源与几何编制前端合同。

## 单世界并行设计

- [`traffic-runtime-parallel-execution.md`][parallel-execution]：Accepted；
  首版独立计算并行、P4 规范串行、首错/join/重试、完整提交与成本验收。
- [`traffic-runtime-execution-config.md`][execution-config]：Accepted；
  配置分离 API、世界执行资源、候选计划、安装/恢复/切换与 LFRS 版本轴。

两文配套 [ADR 0030][adr-0030]（Accepted）。
配置分离与 worker 1–16 已实现（#705，现行 LFRS 7 不保存 worker）。这是执行能力，不是城市性能认证；
#707 的记录不外推为当前 main 认证。

- [`traffic-runtime-columnar-execution.md`][columnar-execution]：[ADR 0031][adr-0031] 接受的列式运动正本、共享基础输入、完整纵向 SIMD 和稀疏资源控制目标；实施与性能结论以 #814 的验收为准，本索引不把它写成已认证。
- [`traffic-runtime-scale-decision-baseline.md`][scale-baseline]：Active；#72 城市规模扩展的决策入口，含现行接口复核、证据边界、方向裁定与资源预算口径。

## 领域规则

这些文档约束 Runtime 仍实现的道路机动车行为，不表示早期运行入口或数据入口仍存在。

- [`core-id-handles.md`][core-id-handles]：稳定标识、修订内序号、代际句柄与生命周期。
- [`lane-graph.md`][lane-graph]：车道图与连接。
- [`route-system.md`][route-system]：路线定义与跟随。
- [`lane-change-minimal-contract.md`][lane-change-contract]：#237 G1 已接受的固定预制过渡合同。
  具名停止判据已接入内核；活动过渡的快照持久化尚未实施，当前 Runtime Snapshot v7 结构化拒绝保存
  （#1217，完整格式归 #1200）。合同接受与局部判据都不等于首版换道已验收。
- [`vehicle-following.md`][vehicle-following]：跟车、安全距离与投影。
- [`signal-system.md`][signal-system]：固定时制信号、门控与车辆合规。
- [`parking-system.md`][parking-system]：#540 G1 已接受，#541 已实现
  `ParkingFacility`、显式泊位/虚拟容量、anchor selector、reserve/park/leave、原子 despawn、
  无 pose 停驻与 tagged binding。该实现不是性能认证。
- [`road-junction-model.md`][road-junction-model]：Junction / Movement / ManeuverPath / ManeuverGate。
- [`waiting-zone-conflict-right-of-way.md`][waiting-zone]：#282 G1 Accepted 的 Waiting 本地边界与 #284
  组合仲裁边界；冲突静态 exact shape 见 LFCA/共享静态路网文档，正式仲裁、持久化、
  修订迁移与生产 fixed tick 共同实现。
- [`traffic-runtime-right-of-way-policy.md`][right-of-way]：#284
  已接受实施合同，细化策略来源、中国右转灯态解释、世界绑定、格式版本矩阵与
  持久化验收，以及具名参考参数和可复现校准口径；交付状态以 GitHub 为准。
- [`traffic-runtime-near-gate-frontier.md`][near-gate]：#740
  已接受。入口 frontier 只为近门车辆打开路线，失效车辆整段重走，其余复用冲突距离；
  本拍生命周期增量必须成为接近来源；红灯门前的到达下界不早于下一档放行。
- [`traffic-runtime-vehicle-placement.md`][vehicle-placement]：#742
  已接受。新生成不要求从路线起点零速开到当前速度；生成和替换要能停住当前约束、
  降到前方更低限速，并且不让前后车立刻无法紧急制动。恢复和修订切换不套用这层检查。
- [`traffic-runtime-contender-frontier.md`][contender-frontier]：#861
  已接受，第一步已实现。准入候选名单的申请者与排队进入者只对已发布近门集合、失效名单和生命周期
  增量求值，格点到达在第一次读取时求值；格点 owner 的平局键统一为 live 序号；
  绑定不成立时整份退回全量重建。
- [`cross-section-access.md`][cross-section-access]：横断面与准入 overlay。
- [`traffic-runtime-admission-contender-filter.md`][contender-filter]：
  Accepted（#828）；准入缓存完整重建前的空贡献证明、同边运动上界、owner/失败语义与有界验证。
- [`traffic-runtime-admission-speed-drop-proof.md`][speed-drop-proof]：
  Accepted（#832 / #834）；首项精确有限距离在制动窗外的成功性证明；#834 候选未采用。
- [`numeric-representation.md`][numeric-representation]：数值分层；已提交一维几何为整数毫米，编制 `f64` 与 Spatial `f32` 仍在量化之前。
- [`traffic-runtime-integer-geometry.md`][traffic-runtime-integer-geometry]：#496 整数毫米 / 微米余数 / `mm/s` 实现合同（Accepted）；#500 编译器 IR 交通一维同一套整数毫米。
- [`spatial-geometry.md`][spatial-geometry]：有界 canonical `f32` 几何与位姿。

## 场景、规模与 Adapter

- [`example-scenarios.md`][example-scenarios]、
  [`signalized-corridor-protected-turning.md`][protected-turning]、
  [`signalized-corridor-population.md`][corridor-population]：信号化走廊几何、转向 profile 与人口策略。
- [`bevy-reference-adapter.md`][bevy-adapter]：Bevy 0.20 Reference Adapter。
- [`junction-observation-and-validation.md`][junction-observation]：#285
  Accepted，复杂路口精确机动门定位、借用式领域观测、参考场景和跨层验证；已提供
  `TrafficWorld::route_gate`、`LaneFlowSession::junction_observation` 与 `junction_debug` 示例。
- [`core-runtime-performance-baseline.md`][performance-baseline]：一万 / 十万产品目标与一百万研究包络。
- [`lf-synth-input-bundle.md`][lf-synth-input-bundle]：LF-SYNTH-v2 正式规模生成、规范输入编码与受检交付。
- [`lf-synth-execution.md`][lf-synth-execution]：W1/W2 完整窗口消费、逐拍语义核验与独立进程证据编码。
- [`lf-synth-measurement.md`][lf-synth-measurement]：Accepted；E 的固定表现、外层帧、
  W4 两帧与计时接线合同，实施与正式证据由 #1166 承担。
- [`real-road-workloads.md`][real-road-workloads]：LuST 真实路网契约。
- [`lust-bevy-population-control.md`][lust-bevy-population-control]：LuST / Bevy 示例层人口调节。
- [`chinese-style-city-workload.md`][chinese-style-city-workload]：#304 的连通路网、七套需求计划、10k/100k 运行与有限
  跨层验证合同；停车复用 #540 Accepted 合同，产品认证独立执行。
- [`urban-demand-harness.md`][urban-demand-harness]：#544 G1 Accepted，细化初态分配、
  七套调用方计划、稳定个体编号、有限校验、计数与 headless 结果包。

## 已退役（只保留结论）

- [`core-runtime.md`][core-runtime]：不是生产合同。现行消费见
  [`traffic-runtime-shared-consumption.md`][shared-consumption]。
- [`data-format.md`][data-format] / [`data-loading.md`][data-loading] /
  [`current-package-import.md`][current-package-import]：不是生产合同，只指向现行静态数据与加载路径。
- [`compiler-budget-calibration.md`][budget-calibration]：工作区已无该次研究执行器；页内给出证据提交，避免把研究数字当成产品通过。
- [`core-runtime-scalability-audit.md`][scalability-audit]：不是实现输入。现行决策见
  [`traffic-runtime-scale-decision-baseline.md`][scale-baseline]。

## 文档状态

- `Draft`：不能直接作为稳定实现输入。
- `Review`：可审阅，仍可能调整。
- `Accepted`：当前阶段实现输入。
- `Active`：持续维护的治理或索引。
- `Retired`：不是生产合同，只把读者带到现行文档。

正式设计文档页头建议包含状态、最后更新、适用范围和关联文档。涉及 Runtime API、
数据格式或 Adapter 协议的实现应先有相关 design 或 ADR。设计文档不记录单次 PR
的测试结果。

[capabilities]: ../reference/capabilities.md
[shared-consumption]: traffic-runtime-shared-consumption.md
[traffic-runtime-phase-protocol]: traffic-runtime-phase-protocol.md
[traffic-runtime-module-boundary]: traffic-runtime-module-boundary.md
[retire-precompiled-static-route]: retire-precompiled-static-route.md
[revision-cutover]: traffic-runtime-revision-cutover.md
[runtime-snapshot]: traffic-runtime-snapshot.md
[state-integrity]: traffic-runtime-state-integrity.md
[adr-0032]: ../adr/0032-runtime-state-integrity-and-distortion-disposition-authority.md
[routing-integration]: traffic-observation-and-routing-integration.md
[conflict-occurrence]: traffic-runtime-conflict-occurrence.md
[traffic-runtime-waiting-zone]: traffic-runtime-waiting-zone.md
[shared-static-network]: shared-static-network.md
[committed-editing]: committed-road-editing-state.md
[adapter-api]: adapter-api.md
[committed-pose-extraction]: committed-pose-extraction.md
[portable-artifact]: portable-canonical-artifact.md
[post-emission]: compiler-post-emission-check-and-minimal-publication-closure.md
[compiler-foundation]: compiler-foundation.md
[network-compiler]: network-compiler.md
[road-editing]: road-editing-source-and-geometry-frontend.md
[parallel-execution]: traffic-runtime-parallel-execution.md
[execution-config]: traffic-runtime-execution-config.md
[adr-0030]: ../adr/0030-single-world-parallel-execution.md
[columnar-execution]: traffic-runtime-columnar-execution.md
[adr-0031]: ../adr/0031-columnar-motion-authority.md
[scale-baseline]: traffic-runtime-scale-decision-baseline.md
[core-id-handles]: core-id-handles.md
[lane-graph]: lane-graph.md
[route-system]: route-system.md
[lane-change-contract]: lane-change-minimal-contract.md
[vehicle-following]: vehicle-following.md
[signal-system]: signal-system.md
[parking-system]: parking-system.md
[road-junction-model]: road-junction-model.md
[waiting-zone]: waiting-zone-conflict-right-of-way.md
[right-of-way]: traffic-runtime-right-of-way-policy.md
[near-gate]: traffic-runtime-near-gate-frontier.md
[vehicle-placement]: traffic-runtime-vehicle-placement.md
[contender-frontier]: traffic-runtime-contender-frontier.md
[cross-section-access]: cross-section-access.md
[contender-filter]: traffic-runtime-admission-contender-filter.md
[speed-drop-proof]: traffic-runtime-admission-speed-drop-proof.md
[numeric-representation]: numeric-representation.md
[traffic-runtime-integer-geometry]: traffic-runtime-integer-geometry.md
[spatial-geometry]: spatial-geometry.md
[example-scenarios]: example-scenarios.md
[protected-turning]: signalized-corridor-protected-turning.md
[corridor-population]: signalized-corridor-population.md
[bevy-adapter]: bevy-reference-adapter.md
[junction-observation]: junction-observation-and-validation.md
[performance-baseline]: core-runtime-performance-baseline.md
[lf-synth-input-bundle]: lf-synth-input-bundle.md
[lf-synth-execution]: lf-synth-execution.md
[lf-synth-measurement]: lf-synth-measurement.md
[real-road-workloads]: real-road-workloads.md
[lust-bevy-population-control]: lust-bevy-population-control.md
[chinese-style-city-workload]: chinese-style-city-workload.md
[urban-demand-harness]: urban-demand-harness.md
[core-runtime]: core-runtime.md
[data-format]: data-format.md
[data-loading]: data-loading.md
[current-package-import]: current-package-import.md
[budget-calibration]: compiler-budget-calibration.md
[scalability-audit]: core-runtime-scalability-audit.md
