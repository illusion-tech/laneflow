# 路网编译器与静态运行时镜像

**文档状态**: Draft（#291 G1 M2 终态架构修订；M1 identity v1 已保留）<br>
**最后更新**: 2026-07-28<br>
**适用范围**: authoring frontend、typed IR、静态网络编译权威、身份派生、portable canonical artifact、target runtime image、source map、semantic diff、独立校验器和 current→target 迁移<br>
**实现状态**: 未实现；当前 production 仍使用 Traffic v0.10 / SpatialPackage v0.1 / ScenarioManifest v0.1、`InitialTrafficData` 和现有 Spatial registry；#292 已重划为 compiler foundation + Synthetic DSL frontend，并继续 Blocked by #291 G1

**关联决策与设计**:

- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0011-schema-identifier-and-publication-contract.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0020-compiler-owned-static-network-and-runtime-image.md`
- `core-id-handles.md`
- `data-format.md`
- `data-loading.md`
- `spatial-geometry.md`
- `road-junction-model.md`
- `signal-system.md`
- `waiting-zone-conflict-right-of-way.md`

## 1. 结论与状态

#291 冻结的目标不是“生成另一份 JSON 的工具”，而是新的静态数据体系：

```text
Synthetic DSL ─┐
Geometry doc ──┼─> typed AST -> HIR -> MIR -> validated canonical LIR
Importers ─────┤                                      │
Editor ────────┘                                      ├─> portable canonical artifact
                                                       ├─> target runtime image
                                                       ├─> source map / diagnostics
                                                       └─> semantic diff

target runtime image ─┬─> Core: StaticTrafficImage + per-world mutable state
                      └─> Spatial: StaticSpatialImage + batch scratch/output
