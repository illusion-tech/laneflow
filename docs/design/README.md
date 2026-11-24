+++
schema_version = 1
id = "docs-design-index"
kind = "index"
status = "active"
revised_date = "2026-11-17"
summary = "提供运行时、数据、场景与引擎适配的设计索引。"
+++

# 设计文档

本目录回答“系统具体怎么做”；ADR 回答“为什么这样定”。设计只保存仍约束实现的
长期合同，单次验证和任务进度由 Issue/PR 承载。

## 阅读入口

跨领域能力先读[当前能力与限制][capabilities]；运行时接入从
[共享静态路网消费][shared-consumption]开始。设计接受、实现存在和性能认证是不同
事实，按专题页的适用边界和具体依据判断。

## 文档状态

状态值、取代关系及页头格式由[文档元数据与发布模型][document-metadata]统一定义。
不在此重复维护枚举或实现进度。当前 Accepted 与 Active 设计都按其原有语义保留。

历史 Retired 页已退役，有效结论由现行责任文档承接；追溯原件可查
[固定提交中的设计文档][retired-designs]。历史内容不构成第二套当前合同。

## 专题文档

[capabilities]: ../reference/capabilities.md
[shared-consumption]: traffic-runtime-shared-consumption.md
[document-metadata]: ../reference/document-metadata.md
[retired-designs]: https://github.com/illusion-tech/laneflow/tree/3f80b65f3fee19cd6085fc0d8be8f58b7a14d551/docs/design

<!-- generated-doc-index:start -->

