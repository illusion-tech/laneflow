+++
schema_version = 1
id = "docs-adr-index"
kind = "index"
status = "active"
revised_date = "2026-11-17"
summary = "提供架构决策记录及其有效性关系的生成索引。"
+++

# 架构决策记录

ADR 回答“为什么这样定”，不替代详细设计。高影响、难回退且涉及多个模块或长期
职责边界的选择应记录 ADR；普通字段补充、一次 PR 测试结果与开放讨论不单独建立 ADR。

## 命名规则

文件名使用 `NNNN-short-title.md`，稳定 ID 使用 `adr-NNNN`。标题变更不改变 ID。
页头只保存接受状态；后继文档单向登记取代范围，下面的有效性由关系图生成。
具体规则见[文档元数据][document-metadata]。

## 状态与阅读边界

已接受不表示条款全部仍有效，也不表示实现已交付。部分取代必须阅读后继文档的范围
说明；无生效取代只说明没有登记生效边。实时 required checks 以
[开发阶段评审][development-gates-checks]为准，历史 ADR 中的数量不是实时配置副本。

1.0 前按[开发原则][pre-1-0]维护当前正本；已取消方案可以压缩为决策理由、有效边界和
后继入口，原件从 Git 历史追溯。1.0 后的兼容与决策演进另行设计。

## 当前 ADR 列表

[document-metadata]: ../reference/document-metadata.md
[development-gates-checks]: ../governance/development-gates.md#62-required-checks
[pre-1-0]: ../../.agents/skills/laneflow-pre-1-0/SKILL.md

<!-- generated-doc-index:start -->

- [0001 Project Scope][doc-adr-0001] — 已接受；无生效取代。定义交通运行时的项目定位、宿主职责与非目标。
- [0002 Dependency and Licensing Constraints][doc-adr-0002] — 已接受；无生效取代。规定依赖和许可证的选择边界。
- [0003 Runtime Tick and Determinism][doc-adr-0003] — 已接受；无生效取代。规定固定步进与确定性执行原则。
- [0004 Core Implementation Language][doc-adr-0004] — 已接受；无生效取代。选择 Rust 作为主要实现语言。
- [0005 Core Identity and Handle Model][doc-adr-0005] — 已接受；部分取代。定义标识、句柄与对象生命周期的决策依据。
- [0006 Vehicle Following Control and Safety Architecture][doc-adr-0006] — 已接受；部分取代。定义跟车控制模型、安全约束与职责分层。
- [0007 Traffic Data Crate and Loader Boundary][doc-adr-0007] — 已接受；部分取代。记录交通数据加载与运行时依赖边界的决策。
- [0008 Pre-1.0 Data Format Version Policy][doc-adr-0008] — 已接受；部分取代。规定 1.0 前数据格式版本及迁移原则。
- [0009 Signal Indication, Movement Gate, and Policy Separation][doc-adr-0009] — 已接受；部分取代。分离信号灯色、空间准入、法规策略与安全裁决。
- [ADR 0010：停车设施、资源绑定与车辆生命周期裁决权][doc-adr-0010] — 已接受；无生效取代。规定停车设施、资源绑定与车辆生命周期裁决权。
- [0011 Schema Identifier and Public Publication Contract][doc-adr-0011] — 已接受；全部取代。记录历史 JSON Schema 标识与公开发布义务。
- [0012 Core 数值正本与表现层精度][doc-adr-0012] — 已接受；全部取代。记录被后继数值决策替代的精度方案。
- [0013 引擎无关的空间几何与长度正本][doc-adr-0013] — 已接受；部分取代。定义空间几何、长度正本与引擎无关职责。
- [0014 补偿残差感知的 f32 Core 数值正本与迁移验收][doc-adr-0014] — 已接受；部分取代。记录补偿残差数值方案、迁移判断及其适用边界。
- [0015 有界 f32 Canonical 空间框架][doc-adr-0015] — 已接受；部分取代。规定有界 f32 空间框架与运行时表示误差。
- [0016 Scenario Population 与车辆回流生命周期裁决权][doc-adr-0016] — 已接受；无生效取代。规定参考场景人口与车辆回流的职责边界。
- [ADR 0017：静态 Road/Junction/Maneuver 与 Gate 身份][doc-adr-0017] — 已接受；部分取代。规定路口、机动路径与机动门的静态身份。
- [ADR 0018：多模式横断面 owner 与参与者准入 overlay][doc-adr-0018] — 已接受；无生效取代。规定横断面所有权和交通参与类别准入分层。
- [ADR 0019：WaitingZone、ConflictZone 与车辆级通行权 authority][doc-adr-0019] — 已接受；无生效取代。规定等待区、冲突区与车辆级通行权的分层。
- [ADR 0020：编译器拥有的静态路网与目标静态镜像][doc-adr-0020] — 已接受；部分取代。规定编译器拥有静态路网的架构与消费职责。
- [ADR 0021：交通基础设施定位与宿主边界][doc-adr-0021] — 已接受；部分取代。规定公开组件定位、宿主边界与规模演进原则。
- [ADR 0022：编制曲线与规范折线的误差预算分层][doc-adr-0022] — 已接受；无生效取代。分离编制曲线、规范折线与运行时表示的误差预算。
- [ADR 0023：道路编辑状态正本与分阶段路网替换][doc-adr-0023] — 已接受；部分取代。规定道路编辑状态正本与分阶段路网替换。
- [ADR 0024：编译器后发射检查与最小发布闭合][doc-adr-0024] — 已接受；部分取代。规定编译器后发射检查与最小发布绑定。
- [ADR 0025：受检规范路网与共享静态路网][doc-adr-0025] — 已接受；部分取代。规定受检规范路网与进程内共享静态修订。
- [0026 合并治理重建：原生审阅 Check、Merge Queue 与 Schema Publication 退役][doc-adr-0026] — 已接受；部分取代。记录合并治理重建与 Schema 公共发布义务退役。
- [0027 退役 External Review 自定义 Check，收敛五项机器检查][doc-adr-0027] — 已接受；无生效取代。退役自定义外部审阅检查并采用原生审阅边界。
- [0028 交通一维几何整数毫米与固定步进合同][doc-adr-0028] — 已接受；无生效取代。规定交通一维几何整数毫米与固定步进合同。
- [0029 退役预编译静态路线，路线入口只留 register_route][doc-adr-0029] — 已接受；无生效取代。移除预编译静态路线，保留每世界路线注册入口。
- [0030 单世界精确并行与执行配置分离][doc-adr-0030] — 已接受；无生效取代。规定单世界精确并行及执行配置分离。
- [0031 列式运动正本与独立 SIMD 数值边界][doc-adr-0031] — 已接受；无生效取代。规定列式运动正本与独立 SIMD 数值边界。
- [0032 运行时状态完整性校验与失真处置裁决权][doc-adr-0032] — 已接受；无生效取代。规定运行时状态完整性检测与失真处置裁决权。
- [0033 摘要角色与摘要算法选型裁决权][doc-adr-0033] — 已接受；无生效取代。规定摘要角色分类与算法选型职责。
- [0034 钉版编译缓存包装器信任与溯源检查契约][doc-adr-0034] — 已接受；无生效取代。规定钉版编译缓存包装器的信任与溯源检查。
- [0035 编译资源上限只声明容量与宿主预算，受控分配统一经资源账本][doc-adr-0035] — 已接受；无生效取代。规定容量限额、宿主预算与受控分配资源账本。