```

核心结论：

- 取消 L1/L2 作为架构层；Synthetic DSL、Geometry、Import、Editor 是平级 frontend；
- compiler 是全部静态网络的唯一编译权威；
- `InitialTrafficData` 和 Core registries 不是 IR；
- typed AST/HIR/MIR/LIR 逐级降低，只有 validated canonical LIR 可以进入 emitter；
- portable artifact 与 target runtime image 是同一 LIR 的不同后端；
- Core/Spatial 直接消费同一 immutable image 中的对齐视图；
- 静态只读数据与每 world 可变状态物理分离；
- production startup 不再解析 JSON、按字符串 rebind、重建 registry 或重新编译
  initial route occurrences；
- independent validator 不复用 compiler 的语义校验实现；
- M1 已冻结的 canonical tuple、BLAKE3-128 `StableId128`、XXH3 瞬态加速边界和
  dense handle 热路径不变。

本文描述 target。ADR 0020 Accepted 且迁移 G4 完成前，现有 JSON/Data/Core/Spatial
路径仍是 current production contract。

## 2. 为什么不能继续 L1/L2

原草案把 scenario builder DSL 定义为 L1，把 geometry compiler 定义为 L2，并让
L1 先输出 Core 输入。这会产生三个结构性错误：

1. **frontend 被误当作 lowering level**：Synthetic DSL 并不比 Geometry/OSM/Editor
   “更低”或“更高”；它们只是不同来源；
2. **Core object graph 泄漏到 compiler 中层**：`InitialTrafficData` 已经完成 Core
   handle 解析、registry/rebind 和 route occurrence 编译，无法作为 target-neutral
   输入；
3. **联合静态事实被拆开**：topology、geometry、identity、length 和 Gate/Waiting
   coverage 只有在同一个 MIR/LIR 中才能一次性裁决，先 Core 后 Spatial 必然重复
   join 和校验。

因此 #292 的 Synthetic DSL 是 compiler frontend 的首个纵向验证，不是 L1。未来
Geometry、OSM 或 Editor frontend 不依赖 #292 的 DSL 语法或 Core-shaped output，
只依赖共同的 typed AST/HIR contract 和 compiler passes。

## 3. 当前生产与目标边界

| 关注点               | 当前 production                                               | target                                                                    |
| -------------------- | ------------------------------------------------------------- | ------------------------------------------------------------------------- |
| authoring            | 手写 canonical JSON + corridor generator 内部 TOML/DTO        | Geometry doc 为长期 SSOT；DSL/import/editor 为 frontend                   |
| Traffic load         | JSON → private DTO → Core constructors → `InitialTrafficData` | runtime verifier → `StaticTrafficImage` view                              |
| Spatial load         | JSON + manifest → external ID bind → `SpatialRegistry`        | runtime verifier → `StaticSpatialImage` view                              |
| identity             | external strings 在加载期解析为 handle                        | compiler 生成 StableId128；image 使用 dense `u32` handle                  |
| static occurrence    | initial/dynamic Route 注册时由 Core 编译                      | initial/static occurrence 在 image 中预编译；dynamic Route 仍由 Core 编译 |
| governance artifact  | exact-current JSON Schema/fixture                             | portable canonical artifact + source map + semantic diff                  |
| performance artifact | JSON object graph normalization 后的 registries               | target/layout/feature-specific immutable runtime image                    |
| validation           | schema + loader + Core/Spatial constructors                   | compiler + independent validator + runtime structural verifier            |

迁移不得把 target 写成现状，也不得为了复用 current DTO/constructor 而冻结错误的
compiler IR。

## 4. Authority

### 4.1 Authoring authority

长期唯一 authoring SSOT 是 Geometry document。它持久化：

- namespace、稳定 key、模块/import；
- road/corridor/section/lane 横断面；
- junction、movement、path 与显式 override；
- curve、elevation、frame 和设计参数；
- signals、Gate/WaitingZone、parking、access/policy 静态声明；
- source annotation 与 editor metadata。

Synthetic DSL、importer 或 migration tool 可以生成/组合同一 typed source model，
但 generated artifact 永远不可手改，也不得反向覆盖 source。

### 4.2 Compiler authority

compiler 唯一负责静态网络的：

- symbol/reference/unit resolution；
- topology/geometry 展开与全局语义；
- StableId128 与 deterministic order；
- dense logical ordinal、owner/member/reverse indexes；
- Traffic/Spatial 长度共同派生；
- initial/static Route/Maneuver/Gate/Waiting occurrence；
- portable artifact 和 runtime image emission。

### 4.3 Runtime authority

- Core：fixed tick、vehicle、dynamic Route lifecycle、controller clock、grant/
  reservation、parking occupancy 和所有可变交通状态；
- Spatial：canonical geometry sampling 与 pose batch；
- Adapter：宿主 entity、Transform、frame placement、presentation lifecycle；
- runtime image：只读静态事实，不持有可变 authority。

## 5. Frontend 架构

### 5.1 Synthetic DSL frontend（#292）

主要用途：

- 测试、fixture、benchmark 和示例场景；
- 用少量参数展开规则走廊、网格、路口与交通配置；
- 作为 compiler 全纵向管线的首个可执行 frontend。

它必须输出带稳定 authoring key 和 source span 的 typed AST，不得：

- 直接构造 `InitialTrafficData` 或 `SpatialRegistry`；
- 跳过 HIR/MIR/LIR validation；
- 使用 Rust 容器遍历顺序作为 identity/order；
- 把 builder-only TOML/Rust type 公开为 interchange contract。

### 5.2 Geometry document frontend

长期生产 authoring frontend。目标模型包含：

1. 参考线：三维 curve segments、弧长与方向；
2. 横断面：沿参考线分段变化的 lane/facility 结构；
3. 连接：junction/connection intent、默认生成策略与显式 override；
4. 规则：signals、Gate/WaitingZone、access、parking 和其他静态 overlay。

曲线在 MIR 中按确定性误差预算离散为 canonical f32 polyline；runtime image 不保存
authoring curve evaluator。具体 curve segment 集合由独立 numeric/authoring G1
和 benchmark 冻结，不在 #291 先选 library。

### 5.3 Import 与 Editor frontend

- importer 保存来源 provenance，必须显式生成稳定 key；不允许用导入遍历 ordinal
  冒充身份；
- editor 直接编辑 authoring model，诊断以 source span/画布 selection 回传；
- 两者都进入共同 HIR，不维护私有 semantic compiler。

## 6. typed IR 与 pass 边界

### 6.1 typed AST

保留 frontend 语法与来源：

- explicit/derived declarations；
- stable authoring key；
- source span、file/module provenance；
- typed number token 与 unit；
- 尚未解析但已分类的 reference。

AST 不含 Core handle、runtime slot、target ABI 或 compiler 推断出的最终 geometry。

### 6.2 HIR

完成：

- module/import/namespace resolution；
- symbol table 与 typed reference；
- unit normalization；
- defaults 的显式化；
- authoring semantic category 与 overlay merge。

HIR 仍能追溯全部 source span。跨 module 重名、引用 cycle、unit/type 错误在此失败。

### 6.3 MIR

完成全局静态语义：

- corridor/section/lane 展开；
- boundary、edge、junction、movement、path 生成；
- curve tessellation、canonical frame partition 和 geometry continuity；
- signals、Gate、WaitingZone、parking、access 与 topology 绑定；
- canonical identity tuple 构造与 identity closure；
- Traffic length / Spatial arc length 共同派生；
- route/path occurrence 和 reverse indexes；
- global ownership、coverage、coherence 与 policy-independent safety checks。

MIR 可以使用 compiler arena 和临时 cache，但不得产生 target layout 或把 hash
fingerprint 当 identity。

### 6.4 validated canonical LIR

LIR 是 emitter 唯一输入：

- 所有实体都有 StableId128 和稳定 logical ordinal；
- 所有引用已转为 typed ordinal；
- 数值已规范化到 canonical units/representation；
- 所有 relation 以 deterministic flat sequence/range 表达；
- 静态 occurrence、sampling tables 和 layout-independent precompute 已完成；
- source map key 与 semantic diff key 已冻结；
- 未解决引用、隐式默认或需要后端判断的语义均为零。

LIR 使用 `u32` logical ordinal；超出容量必须结构化失败，不能升级为平台相关
`usize` 后继续编译。

### 6.5 推荐 pass 顺序

```text
parse/type
  -> upgrade authoring format
  -> resolve namespace/module/symbol/unit
  -> expand cross-section and synthetic constructs
  -> construct topology and geometry
  -> bind signals/parking/access/waiting
  -> derive canonical identity
  -> validate global semantics
  -> normalize deterministic LIR order
  -> precompute occurrence/index/sampling data
  -> freeze validated canonical LIR
  -> emit all artifacts atomically