- [适配器应用程序接口（API）][doc-design-adapter-api] — 已接受；无生效取代。定义引擎适配器与运行时、空间层的接口边界。
- [Bevy Reference Adapter][doc-design-bevy-reference-adapter] — 已接受；无生效取代。定义 Bevy 参考适配器的接入与生命周期。
- [中国特色城市工作负载 v1][doc-design-chinese-style-city-workload] — 已接受；无生效取代。定义城市工作负载、需求计划与有限验证范围。
- [CI 受影响验证计划][doc-design-ci-affected-verification-plan] — 已接受；无生效取代。定义按变更影响生成 CI 验证计划的合同：影响模型、变更三态归类、可靠性规则、全量时机与过渡策略。
- [已提交位姿缓冲与按需提取][doc-design-committed-pose-extraction] — 已接受；无生效取代。定义已提交位姿来源借用、缓冲交换与按需提取。
- [已提交道路编辑状态与编辑来源][doc-design-committed-road-editing-state] — 已接受；无生效取代。定义已提交道路编辑状态和来源绑定。
- [编译器基础设施与合成领域专用语言前端][doc-design-compiler-foundation] — 已接受；无生效取代。定义编译器中间表示、前端接入与编译基础合同。
- [编译器后发射检查与最小发布闭合][doc-design-compiler-post-emission-check-and-minimal-publication-closure] — 已接受；无生效取代。定义后发射检查与发布绑定的执行合同。
- [编译器资源账本][doc-design-compiler-resource-ledger] — 已接受；无生效取代。定义编译、发射与公开检查共用的资源账本。
- [Traffic Runtime 标识与句柄模型][doc-design-core-id-handles] — 已接受；无生效取代。定义稳定标识、修订内序号和代际句柄。
- [Traffic Runtime 产品性能基线][doc-design-core-runtime-performance-baseline] — 已接受；无生效取代。定义运行时产品规模目标、研究包络与测量边界。
- [多模式横断面与准入分层][doc-design-cross-section-access] — 已接受；无生效取代。定义横断面、所有者与准入覆盖规则。
- [走廊共享里程与横断面几何][doc-design-cross-section-stationing-and-lateral-geometry] — 已接受；无生效取代。定义走廊共享里程、段级边界与整车扫过空间的几何合同。
- [示例场景设计][doc-design-example-scenarios] — 已接受；无生效取代。定义参考场景的几何、路线与验证用途。
- [测量环境与进程观测][doc-design-host-observe] — 已接受；无生效取代。定义测量环境记录、进程快照与子进程观测。
- [复杂路口只读观测与跨层验证][doc-design-junction-observation-and-validation] — 已接受；无生效取代。定义复杂路口只读观测、参考场景与有限验证。
- [最小换道合同（Lane Change Minimal Contract）][doc-design-lane-change-minimal-contract] — 已接受；无生效取代。定义固定预制过渡的最小换道合同与验收边界。
- [Lane Graph 设计][doc-design-lane-graph] — 已接受；无生效取代。定义车道图、连接和交通拓扑。
- [LF-SYNTH 执行核验][doc-design-lf-synth-execution] — 持续维护；无生效取代。定义合成工作负载执行窗口与逐拍语义核验。
- [LF-SYNTH 规范输入交付][doc-design-lf-synth-input-bundle] — 持续维护；无生效取代。定义正式合成规模、规范输入和受检交付。
- [LF-SYNTH-v2 表现与测量接线][doc-design-lf-synth-measurement] — 已接受；无生效取代。定义合成工作负载的表现选择、帧输入和计时接线。
- [LuST / Bevy 个体人口调节契约][doc-design-lust-bevy-population-control] — 已接受；无生效取代。定义 LuST 与 Bevy 示例层的人口调节。
- [路网编译器与共享静态路网][doc-design-network-compiler] — 已接受；无生效取代。定义静态路网编译架构、实体身份和规范化。
- [数值表示与精度][doc-design-numeric-representation] — 已接受；无生效取代。定义编制、交通与空间数值表示的分层。
- [操作系统计数器读取][doc-design-os-probe] — 已接受；无生效取代。定义本进程与整机操作系统计数器读取。
- [停车系统设计][doc-design-parking-system] — 已接受；无生效取代。定义停车资源、预留、停入、离场与车辆移除。
- [可移植规范制品与辅助制品格式][doc-design-portable-canonical-artifact] — 已接受；无生效取代。定义可移植规范制品、分块和容量合同。
- [真实路网 Workload][doc-design-real-road-workloads] — 已接受；无生效取代。定义 LuST 真实路网工作负载与验收范围。
- [退役预编译静态路线][doc-design-retire-precompiled-static-route] — 已接受；无生效取代。定义移除静态路线后的场景目录与运行时接入。
- [道路编辑来源与几何编制前端][doc-design-road-editing-source-and-geometry-frontend] — 已接受；无生效取代。定义道路编辑来源与几何编制前端。
- [Road / Junction / Maneuver 静态模型][doc-design-road-junction-model] — 已接受；无生效取代。定义道路、路口、通行流向与机动路径模型。
- [Route System 设计][doc-design-route-system] — 已接受；无生效取代。定义每世界路线、注册与车辆跟随。
- [共享静态路网][doc-design-shared-static-network] — 已接受；无生效取代。定义从受检制品构建共享静态路网修订。
- [Signal System 设计][doc-design-signal-system] — 已接受；无生效取代。定义固定时制信号、门控与车辆合规。
- [Signalized Corridor Population][doc-design-signalized-corridor-population] — 已接受；无生效取代。定义信号化走廊人口与车辆回流。
- [Signalized Corridor Protected Turning Profile][doc-design-signalized-corridor-protected-turning] — 已接受；无生效取代。定义信号化走廊受保护转向与清空判据。
- [空间几何设计][doc-design-spatial-geometry] — 已接受；无生效取代。定义有界空间几何、长度绑定与位姿采样。
- [交通观测与 Routing 接入][doc-design-traffic-observation-and-routing-integration] — 已接受；无生效取代。定义交通观测、动态成本与宿主候选路线接入。
- [准入缓存完整重建前的保守筛选][doc-design-traffic-runtime-admission-contender-filter] — 已接受；无生效取代。定义准入缓存重建前的保守空贡献筛选。
- [准入空贡献筛选的远处限速下降证明][doc-design-traffic-runtime-admission-speed-drop-proof] — 已接受；无生效取代。定义准入筛选中远处限速下降的成功性证明。
- [交通运行时列式批量执行][doc-design-traffic-runtime-columnar-execution] — 已接受；无生效取代。定义列式批量执行、共享输入和稀疏资源控制。
- [路线冲突出现记录与正式仲裁能力边界][doc-design-traffic-runtime-conflict-occurrence] — 已接受；无生效取代。定义路线冲突出现记录与正式仲裁能力边界。
- [准入候选名单复用入口 frontier][doc-design-traffic-runtime-contender-frontier] — 已接受；无生效取代。定义准入候选名单复用与失效重建。
- [TrafficWorld 执行配置、资源与快照边界][doc-design-traffic-runtime-execution-config] — 已接受；无生效取代。定义世界执行配置、资源和快照边界。
- [交通运行时整数毫米几何][doc-design-traffic-runtime-integer-geometry] — 已接受；无生效取代。定义整数毫米运动、速度与停止约束。
- [Traffic Runtime 仿真与管理模块边界][doc-design-traffic-runtime-module-boundary] — 持续维护；无生效取代。定义仿真内核与管理模块的私有依赖边界。
- [近门车辆的入口 frontier][doc-design-traffic-runtime-near-gate-frontier] — 已接受；无生效取代。定义近门车辆集合、入口更新与休眠。
- [TrafficWorld 单世界精确并行执行][doc-design-traffic-runtime-parallel-execution] — 已接受；无生效取代。定义单世界精确并行的阶段、提交与失败语义。
- [TrafficWorld 串行阶段协议与内部状态分区][doc-design-traffic-runtime-phase-protocol] — 持续维护；无生效取代。定义串行阶段协议、状态分区与借用边界。
- [修订切换事务][doc-design-traffic-runtime-revision-cutover] — 已接受；无生效取代。定义在线路网修订切换事务与失败原子性。
- [路权策略编译与 ConflictArbiter 实施合同][doc-design-traffic-runtime-right-of-way-policy] — 已接受；无生效取代。定义路权策略编译与正式冲突仲裁合同。
- [Traffic Runtime 城市规模扩展决策基线][doc-design-traffic-runtime-scale-decision-baseline] — 持续维护；无生效取代。记录城市规模扩展的现行决策与证据边界。
- [交通运行时共享静态路网消费][doc-design-traffic-runtime-shared-consumption] — 已接受；无生效取代。定义运行时对共享静态路网的消费合同。
- [运行时快照][doc-design-traffic-runtime-snapshot] — 已接受；无生效取代。定义运行时快照、恢复、摘要与跨修订迁移。
- [运行时状态完整性校验与失真恢复][doc-design-traffic-runtime-state-integrity] — 已接受；无生效取代。定义状态完整性校验、失真恢复与宿主处置。
- [新生成的运动安全准入][doc-design-traffic-runtime-vehicle-placement] — 已接受；无生效取代。定义车辆新生成与替换的运动安全准入。
- [Traffic Runtime WaitingZone 运行时][doc-design-traffic-runtime-waiting-zone] — 已接受；无生效取代。定义等待区准入、存储、队列与持久化。
- [交通验证的显式组合与共享判据][doc-design-traffic-verification] — 已接受；无生效取代。定义多个交通验证执行器共享独立判据、显式组合验证规约和管理验证会话的合同。
- [LF-CN-URBAN 需求计划与无界面验证][doc-design-urban-demand-harness] — 已接受；无生效取代。定义城市需求计划、无界面执行与结果核验。
- [Vehicle Following 设计][doc-design-vehicle-following] — 已接受；无生效取代。定义跟车、纵向控制、安全距离与投影。
- [WaitingZone、ConflictZone 与通行权分层][doc-design-waiting-zone-conflict-right-of-way] — 已接受；无生效取代。定义等待区、冲突区与通行权联合分层。