[doc-adr-0001]: 0001-project-scope.md
[doc-adr-0002]: 0002-dependency-and-licensing-constraints.md
[doc-adr-0003]: 0003-runtime-tick-and-determinism.md
[doc-adr-0004]: 0004-core-implementation-language.md
[doc-adr-0005]: 0005-core-identity-and-handle-model.md
[doc-adr-0006]: 0006-vehicle-following-control-and-safety.md
[doc-adr-0007]: 0007-traffic-data-crate-and-loader-boundary.md
[doc-adr-0008]: 0008-pre-1.0-data-format-version-policy.md
[doc-adr-0009]: 0009-signal-indication-gate-and-policy-separation.md
[doc-adr-0010]: 0010-parking-binding-and-vehicle-lifecycle-authority.md
[doc-adr-0011]: 0011-schema-identifier-and-publication-contract.md
[doc-adr-0012]: 0012-core-numeric-authority-and-presentation-precision.md
[doc-adr-0013]: 0013-engine-neutral-spatial-geometry-and-length-authority.md
[doc-adr-0014]: 0014-residual-aware-f32-core-authority-and-migration-gates.md
[doc-adr-0015]: 0015-bounded-f32-canonical-spatial-frames.md
[doc-adr-0016]: 0016-scenario-population-and-recycle-lifecycle-authority.md
[doc-adr-0017]: 0017-static-road-junction-maneuver-and-gate-identity.md
[doc-adr-0018]: 0018-multimodal-cross-section-and-access-overlay.md
[doc-adr-0019]: 0019-waiting-zone-conflict-right-of-way-authority.md
[doc-adr-0020]: 0020-compiler-owned-static-network-and-static-image.md
[doc-adr-0021]: 0021-traffic-infrastructure-and-host-boundary.md
[doc-adr-0022]: 0022-authoring-curve-and-canonical-polyline-error-budgets.md
[doc-adr-0023]: 0023-road-editing-state-and-phased-network-replacement.md
[doc-adr-0024]: 0024-compiler-post-emission-check-and-minimal-publication-closure.md
[doc-adr-0025]: 0025-checked-canonical-network-and-shared-static-network.md
[doc-adr-0026]: 0026-merge-governance-rebuild.md
[doc-adr-0027]: 0027-retire-external-review-check.md
[doc-adr-0028]: 0028-integer-millimeter-traffic-geometry.md
[doc-adr-0029]: 0029-retire-precompiled-static-route.md
[doc-adr-0030]: 0030-single-world-parallel-execution.md
[doc-adr-0031]: 0031-columnar-motion-authority.md
[doc-adr-0032]: 0032-runtime-state-integrity-and-distortion-disposition-authority.md
[doc-adr-0033]: 0033-digest-role-and-algorithm-selection-authority.md
[doc-adr-0034]: 0034-pinned-compiler-cache-wrapper-trust.md
[doc-adr-0035]: 0035-compile-limits-and-resource-ledger.md

<!-- generated-doc-index:end -->