```

pass 可以并行或增量执行，但 clean single-thread compile 是确定性 oracle；任何模式
都必须生成相同 portable artifact 和 semantic diff。

## 7. 身份派生契约（M1 identity v1）

### 7.1 权威与分层

身份的权威是可审计的 **canonical identity tuple**，不是 UUID/哈希值本身：

```text
CanonicalIdentity =
  authoringNamespaceId
  + entityKind
  + stable parent anchors
  + stable local anchors
```

`authoringNamespaceId` 标识一个独立 authoring 文档或生成域；stable anchor 是
author 显式声明、持久化并可重命名显示名称而不变化的 ASCII key。显示名称、坐标、
浮点几何、采样点、数组下标、横向/纵向序号、容器迭代顺序、自动分类结果和全局
自增值都不得成为 identity anchor。

compiler 从 canonical tuple 确定性编码 `StableId128`。Portable artifact、runtime
image cold table 和 source map 使用该稳定身份；target image hot table 使用编译期
分配的 typed dense handle。identity 计算和字符串解析不得进入 startup rebind 或
steady tick。

### 7.2 稳定 authoring anchor

- `authoringNamespaceId` 与各 entity key 在首次创建时写入 authoring SSOT；复制文档
  若需要新身份域，必须显式创建 namespace；
- helper 只能从稳定 parent key、local key 和 semantic role 组合 child key，不得
  默认使用“当前第 N 个 child”；
- lane/section/path 展示顺序是独立属性；插入 sibling 不改变既有身份；
- compiler 推断出尚无稳定 key 的 junction/boundary/connection 时，只能产生待确认
  suggestion/diagnostic，不得静默以坐标或遍历次序生成可发布身份。

### 7.3 v1 canonical tuple

`entityKind` 是强制 domain separator：

| 实体                       | `entityKind`           | stable parent/local anchors                                                         |
| -------------------------- | ---------------------- | ----------------------------------------------------------------------------------- |
| RoadCorridor               | `RoadCorridor`         | `corridorKey`                                                                       |
| RoadSection                | `RoadSection`          | `corridorKey`, `sectionKey`                                                         |
| authoring lane             | `AuthoringLane`        | `corridorKey`, `sectionKey`, `laneKey`                                              |
| 道路车道 LaneEdge          | `RoadLaneEdge`         | `corridorKey`, `sectionKey`, `laneKey`, `startBoundaryKey`, `endBoundaryKey`        |
| junction internal LaneEdge | `JunctionInternalEdge` | `junctionKey`, `internalEdgeKey`, `startBoundaryKey`, `endBoundaryKey`              |
| Junction                   | `Junction`             | `junctionKey`                                                                       |
| Movement                   | `Movement`             | `junctionKey`, `movementKey`, `directedEntryApproachKey`, `directedExitApproachKey` |
| ManeuverPath               | `ManeuverPath`         | `movementStableId`, `pathKey`, `entryEdgeStableId`, `exitEdgeStableId`              |

RoadSection lane 可展开为多条 LaneEdge，因此 `laneKey` 不能单独充当 edge identity。
稳定 boundary key 区分同一 lane chain 中的 segment。Junction internal edge 使用
junction 作用域的显式 `internalEdgeKey`，支持多个 ManeuverPath 共享同一 internal
edge。Movement 的 left/straight/right/u-turn 分类是可重算 metadata，不是 identity。

ManeuverPath 是静态 topology identity。Route occurrence 只表达：

```text
route handle + entryRouteEdgeIndex + ManeuverPathHandle
```

occurrence 不获得全局 StableId128；重复 edge/path 由 `entryRouteEdgeIndex` 区分。

### 7.4 canonical byte encoding 与 StableId128

v1 编码不依赖语言对象布局或 JSON：

```text
"LFID"                           // 4-byte magic
u16_le(identity_version = 1)
u16_le(entity_kind)
u16_le(field_count)
repeated {
  u16_le(field_tag)
  u32_le(byte_length)
  exact_field_bytes
}
```

整数固定 little-endian；字符串是符合 external ID 约束的大小写敏感 ASCII bytes，
不 trim、case-fold 或 Unicode normalize。缺失字段与空字符串非法；新增或重解释
字段必须提升 identity version。

| code | `entityKind`           | slug            |
| ---: | ---------------------- | --------------- |
|    1 | `RoadCorridor`         | `corridor`      |
|    2 | `RoadSection`          | `section`       |
|    3 | `AuthoringLane`        | `lane`          |
|    4 | `RoadLaneEdge`         | `road-edge`     |
|    5 | `JunctionInternalEdge` | `internal-edge` |
|    6 | `Junction`             | `junction`      |
|    7 | `Movement`             | `movement`      |
|    8 | `ManeuverPath`         | `path`          |

| tag | field                      | encoding     |
| --: | -------------------------- | ------------ |
|   1 | `authoringNamespaceId`     | ASCII bytes  |
|   2 | `corridorKey`              | ASCII bytes  |
|   3 | `sectionKey`               | ASCII bytes  |
|   4 | `laneKey`                  | ASCII bytes  |
|   5 | `startBoundaryKey`         | ASCII bytes  |
|   6 | `endBoundaryKey`           | ASCII bytes  |
|   7 | `junctionKey`              | ASCII bytes  |
|   8 | `pathKey`                  | ASCII bytes  |
|   9 | `movementKey`              | ASCII bytes  |
|  10 | `directedEntryApproachKey` | ASCII bytes  |
|  11 | `directedExitApproachKey`  | ASCII bytes  |
|  12 | `movementStableId`         | 16 raw bytes |
|  13 | `entryEdgeStableId`        | 16 raw bytes |
|  14 | `exitEdgeStableId`         | 16 raw bytes |
|  15 | `internalEdgeKey`          | ASCII bytes  |

持久化算法：

```text
StableId128 =
  first_16_bytes(BLAKE3(ascii("laneflow.stable-id.v1\0") || canonical_bytes))
