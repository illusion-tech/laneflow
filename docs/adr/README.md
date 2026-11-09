# 架构决策记录

本目录用于记录 LaneFlow 的架构决策记录。

ADR 关注“为什么这样定”，不替代详细设计文档。涉及高影响、难回退、会影响多个模块或长期兼容性的决策，应优先进入 ADR。

## 适用范围

优先写 ADR 的议题：

- LaneFlow 交通运行时与 Engine Adapter 的边界。
- Traffic Runtime 不依赖具体游戏引擎或外部重型交通仿真器。
- Runtime tick、确定性、时间步长策略。
- lane graph、route、signal、parking 等核心数据模型。
- 数据格式版本策略。
- Adapter API 稳定性策略。
- 破坏性变更和兼容性策略。

不适合写 ADR 的内容：

- 普通字段补充。
- 单个示例项目配置。
- 一次 PR 的测试结果。
- 尚未形成结论的开放讨论。

## 当前 ADR 列表

编号、标题与文件顺序一致。状态取自各 ADR 页头；后继 ADR 的部分取代关系写在括号里。
ADR 0027 记录的是当时把 required checks 收敛为五项；现行九项清单见
[开发阶段评审 §6.2](../governance/development-gates.md#62-required-checks)。

- [0001 Project Scope](0001-project-scope.md)（Accepted）：项目定位与范围边界。
- [0002 Dependency and Licensing Constraints](0002-dependency-and-licensing-constraints.md)（Accepted）：依赖与许可证约束，不依赖 SUMO / CARLA / libsumo。
- [0003 Runtime Tick and Determinism](0003-runtime-tick-and-determinism.md)（Accepted）：Runtime tick 与确定性策略。
- [0004 Core Implementation Language](0004-core-implementation-language.md)（Accepted）：实现语言为 Rust。标题中的 Core 是该 ADR 的历史名称。
- [0005 Core Identity and Handle Model](0005-core-identity-and-handle-model.md)（Accepted）：identity、handle 与 lifecycle。标题中的 Core 是该 ADR 的历史名称。
- [0006 Vehicle Following Control and Safety Architecture](0006-vehicle-following-control-and-safety.md)（Accepted）：跟车控制、安全与扩展边界。
- [0007 Traffic Data Crate and Loader Boundary](0007-traffic-data-crate-and-loader-boundary.md)（Accepted）：Traffic Data crate、loader 与规范化边界。
- [0008 Pre-1.0 Data Format Version Policy](0008-pre-1.0-data-format-version-policy.md)（Accepted）：1.0 前数据格式版本策略。
- [0009 Signal Indication, Movement Gate, and Policy Separation](0009-signal-indication-gate-and-policy-separation.md)（Accepted）：indication、门控与策略分离；pair-based Gate identity 由 0017 取代。
- [0010：停车设施、资源绑定与车辆生命周期裁决权](0010-parking-binding-and-vehicle-lifecycle-authority.md)（Accepted；#540 G1）：`ParkingFacility`、显式/虚拟停车资源与 Adapter 裁决权。
- [0011 Schema Identifier and Public Publication Contract](0011-schema-identifier-and-publication-contract.md)（Superseded）：公共发布义务已被 0026 取代，只保留历史说明。
- [0012 Core 数值正本与表现层精度](0012-core-numeric-authority-and-presentation-precision.md)（Superseded）：已被 0014 取代。
- [0013 引擎无关的空间几何与长度正本](0013-engine-neutral-spatial-geometry-and-length-authority.md)（部分取代）：精度条款已被 0015 取代。
- [0014 补偿残差感知的 f32 Core 数值正本与迁移验收](0014-residual-aware-f32-core-authority-and-migration-gates.md)（Accepted）：Spatial 精度条款已被 0015 部分取代；生产一维几何实施已被 0028 部分取代。
- [0015 有界 f32 Canonical 空间框架](0015-bounded-f32-canonical-spatial-frames.md)（Accepted）：有界局部 canonical frame 与运行时 f32 空间类型。
- [0016 Scenario Population 与车辆回流生命周期裁决权](0016-scenario-population-and-recycle-lifecycle-authority.md)（Accepted）：场景人口与确定性回流。
- [0017：静态 Road/Junction/Maneuver 与 Gate 身份](0017-static-road-junction-maneuver-and-gate-identity.md)（Accepted）：静态 Junction/Movement/ManeuverPath 与 ManeuverGate。
- [0018：多模式横断面 owner 与参与者准入 overlay](0018-multimodal-cross-section-and-access-overlay.md)（Accepted）：横断面 owner 与准入 overlay。
- [0019：WaitingZone、ConflictZone 与车辆级通行权 authority](0019-waiting-zone-conflict-right-of-way-authority.md)（Accepted）：#282 与 #284 已实现。
- [0020：编译器拥有的静态路网与目标静态镜像](0020-compiler-owned-static-network-and-static-image.md)（Accepted；#291 G1）：独立 validator 与收据职责已由 0024 部分取代；镜像文件/ABI 已由 0025 继续取代。
- [0021：交通基础设施定位与宿主边界](0021-traffic-infrastructure-and-host-boundary.md)（Accepted）：宿主应用、出行编排、路径规划与交通运行时分层。
- [0022：编制曲线与规范折线的误差预算分层](0022-authoring-curve-and-canonical-polyline-error-budgets.md)（Accepted；#296 FlatBuffers G1）：`Fine2Cm`、`Balanced5Cm`、`Compact10Cm`。
- [0023：道路编辑状态正本与分阶段路网替换](0023-road-editing-state-and-phased-network-replacement.md)（Accepted；#296 FlatBuffers G1）：§2.1 的来源形状已被 0029 部分取代。
- [0024：编译器后发射检查与最小发布闭合](0024-compiler-post-emission-check-and-minimal-publication-closure.md)（Accepted）：LFCA/LFSM/LFSD 后发射检查与 LFCP v2。
- [0025：受检规范路网与共享静态路网](0025-checked-canonical-network-and-shared-static-network.md)（Accepted）：从受检 LFCA 构建 `SharedNetworkRevision`。
- [0026 合并治理重建：原生审阅 Check、Merge Queue 与 Schema Publication 退役](0026-merge-governance-rebuild.md)（Accepted；#468）：第 5–9 节已被 0027 取代。
- [0027 退役 External Review 自定义 Check，收敛五项机器检查](0027-retire-external-review-check.md)（Accepted；#492）：退役 External Review。标题中的五项是该 ADR 当时的决定；现行 required checks 不在本条复述。
- [0028 交通一维几何整数毫米与固定步进合同](0028-integer-millimeter-traffic-geometry.md)（Accepted）：整数毫米、微米余数与 `4..=1000` ms。已部分取代 0014 的生产一维几何实施。
- [0029 退役预编译静态路线，路线入口只留 `register_route`](0029-retire-precompiled-static-route.md)（Accepted）：路网产品删除 `StaticRoute`。道路编辑来源 `format_version = 4`；本 ADR 将场景 catalog 定为 0.3，现行 catalog 为 0.4。当时表内的 LFCA 4 基线已被现行 LFCA 5 / LFSM 4 / LFSD 4 取代，以可移植制品合同为准。
- [0030 单世界精确并行与执行配置分离](0030-single-world-parallel-execution.md)（Accepted）：首版独立计算并行与执行配置分离。配置分离与 worker 1–16 已实现；城市性能未认证。
- [0031 列式运动正本与独立 SIMD 数值边界](0031-columnar-motion-authority.md)（Accepted；#814）：列式运动正本。实施不等于收益认证。
- [0032 运行时状态完整性校验与失真处置裁决权](0032-runtime-state-integrity-and-distortion-disposition-authority.md)（Accepted；#955 G1）：检测权与宿主处置裁决权。机制见 [运行时状态完整性设计](../design/traffic-runtime-state-integrity.md)。
- [0033 摘要角色与摘要算法选型裁决权](0033-digest-role-and-algorithm-selection-authority.md)（Accepted；#1051）：五类摘要角色。SHA-256 只作为互操作摘要默认算法；运行时确定性状态摘要现行版本为 8 / BLAKE3-256。
- [0034 钉版编译缓存包装器信任与溯源检查契约](0034-pinned-compiler-cache-wrapper-trust.md)（Accepted；#1074）：钉版包装器的准入与 `build_compiler_matches_pinned_toolchain`。

## 命名规则

文件命名使用：

```text
NNNN-short-title.md
```

示例：

- `0001-project-scope.md`
- `0002-dependency-and-licensing-constraints.md`
- `0003-runtime-tick-and-determinism.md`
- `0004-core-implementation-language.md`

## 状态

ADR 状态建议使用：

- `Proposed`
- `Accepted`
- `Deprecated`
- `Superseded`

若决策被替代，应新增后续 ADR，不要静默改写历史决策。
