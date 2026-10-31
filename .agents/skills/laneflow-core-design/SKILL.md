---
name: laneflow-core-design
description: 处理 LaneFlow 交通运行时（Traffic Runtime）设计。适用于当前 `TrafficWorld` 道路机动车特化的车辆状态（Vehicle State）、车道图（Lane Graph）、路线（Route）、跟车（Vehicle Following）、信号（Signal）、路口规则（Intersection Rules）与停车（Parking），确定性固定步进（Tick）与单世界并行、列式运动、状态完整性、路网修订与切换、快照/恢复、路径规划接入和 Runtime API 变更，以及交通参与单元（Traffic Participant Unit）与交通执行域（Traffic Execution Domain）的长期方向。
---

# LaneFlow 交通运行时设计（Traffic Runtime Design）

Skill 标识符（Skill ID）`laneflow-core-design` 沿用历史名称，仅作为 Traffic Runtime
设计任务的发现入口，说明见 `.agents/README.md`。

## 先读这些

1. `docs/architecture.md`
2. `docs/adr/0001-project-scope.md`
3. `docs/design/README.md`
4. `docs/governance/development-gates.md`
5. `docs/reference/glossary.md`
6. `crates/laneflow-runtime/README.md`
7. 涉及公开组件定位与宿主边界、出行编排、Routing、路网修订、存档/回放、并行或
   fidelity 时，读取 `docs/adr/0021-traffic-infrastructure-and-host-boundary.md`

## 按任务读取

下表是条件导航：只读取与本次修改相关的合同；算法、格式版本与验收数字以合同为准，
不在本 Skill 复制。

| 修改内容                                         | 按需读取                                                                                                                                                                                                                                                                  |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| fixed tick、阶段顺序、内部模块归属               | `docs/design/traffic-runtime-phase-protocol.md`、`docs/design/traffic-runtime-module-boundary.md`                                                                                                                                                                         |
| worker、调度、执行资源、首错与 join              | `docs/adr/0030-single-world-parallel-execution.md`、`docs/design/traffic-runtime-execution-config.md`、`docs/design/traffic-runtime-parallel-execution.md`                                                                                                                |
| 运动存储、批量计算、SIMD                         | `docs/adr/0031-columnar-motion-authority.md`、`docs/design/traffic-runtime-columnar-execution.md`、`crates/laneflow-motion-kernel/README.md`                                                                                                                              |
| 修订认证、状态完整性、失真处置                   | `docs/adr/0032-runtime-state-integrity-and-distortion-disposition-authority.md`、`docs/design/traffic-runtime-state-integrity.md`                                                                                                                                         |
| Waiting、Conflict 与路权策略                     | `docs/design/waiting-zone-conflict-right-of-way.md`、`docs/design/traffic-runtime-waiting-zone.md`、`docs/design/traffic-runtime-right-of-way-policy.md`                                                                                                                  |
| spawn/replace 等新鲜摆放                         | `docs/design/traffic-runtime-vehicle-placement.md`                                                                                                                                                                                                                        |
| 保存、恢复、修订切换、编辑来源                   | `docs/design/traffic-runtime-snapshot.md`、`docs/design/traffic-runtime-revision-cutover.md`、`docs/design/committed-road-editing-state.md`                                                                                                                               |
| 摘要、指纹和算法选择                             | `docs/adr/0033-digest-role-and-algorithm-selection-authority.md`                                                                                                                                                                                                          |
| 静态路网编译、共享静态路网与 Runtime 消费路径    | `docs/adr/0020-compiler-owned-static-network-and-static-image.md`、`docs/adr/0025-checked-canonical-network-and-shared-static-network.md`、`docs/design/network-compiler.md`、`docs/design/shared-static-network.md`、`docs/design/traffic-runtime-shared-consumption.md` |
| 编译器基础设施、官方前端受检模块、已验证规范 LIR | `docs/design/compiler-foundation.md`；复核生产编译性能时读取 `docs/reference/v0.10-compiler-production-baseline.md`                                                                                                                                                       |
| 道路编辑状态、候选道路替换或来源权威             | `docs/adr/0023-road-editing-state-and-phased-network-replacement.md`、`docs/design/road-editing-source-and-geometry-frontend.md`                                                                                                                                          |
| 已关闭的编译器预算校准（#308）                   | 只读 `docs/design/compiler-budget-calibration.md` 的短结论；当时制品见精确证据提交 `de4cd460a96415cafbd811141568b81f74d73534` 与交付 PR #310                                                                                                                              |