```

`\0` 是一个零字节 domain separator。文本形态为
`lfid1_<external-slug>_<32 lowercase hex>`。

### 7.5 算法角色与碰撞处理

- **BLAKE3-128**：唯一持久化 entity identity digest；
- **XXH3-64/128**：仅用于 compiler 进程内 hash table/cache/fingerprint；命中后
  必须比较完整 canonical tuple，不得进入 artifact 引用；
- **XXH64/FNV64**：不作为持久身份；64-bit 碰撞空间不值得换取编译期微小收益；
- **SHA-256**：继续承担 artifact/publication integrity，不与 entity identity 混用。

compiler 维护 `StableId128 -> CanonicalIdentity + owning span` 登记。两个 owning
declaration 产生同一 tuple 返回 `DuplicateCanonicalIdentity`；相同 digest 对应不同
tuple 返回 `IdentityDigestCollision`。不得追加 ordinal、salt 或 suffix 静默修复。

### 7.6 稳定性要求

- sibling 重排、无关实体插入和未跨 identity boundary 的几何微调不改变既有 ID；
- section split、boundary/key 变更只改变可计算的 identity closure；
- canonical bytes/ID 有跨平台 known-vector tests；
- runtime hot state 只保存 typed dense handle；StableId128/字符串/hash 不进 tick。

## 8. Artifact 与 source map

### 8.1 Portable canonical artifact

平台无关、确定性、closed shape，并包含：

- canonical format / identity / constraint versions；
- logical entities、typed ordinals、normalized numeric values；
- topology/geometry/static rule relations；
- canonical digest 与 compiler provenance。

它用于 publication、长期审计、migration、跨实现 validator 和 runtime image
regeneration。它不是 mmap hot layout，不承诺与 Rust struct ABI 相同。

### 8.2 Target runtime image

按 `targetTriple + runtimeImageLayoutVersion + featureSet` 生成：

```text
StaticNetworkImage
  header
  StaticTrafficImage
    hot SoA tables
    CSR adjacency / flat ranges
    precompiled route/path/gate/waiting occurrences
  StaticSpatialImage
    frame/edge-aligned geometry tables
    flat points / cumulative arc / sampling ranges
  warm query tables
  optional cold identity/diagnostic tables
