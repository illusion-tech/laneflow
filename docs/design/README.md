# 设计文档

本目录回答“系统具体怎么做”。`docs/adr/` 回答“为什么这样定”。GitHub 记录当前
任务与评审；这里只保存仍约束实现的长期设计。已关闭切片的收口流水账、验证轮次和
截图不进本目录，见 git 历史。

## 当前可运行路径

- `traffic-runtime-shared-consumption.md`：`TrafficWorld` 安装共享静态路网，
  Spatial 只 bind 同一根 `Arc`，current Core/JSON 入口已拆除。路线入口只留
  `register_route`（ADR 0029）。
- `retire-precompiled-static-route.md`：路网制品不声明路线；场景 catalog 0.3
  拥有示例边序列（ADR 0029）。
- `traffic-runtime-revision-cutover.md`：在线修订切换事务、切换描述符、封闭迁移
  策略、迁移增量日志与失败关闭预算（#302 G1；G2 起实现）。
- `traffic-runtime-snapshot.md`：版本化运行时快照容器、保存/恢复合同、回放与
  跨修订迁移入口（#302 G1；G2 起实现）。
- `traffic-observation-and-routing-integration.md`：已提交交通观测 full/delta/partition、
  宿主自有 Routing 成本绑定、候选路线注册与 #302 失效接缝（#303 G1 已接受；G2
  起实现）。
- `traffic-runtime-conflict-occurrence.md`：#283 的路线冲突出现项、route-local
  坐标、独立容量与 #284 前的全车身能力保护（#283 G1 已接受）。
- `traffic-runtime-waiting-zone.md`：#282 G1 Accepted 的 WaitingZone 本地
  membership/admission/storage/queue 设计，以及当前 LFRS 4、runtime state 4、
  deterministic digest 6 合同；不包含 #284 的组合 ledger。
- `shared-static-network.md`：从受检 LFCA 构建 `SharedNetworkRevision`。
- `adapter-api.md`：Runtime / Spatial 与引擎适配器的只读快照、位姿和权威边界。
- `portable-canonical-artifact.md`：统一 LFCA 4 / LFSM 3 / LFSD 3 /
  LFCP 2、确定性分块与单路网一百万现实混合静态实体容量合同。
- `compiler-post-emission-check-and-minimal-publication-closure.md`：后发射检查与
  LFCP v2 最小发布闭合。
- `compiler-foundation.md`：编译器 crate、Typed AST → HIR → MIR → Canonical LIR、
  合成 DSL 与官方前端接入。
- `network-compiler.md`：#291 目标静态编译架构。
- `road-editing-source-and-geometry-frontend.md`：道路编辑 FlatBuffers v3 来源与
  几何编制前端合同。

## 领域规则

这些文档约束 Runtime 仍实现的道路机动车行为，不表示早期运行入口或数据入口仍存在。

- `core-id-handles.md`：external ID、typed handle、lifecycle。
- `lane-graph.md`：车道图与连接。
- `route-system.md`：路线定义与跟随。
- `vehicle-following.md`：跟车、安全距离与投影。
- `signal-system.md`：固定时制信号、门控与车辆合规。
- `parking-system.md`：#540 G1 Accepted；`ParkingFacility`、显式泊位/虚拟容量、
  anchor selector、reserve/park/leave、原子 despawn、无 pose 停驻与迁移合同（#541 起实现）。
- `road-junction-model.md`：Junction / Movement / ManeuverPath / ManeuverGate。
- `waiting-zone-conflict-right-of-way.md`：#282 G1 Accepted 的 Waiting 本地边界与 #284
  组合仲裁边界；冲突静态 exact shape 见 LFCA/共享根文档，路线出现项与
  `ConflictRuntimeUnavailable` 临时能力保护见 #559 当前设计。
- `cross-section-access.md`：横断面与准入 overlay。
- `numeric-representation.md`：数值分层；已提交一维几何为整数毫米，编制 `f64` 与 Spatial `f32` 仍在量化之前。
- `traffic-runtime-integer-geometry.md`：#496 整数毫米 / 微米余数 / `mm/s` 实现合同（Accepted）；#500 编译器 IR 交通一维同一套整数毫米。
- `spatial-geometry.md`：有界 canonical `f32` 几何与位姿。

## 场景、规模与 Adapter

- `example-scenarios.md`、`signalized-corridor-protected-turning.md`、
  `signalized-corridor-population.md`：信号化走廊几何、转向 profile 与人口策略。
- `bevy-reference-adapter.md`：Bevy 0.19 Reference Adapter。
- `core-runtime-performance-baseline.md`：一万 / 十万产品目标与一百万研究包络。
- `real-road-workloads.md`：LuST 真实路网契约。
- `lust-bevy-population-control.md`：LuST / Bevy 示例层人口调节。
- `core-runtime-scalability-audit.md`：城市级可扩展性前置约束；不实现生产分区。
- `chinese-style-city-workload.md`：#304 的 topology/demand/runtime 分层草案；其中 #540
  停车切片已 Accepted，其他城市工作负载切片仍待各自冻结。

## 已退役（只保留结论）

- `core-runtime.md`：早期运行时基线；现行路径见
  `traffic-runtime-shared-consumption.md`。
- `data-format.md` / `data-loading.md` / `current-package-import.md`：早期数据入口与加载
  crate 已删除。
- `compiler-budget-calibration.md`：#308 一次性研究已关闭，证据在 git 历史。

## 文档状态

- `Draft`：不能直接作为稳定实现输入。
- `Review`：可审阅，仍可能调整。
- `Accepted`：当前阶段实现输入。
- `Active`：持续维护的治理或索引。
- `Retired`：结论仍有效，正文已收缩；细节见 git 历史。

正式设计文档页头建议包含状态、最后更新、适用范围和关联文档。涉及 Runtime API、
数据格式或 Adapter 协议的实现应先有相关 design 或 ADR。设计文档不记录单次 PR
的测试结果。