若所需 design 文档尚不存在，在对 Traffic Runtime 做高影响变更前，应先创建或提出
最小设计基线。1.0 前同时阅读 `.agents/skills/laneflow-pre-1-0/SKILL.md`。

## 命名边界

- 可运行世界：中文规范名“LaneFlow 交通运行时（LaneFlow Traffic Runtime）”，
  精确标识符 `laneflow-runtime` / `TrafficWorld`。
- `laneflow-core` / `CoreWorld` 已拆除。新设计不得重新引入 `Core` 作为运行时名称。
- 中文术语和中文定义以 `docs/reference/glossary.md` 为权威，英文只作辅助理解。

## 动态执行层边界

当前 `TrafficWorld` 只实现道路机动车车辆特化，负责：

- `VehicleState`、车道图遍历、Route 跟随与前车避让
- 信号遵守、路口规则、Waiting/Conflict 联合裁决与显式路权策略
- 停车行为
- 引擎无关的固定步进（Fixed Tick）行为

长期方向上，Traffic Runtime 以交通参与单元（Traffic Participant Unit）作为通用
抽象，并按交通执行域（Traffic Execution Domain）分离网络、运动/安全求解和生命
周期。它负责已实现执行域的参与单元、动态通行定义、每世界可变状态、运行时执行
计划和路网修订绑定。当前车辆能力不能被写成终态唯一参与者模型，通用术语也不能
反向声称非机动车、行人或轨道执行域已经实现。

Traffic Runtime 不得依赖：

- Unity API
- Unreal API
- Godot API
- O3DE API
- DOM 或 WebGL 展示 API
- 引擎特有的 actor、entity、prefab 或 scene object 模型

## 设计检查

对 Traffic Runtime 变更，必须显式确认：

- 是否改变 Traffic Runtime API？
- 是否改变数据格式假设？
- 是否影响 Adapter API？
- 是否需要确定性行为测试？
- 是否需要 ADR？
- 是否错误地让静态 / 共享契约留在动态运行时包（crate），或让编译器 / 验证器
  （Compiler / Validator）依赖当前核心对象图（Current Core Object Graph）？
- 是否把城市经济、出行需求、路线选择策略或游戏规则错误放进 Traffic Runtime？
- 是否把当前 `VehicleState`、Route、道路 occupancy 或 Parking 特化错误提升为
  所有交通参与单元的终态公共基类，或把 `ParticipantClass` 当成执行域/行为能力？
- 是否把最终分区/工作线程（Partition/Worker）写入共享静态路网，或让分区切分
  （Partition Cut）增加一 tick 延迟、改变已提交状态/事件？
- 是否区分不可变路网修订、每世界 runtime snapshot 与可重建执行计划？
- 是否把 multi-world 吞吐、Presentation LOD 或未冻结 aggregate 当成单城市世界
  fidelity/性能证据？

## 实现偏好

- 优先小而可测的运行时原语。
- 优先显式输入输出，而非隐藏引擎状态。
- 优先确定性 tick 行为。
- 展示、mesh、动画、LOD、调试 UI 放在 Adapter 或 Presentation 层。

## 交付说明

Traffic Runtime 相关工作应汇报：

- Traffic Runtime 行为变更
- API 或数据格式影响
- 已运行的测试或验证
- 文档或 ADR 是否更新
- 尚未解决的设计问题