```

设计约束：

- shared immutable bytes，多 CoreWorld 复用；
- hot/warm/cold 分段，server/headless profile 可裁剪 cold/debug data；
- table offset/index 使用 checked `u32`；不保存原生指针；
- verifier 完成后 view 的高频索引是 O(1) 或连续 range traversal；
- schema/Serde/object graph 不是 runtime image ABI；
- 具体 archive/zero-copy library 由安全审计和 benchmark 决定。

### 8.3 Source map 与诊断

source map 以 StableId128/LIR ordinal 为键，保留：

- owning declaration 和 contributing spans；
- canonical identity tuple；
- frontend/module/import provenance；
- compiler pass/constraint version；
- generated relation 的推导链。

诊断对标 rustc：稳定 code、severity、primary/secondary span、原因和可执行建议。
authoring error 指向 source/画布；artifact corruption/version mismatch 面向运维，不
返回 generated JSON 行号。

### 8.4 Semantic diff

PR 审阅不依赖二进制 diff。semantic diff 按 StableId128 报告：

- entity add/remove/rename-display；
- topology reconnect、owner/member 变化；
- geometry/length/tolerance-significant change；
- Gate/Waiting/signal/access behavior change；
- identity closure 变化及原因；
- target image layout-only change（不得伪装成 semantic change）。

## 9. 静态/可变状态和运行时消费

### 9.1 Core

`CoreWorld` 共享 `StaticTrafficImage`，只分配：

- vehicles 与 route generations；
- controller clocks/indications；
- grant/reservation/waiting/parking occupancy；
- command/event/snapshot buffers；
- dynamic Route occurrence metadata。

compiler 预编译 authoring/static initial routes；runtime 新注册的 dynamic Route
继续由 Core 按 image candidate indexes 编译 occurrences，保持 ADR 0017 lifecycle
语义。

### 9.2 Spatial

Spatial 共享 `StaticSpatialImage`。Geometry tables 与 Traffic edge ordinal 在 image
中已对齐，不再建立 `HashMap<EdgeHandle, slot>` 或按 external ID join。Pose batch
仍遵守 ADR 0015 的 canonical f32、frame token、稳定顺序、零分配和失败原子性。

### 9.3 启动 verifier

只检查：

- magic/header/version/target/feature；
- canonical digest/provenance；
- offset/alignment/section bounds；
- table cardinality/range/cross-index；
- numeric structural invariants required for memory safety；
- optional signature/publication integrity。

不重新执行 authoring topology、identity、coverage 或 geometry tessellation。

## 10. 独立验证

```text
source ─> compiler ─> canonical artifact ─> independent validator
                    └> runtime image ─────> runtime verifier