[doc-design-adapter-api]: adapter-api.md
[doc-design-bevy-reference-adapter]: bevy-reference-adapter.md
[doc-design-chinese-style-city-workload]: chinese-style-city-workload.md
[doc-design-ci-affected-verification-plan]: ci-affected-verification-plan.md
[doc-design-committed-pose-extraction]: committed-pose-extraction.md
[doc-design-committed-road-editing-state]: committed-road-editing-state.md
[doc-design-compiler-foundation]: compiler-foundation.md
[doc-design-compiler-post-emission-check-and-minimal-publication-closure]: compiler-post-emission-check-and-minimal-publication-closure.md
[doc-design-compiler-resource-ledger]: compiler-resource-ledger.md
[doc-design-core-id-handles]: core-id-handles.md
[doc-design-core-runtime-performance-baseline]: core-runtime-performance-baseline.md
[doc-design-cross-section-access]: cross-section-access.md
[doc-design-cross-section-stationing-and-lateral-geometry]: cross-section-stationing-and-lateral-geometry.md
[doc-design-example-scenarios]: example-scenarios.md
[doc-design-host-observe]: host-observe.md
[doc-design-junction-observation-and-validation]: junction-observation-and-validation.md
[doc-design-lane-change-minimal-contract]: lane-change-minimal-contract.md
[doc-design-lane-graph]: lane-graph.md
[doc-design-lf-synth-execution]: lf-synth-execution.md
[doc-design-lf-synth-input-bundle]: lf-synth-input-bundle.md
[doc-design-lf-synth-measurement]: lf-synth-measurement.md
[doc-design-lust-bevy-population-control]: lust-bevy-population-control.md
[doc-design-network-compiler]: network-compiler.md
[doc-design-numeric-representation]: numeric-representation.md
[doc-design-os-probe]: os-probe.md
[doc-design-parking-system]: parking-system.md
[doc-design-portable-canonical-artifact]: portable-canonical-artifact.md
[doc-design-real-road-workloads]: real-road-workloads.md
[doc-design-retire-precompiled-static-route]: retire-precompiled-static-route.md
[doc-design-road-editing-source-and-geometry-frontend]: road-editing-source-and-geometry-frontend.md
[doc-design-road-junction-model]: road-junction-model.md
[doc-design-route-system]: route-system.md
[doc-design-shared-static-network]: shared-static-network.md
[doc-design-signal-system]: signal-system.md
[doc-design-signalized-corridor-population]: signalized-corridor-population.md
[doc-design-signalized-corridor-protected-turning]: signalized-corridor-protected-turning.md
[doc-design-spatial-geometry]: spatial-geometry.md
[doc-design-traffic-observation-and-routing-integration]: traffic-observation-and-routing-integration.md
[doc-design-traffic-runtime-admission-contender-filter]: traffic-runtime-admission-contender-filter.md
[doc-design-traffic-runtime-admission-speed-drop-proof]: traffic-runtime-admission-speed-drop-proof.md
[doc-design-traffic-runtime-columnar-execution]: traffic-runtime-columnar-execution.md
[doc-design-traffic-runtime-conflict-occurrence]: traffic-runtime-conflict-occurrence.md
[doc-design-traffic-runtime-contender-frontier]: traffic-runtime-contender-frontier.md
[doc-design-traffic-runtime-execution-config]: traffic-runtime-execution-config.md
[doc-design-traffic-runtime-integer-geometry]: traffic-runtime-integer-geometry.md
[doc-design-traffic-runtime-module-boundary]: traffic-runtime-module-boundary.md
[doc-design-traffic-runtime-near-gate-frontier]: traffic-runtime-near-gate-frontier.md
[doc-design-traffic-runtime-parallel-execution]: traffic-runtime-parallel-execution.md
[doc-design-traffic-runtime-phase-protocol]: traffic-runtime-phase-protocol.md
[doc-design-traffic-runtime-revision-cutover]: traffic-runtime-revision-cutover.md
[doc-design-traffic-runtime-right-of-way-policy]: traffic-runtime-right-of-way-policy.md
[doc-design-traffic-runtime-scale-decision-baseline]: traffic-runtime-scale-decision-baseline.md
[doc-design-traffic-runtime-shared-consumption]: traffic-runtime-shared-consumption.md
[doc-design-traffic-runtime-snapshot]: traffic-runtime-snapshot.md
[doc-design-traffic-runtime-state-integrity]: traffic-runtime-state-integrity.md
[doc-design-traffic-runtime-vehicle-placement]: traffic-runtime-vehicle-placement.md
[doc-design-traffic-runtime-waiting-zone]: traffic-runtime-waiting-zone.md
[doc-design-traffic-verification]: traffic-verification.md
[doc-design-urban-demand-harness]: urban-demand-harness.md
[doc-design-vehicle-following]: vehicle-following.md
[doc-design-waiting-zone-conflict-right-of-way]: waiting-zone-conflict-right-of-way.md

<!-- generated-doc-index:end -->