canonical artifact ─> independent image rebuild ─> byte/digest comparison
```

independent validator 不调用 compiler semantic validation。两者可以共享机器可读
枚举、field tags 和约束常量，但 topology/ownership/coverage/geometry/occurrence
算法必须有独立实现或独立 oracle。

验证矩阵：

- identity known vectors、reorder/insertion/metamorphic tests；
- clean/incremental/parallel equivalence；
- compiler vs independent validator differential/fuzz；
- canonical artifact corruption 和 runtime image offset/range fuzz；
- source map completeness 与 diagnostic stability；
- semantic diff golden tests；
- current JSON path 与 target image path behavior/determinism/pose equivalence；
- startup wall time、peak allocation、retained static memory；
- multi-world shared-static memory；
- 10k/100k Core tick 与 Spatial pose；
- #72 的 1M entity offline compile/image-build baseline。

## 11. 版本、发布与供应链

独立版本轴：

```text
authoringFormatVersion
canonicalFormatVersion
identityVersion
runtimeImageLayoutVersion
constraintSetVersion
compilerBuildId
targetTriple
featureSet
canonicalArtifactDigest
```

- authoring/canonical 历史迁移离线完成；
- runtime exact-current/fail-closed，不在 production startup 迁移；
- portable artifact immutable publication 继承 ADR 0011；
- runtime image 可按同一 canonical digest 产生多个 target/profile variant；
- compiler、validator、image builder 的 provenance 必须可审计；
- runtime 不联网解析 schema、artifact 或 toolchain。

源文档是 authoring SSOT。Generated artifact 可以作为 release/CI artifact 或为特定
治理阶段 checked in，但只能由 compiler 生成并由 hash/digest Gate 验证；永远不
允许手改或与 source 竞争 authority。

## 12. 性能架构

### 12.1 编译期

- arena + typed `u32` ordinal，stable sequence；
- XXH3 只作为 tuple/cache candidate fingerprint，命中后 full equality；
- parallel pass 按 deterministic shard/merge；
- incremental invalidation 以 source/module/identity closure 为边界；
- geometry tessellation 与 semantic validation 可并行，但 LIR freeze 单一稳定顺序。

### 12.2 运行时

- SoA/CSR/flat ranges；
- typed dense `u32` handles；
- precompiled candidate/occurrence/reverse indexes；
- hot/cold split 与可选 cold mapping；
- immutable image 共享、mutable arrays per world；
- tick 不做 string/hash/path matching；
- pose 不做 Traffic/Spatial join；
- production load 不做 JSON parse/registry rebuild。

### 12.3 Gate

具体数字由实现 G1 在固定性能机上用 current baseline 冻结，但 Gate 至少覆盖：

- load latency、peak allocation、retained bytes；
- 2/8/32 worlds 的 shared-static scaling；
- 10k/100k Core 与 Spatial 既有上限不得回退；
- dynamic Route compilation 不进入 vehicle tick；
- 1M entity offline compile、incremental rebuild 和 image emission；
- target-specific SIMD/alignment 候选相对 portable/common layout 的收益。

不能用“BLAKE3/StableId128 可能变大”推导 tick 回退：ID 位于 cold/compiler boundary，
tick 只使用 32-bit dense handle。若 cold mapping retained memory 成为问题，使用
feature profile、压缩或外置 source map解决，不缩短持久 identity。

## 13. Crate 与依赖目标

```text
laneflow-format
      ↑             ↑
laneflow-compiler   laneflow-validator
      │
      └─> laneflow-runtime-image <─ image verifier/view
                       │
             ┌─────────┴─────────┐
             v                   v
       laneflow-core       laneflow-spatial
             └─────────┬─────────┘
                       v
              laneflow-adapter-*
```

Core 不依赖 compiler、validator、Serde、filesystem、Spatial 或引擎。Spatial 不依赖
compiler 或引擎。`laneflow-data` 是 current JSON compatibility façade，cutover 后
不再拥有静态 normalization authority。是否合并某些小 crate 是实施组织问题，依赖
方向和职责不能合并。

## 14. 迁移路线

```text
阶段 0  current JSON/Data/Core/Spatial 路径继续生产服役
阶段 1  #291：ADR 0020 + 本设计完成 G1
阶段 2  #292：compiler foundation + Synthetic DSL frontend 纵向闭环
阶段 3  恢复 #282–#285，并用 Synthetic DSL/LIR 生成拓扑密集验证场景
阶段 4  Geometry document frontend + topology/geometry MIR（可与阶段 3 并行）
阶段 5  portable artifact + independent validator + source map/semantic diff
阶段 6  target runtime image + Core/Spatial shared image path
阶段 7  behavior/perf/security cutover Gate
阶段 8  production cutover，移除重复 JSON normalization/registry construction
```

阶段是架构迁移顺序，不是把终态降级为最小方案。每个阶段都必须沿同一个
AST/HIR/MIR/LIR 与 artifact/image contract 前进，不允许先建一个注定废弃的 Core
builder API。

Cutover 前必须证明：

1. current/target 场景的静态语义、tick、event、pose 等价；
2. deterministic artifact/image；
3. independent validator 与 runtime verifier 安全；
4. startup、memory、10k/100k 和 multi-world Gate；
5. publication/migration/source map/semantic diff 可用；
6. fallback/rollback 只切换 artifact 路径，不存在两套可变 authority。

## 15. 风险登记

| 风险                        | 结果                                     | 控制                                                        |
| --------------------------- | ---------------------------------------- | ----------------------------------------------------------- |
| compiler 系统性 bug         | 批量污染全部资产                         | independent validator、differential/fuzz、semantic diff     |
| binary verifier 漏洞        | 不可信 bytes 破坏内存安全                | offset-based format、no native pointers、fuzz/unsafe audit  |
| IR 泄漏 runtime 类型        | backend/target 被 Core object graph 锁死 | LIR target-neutral、emitter 不补语义                        |
| identity 漂移               | 引用、diff、缓存和存档失效               | explicit key、known vectors、closure/metamorphic tests      |
| 增量/并行非确定             | CI/release bytes 漂移                    | clean single-thread oracle、stable merge                    |
| hot/cold 边界错误           | cache/retained memory 回退               | layout benchmark、feature profile、multi-world Gate         |
| current/target 双路径长期化 | 测试矩阵和语义漂移                       | 明确 cutover Gate；target 不受 compatibility façade 塑形    |
| source/generated 双 SSOT    | 手工修改与漂移                           | source-only authority、generated digest Gate、semantic diff |
| 过早选择 archive library    | ABI、安全或 MSRV 锁定                    | 先冻结 contract，再 benchmark/audit                         |

## 16. #291 G1 完成条件

- ADR 0020 明确历史 ADR 的继续有效与取代范围；
- 本文不再存在 L1/L2 或 Core-shaped compiler IR；
- Data/Core/Spatial/Adapter 文档清楚标注 current 与 target；
- #292 已重划为 compiler foundation + Synthetic DSL frontend，并继续保持
  `Blocked by #291`；
- identity v1、artifact/image/version/validation/performance contract 一致；
- 本地 docs links/format/contract checks 通过；
- 外部 clean re-review 无未解决 Major，G1 才可推进。
