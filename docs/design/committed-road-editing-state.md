# 已提交道路编辑状态与编辑来源

**文档状态**: Accepted（#1116 G1）<br>
**最后更新**: 2026-10-28<br>
**适用范围**: committed `RoadEditingState` 的形状与所属层级、`CommittedNetworkSource` 编辑
来源变体、`EditableDiffBase` 的持有与校验、重编译责任与信任边界、编辑来源在安装 / 恢复 /
切换 / 审计各入口的合同、快照中的编辑来源绑定<br>
**设计入口**: [#1116](https://github.com/illusion-tech/laneflow/issues/1116)<br>
**关联决策**: [ADR 0023](../adr/0023-road-editing-state-and-phased-network-replacement.md)、
[ADR 0025](../adr/0025-checked-canonical-network-and-shared-static-network.md)、
[ADR 0033](../adr/0033-digest-role-and-algorithm-selection-authority.md)<br>
**关联设计**: [`shared-static-network.md`](shared-static-network.md) §8/§9、
[`traffic-runtime-snapshot.md`](traffic-runtime-snapshot.md) §2/§5/§9、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md) §3/§7、
[`traffic-runtime-state-integrity.md`](traffic-runtime-state-integrity.md) §2.2/§4.2、
[`road-editing-source-and-geometry-frontend.md`](road-editing-source-and-geometry-frontend.md) §4/§5、
[`portable-canonical-artifact.md`](portable-canonical-artifact.md) §3

> **层次约定**：§1–§7 只冻结合同与不变量；API 标识符、错误变体名与模块落点登记在
> [§9 G2 落定清单](#9-g2-落定清单实现内容不属-g1-冻结)。本文不新增合同数值；编辑来源的
> 停顿数据是描述性实测，登记于快照合同 §8（#529）。

ADR 0023 / 0025 已经确定：可编辑世界的已提交路网来源是 committed `RoadEditingState`，
可编辑聚合另外保留与当前修订精确一致的 `EditableDiffBase`，二者与共享路网修订、动态
状态在同一安全边界原子晋升。本文补齐这两个对象在生产代码中的形状，以及各入口对它们
的检查义务，使 #529 的编辑来源端到端恢复、editable load 预算与编辑来源认证可以开工。

## 1. 问题与范围

**问题**：

- `CommittedNetworkSource` 现在只有发布形态；切换与快照合同均登记了编辑形态的义务，
  但都以“类型落地后”为前提。
- 编译器的生产前端以单个 `LFRE` 模块为准入单位（`road-editing-source-and-geometry-frontend.md`
  §5），没有“与 Runtime 修订共同提交的模块集合”这一对象。
- `laneflow-runtime` 的生产依赖不含 `laneflow-compiler`，Runtime 无法自行重编译。

**本文冻结**：§2 类型与所属层级；§3 编辑来源变体与 `EditableDiffBase`；§4 重编译责任与
信任边界；§5 各入口合同；§6 快照绑定；§7 资源口径。

**本文不做**：

- `RoadEditingSourceDiff` 与编辑冲突合并（#345）；
- 编辑器 UI、working / candidate 状态管理、跨语言 SDK 与引擎事务封装；
- 改变 `LFRE` schema、编译器前端准入规则或 `NetworkRevisionId` 的定义；
- #529 的端到端恢复测试、预算取证与认证分支实现。

## 2. `RoadEditingState`：形状与所属层级

### 2.1 所属层级

`RoadEditingState` 定义在 `laneflow-runtime`，与 `PublishedLfcaReference` 同层，属于
`CommittedNetworkSource` 的载荷类型。依赖方向保持不变：

- Runtime 只持有、克隆、计算摘要与绑定该对象，**不解码 `LFRE`，也不调用编译器**；
- 编译器不依赖该类型；宿主逐模块借出字节，交给编译器现有的 `LFRE` 模块准入入口；
- 共享静态契约 crate（`no_std`、无依赖）不承载该类型。

### 2.2 形状

`RoadEditingState` 是一组道路编辑模块的有序集合，每个模块包含：

- **来源文档键**：与编译器 `LFRE` 模块准入时宿主声明的 expected source document key
  为同一值；
- **来源字节**：该模块完整的 size-prefixed `LFRE` buffer（`LF-ROAD-EDITING-SOURCE-v4`）。

不进入该对象的内容：显示用来源文本、编译限额、发射 provenance、LFCA / LFSM / LFSD、
HIR / MIR / LIR、共享静态数组、运行时句柄以及任何 working / candidate 状态
（`road-editing-source-and-geometry-frontend.md` §4 的“不得保存为来源权威”清单继续适用）。

只有官方 `LFRE` 前端产生的模块可以进入该对象。合成模块（匿名 AST）只允许用于非发布
测试，不构成可编辑世界的来源。

### 2.3 构造时的结构校验

构造只做 Runtime 自身能够判断的结构检查，任一失败即拒绝，不产生部分对象：

1. 至少包含一个模块；
2. 每个来源文档键非空，模块之间不重复；
3. 每个模块的来源字节非空；
4. 模块按来源文档键的 UTF-8 字节序排列为规范顺序（构造时排序，不接受调用方自报顺序）；
5. 全部容量按可失败预留取得，分配失败返回错误，不中止进程。

键的词法规则、`LFRE` framing / verifier / 版本 / 语义检查，以及模块之间的命名空间与导入
闭合，都只由编译器准入负责。Runtime 不复刻这些规则，以免形成第二套可接受值域。因此
Runtime 能接受的结构是编译器能接受的超集；无法编译的状态不可能取得共享路网修订，
也就无法进入活动聚合（§4）。

### 2.4 道路编辑状态摘要

`RoadEditingState` 在构造成功时计算一次道路编辑状态摘要，此后不可变。

**角色与算法**（[ADR 0033](../adr/0033-digest-role-and-algorithm-selection-authority.md) §3）：

- 角色：内部完整性摘要，认证宿主提供的状态与快照绑定的状态是同一份字节；
- 验证方：LaneFlow 实现（Runtime 快照恢复入口），没有外部重算方；宿主按摘要寻址时
  使用 Runtime 给出的摘要值，不独立重算；
- 持久化范围：快照（`LFRS`）与宿主存档中的寻址键；
- 算法：BLAKE3-256，即该角色的默认算法，无偏离。`LFRE` 不属于可移植规范制品家族，
  不适用该家族保持 SHA-256 的理由。

```text
BLAKE3-256(
  "laneflow:road-editing-state-digest:v1\0"
  || digestVersion:u16 = 1
  || moduleCount:u32
  || for each module in canonical order:
       sourceDocumentKeyByteLength:u32
       sourceDocumentKeyUtf8Bytes
       sourceByteLength:u64
       sourceBytes
)
```

所有整数采用小端序。更换算法或前像即递增 `digestVersion`；1.0 前不保留旧版本的计算或
比对路径。摘要与精确总字节长度（全部来源字节长度之和）一起构成快照中的编辑来源绑定
（§6）。该摘要只绑定“哪一份已提交状态”，不替代 LFSM 的 `sourceCollectionDigest`（后者由
编译器在准入后从模块命名空间计算），也不进入 `NetworkRevisionId`。公开类型按角色命名，
不在名称中写入算法。

§3.2 的差异基线对应关系判据不新增摘要：它重算的是 LFCA 对象精确字节摘要，属于可移植
规范制品家族的互操作摘要，按 ADR 0033 §9 保持 SHA-256，并与修订 origin 中的同一字段
比较。

### 2.5 共享与克隆

来源字节在构造后不可变，以引用计数共享。活动聚合、快照捕获与审计读取都共享同一份
字节，克隆为常数开销、不复制载荷。

## 3. 编辑来源变体与 `EditableDiffBase`

### 3.1 变体形状

`CommittedNetworkSource` 增加编辑来源变体，**同时携带** `RoadEditingState` 与
`EditableDiffBase`：

```text
CommittedNetworkSource =
    Published { PublishedLfcaReference }
  | Editable  { RoadEditingState, EditableDiffBase }
```

把差异基线放进同一变体，是为了让“编辑来源必有且只有一个与当前修订对应的差异基线”和
“发布来源没有差异基线”在类型上无法写错。`EditableDiffBase` 仍不属于可重建来源的持久
内容：快照只绑定 `RoadEditingState`（§6），差异基线在每次恢复时由重编译重新取得。

### 3.2 `EditableDiffBase`

`EditableDiffBase` 持有构建当前共享路网修订所用的 exact LFCA 字节，以引用计数共享、
不可变。它是下一次 LFSD 发射时 `PortableDiffBase::Artifact` 的唯一合法输入
（`shared-static-network.md` §9）。

**对应关系判据**：差异基线字节的 SHA-256 与精确长度，必须分别等于目标修订
`CanonicalNetworkOrigin` 的 LFCA digest 与 exact length。这里比较的是字节身份，比同修订
判据更严格：差异基线必须正是构建该修订的那份字节，而不是同语义的重发布字节。

Runtime 在接收任何编辑来源时都检查对应关系（§5）；失配即失败关闭，旧聚合原样生效。

## 4. 重编译责任与信任边界

**重编译由宿主负责**（或代表宿主的 Adapter 会话）。宿主持有编译器依赖，按以下顺序从
`RoadEditingState` 得到可交给 Runtime 的输入：

```text
RoadEditingState
  -> 逐模块 LFRE 准入（编译器现行入口）-> 编译
  -> 发射 LFCA（及 LFSM / LFSD）-> 后发射检查
  -> 构建共享路网修订
  -> 把 (修订, RoadEditingState, LFCA 字节作为 EditableDiffBase) 交给 Runtime 入口
```

编译限额、发射 provenance 与 `SpatialBuildOption` 都是宿主提供的必选参数，不进入
`RoadEditingState`。

**信任边界**：

- Runtime **不证明** `RoadEditingState` 与共享路网修订的语义对应，因为它无法重编译。
  这项对应关系由完成编译的宿主保证，并由共享路网修订认证的编辑来源分支独立复核
  （完整性设计 §2.2、§4.2；入口 `attest_editable_shared_network_revision`，#529）。
- Runtime **证明**差异基线与修订的字节对应（§3.2），以及快照绑定与宿主提供的状态一致（§6）。

**重编译的可复现范围**：

- LFCA 的 `CompilerProvenance` 含 `compilerBuildId`（`portable-canonical-artifact.md` §3.7），
  因此只有在**同一编译器构建、同一组宿主参数**下，重编译才逐字节相同。
- 跨编译器构建时只保证同修订判据（`NetworkRevisionId` + `networkRevisionDerivationVersion`
  + 契约版本精确相等）。恢复与切换只依赖同修订判据，不要求跨构建的字节一致。
- 共享路网修订认证在进程内复核在用修订时，宿主必须使用构建该修订时的同一组参数；
  参数不一致导致的字节差异属于宿主调用错误，不是修订失真。

## 5. 入口合同

各入口沿用现有的失败关闭语义：任一检查失败都只丢弃局部 staging，旧聚合、旧动态状态、
旧来源与旧差异基线原样生效，零事件。

| 入口                         | 编辑来源的额外检查                                                                                                                                              |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 安装                         | 差异基线与目标修订字节对应（§3.2）                                                                                                                              |
| 快照恢复                     | 差异基线与目标修订字节对应；宿主提供的 `RoadEditingState` 的摘要与精确长度等于快照中的编辑来源绑定；快照来源种类与宿主提供的来源种类一致（§6）                  |
| 同修订换根                   | 差异基线与目标修订字节对应；同修订判据不变                                                                                                                      |
| 跨修订直移                   | 差异基线与目标修订字节对应；LFSD 的 base binding 必须等于活动聚合的修订 origin（现有描述符检查），活动聚合为编辑来源时这等价于“LFSD 以当前差异基线为 base 发射” |
| 共享路网修订认证（编辑来源） | 宿主从活动聚合取出 `RoadEditingState` 重编译，以产出的完整 `origin` 作为期望值（完整性设计 §4.2；`attest_editable_shared_network_revision`，#529）              |

**来源种类的转换**（均经现有切换入口，任一失败都不改变活动聚合）：

| 转换        | 入口                               | 说明                                                                                                                                |
| ----------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| 编辑 → 编辑 | 跨修订直移（确认建造）或同修订换根 | 提交成功后，目标 LFCA 原子成为新的差异基线；失败则保留旧基线                                                                        |
| 发布 → 编辑 | 同修订换根                         | 发布资产启用道路编辑：宿主提供重编译所得、同修订的 `RoadEditingState`，新根对象必须用重编译出的 exact LFCA 构建；成功前不得启用编辑 |
| 编辑 → 发布 | 跨修订直移或同修订换根             | 世界转为 runtime-only，差异基线随旧聚合退休，此后不能发射 LFSD                                                                      |
| 发布 → 发布 | 现行合同                           | 不变                                                                                                                                |

**取出接口**：活动聚合只读借出 `RoadEditingState` 与 `EditableDiffBase`，分别供宿主保存
状态内容、重编译审计与下一次发射使用。借出不改变聚合，也不允许外部替换其中任一成员。

## 6. 快照中的编辑来源绑定

**绑定而不内嵌**：快照对编辑来源只绑定道路编辑状态摘要、精确总字节长度与模块数，不内嵌
`LFRE` 字节；`EditableDiffBase` 照旧禁绑。宿主按摘要寻址持久化状态内容，同一份状态可被
任意多个检查点共享。

这与发布来源对称：发布来源绑定 asset key + digest，宿主持有资产本体；编辑来源绑定摘要，
宿主持有状态本体。

**保存**：

- 捕获只读取活动聚合中已计算的摘要与长度，不复制来源字节，因此不改变现有捕获的可失败
  预留面；
- 宿主负责在保存快照的同时持久化对应的 `RoadEditingState` 内容；
  快照写入成功而状态内容缺失时，恢复会失败关闭（见下）。

**恢复**：宿主按快照中的摘要取回状态内容并构造 `RoadEditingState`，重编译、构建修订后，把
修订、状态与差异基线交给快照恢复入口。在任何交通状态 lowering 之前，Runtime 依次检查：

1. 快照来源种类与宿主提供的来源种类一致；
2. 宿主提供状态的摘要与精确长度等于快照绑定，模块数一致；
3. 差异基线与目标修订字节对应（§3.2）；
4. 现有同修订判据：快照修订与目标修订闭合。

状态内容缺失、摘要失配、重编译失败、版本不匹配或差异基线失配，一律失败关闭，不发布 world。

**容器版本**：编辑来源绑定使 `LFRS` 的来源种类与来源表发生变化，容器 `formatVersion`
随实现从 6 升到 7；`runtime_state_version` 与确定性状态摘要版本不变（来源不进入摘要
前像）。1.0 前不保留 v6 reader（`.agents/skills/laneflow-pre-1-0/SKILL.md`）。

## 7. 资源口径

- **稳态保留**：编辑聚合新增 `RoadEditingState` 的来源字节与 `EditableDiffBase` 的 LFCA
  字节，各一份、以引用计数共享；多个世界共享同一修订时，不复制这两份载荷。
- **保存**：编辑来源的捕获成本为常数，快照体积与发布来源同量级。
- **恢复 / 加载**：editable load 停顿 = 状态读取 + 重编译 + 修订构建 + 快照恢复。前两项
  属于宿主侧，最后一项沿用现有恢复路径。它按快照合同 §8 单独登记，不用 published 数值代替
  （#529）。
- **峰值**：重编译期间的编译器 scratch、LFCA / LFSM / LFSD 与候选修订，按编译器与共享
  静态路网现行限额约束，本文不新增限额维度。

## 8. 未采用的方向

- **有类型道路编辑模型下沉到底层 crate，Runtime 持有有类型值**：需要把编译器前端模型
  拆到共享层，维护面扩大；Runtime 也不需要解读其内容。
- **Runtime 生产依赖编译器**：把编译器带入 Runtime 生产构建，违背现有分层；重编译的
  限额与 provenance 本来就是宿主策略。
- **在 `LFRS` 中内嵌 `LFRE` 字节**：快照自成一体，但每个检查点都要复制一份完整道路
  状态，审计重放的检查点体积与保存停顿都随路网规模增长；按摘要寻址的宿主持久化可以
  让检查点共享同一份状态。
- **差异基线作为聚合的独立可选成员**：与来源种类分离后，“编辑来源缺基线”或“发布来源
  带基线”在类型上可以表示，只能靠运行时检查排除；放进同一变体后不可表示。

## 9. G2 落定清单（实现内容，不属 G1 冻结）

以下为 #1119 落定的实现选择：

- **模块落点与标识符**：`laneflow-runtime` facade 模块 `editable_source` 承载
  `RoadEditingState`、`RoadEditingStateModule`、`InvalidRoadEditingState`、
  `RoadEditingStateDigest`（及 `ROAD_EDITING_STATE_DIGEST_DOMAIN` /
  `ROAD_EDITING_STATE_DIGEST_VERSION`）与 `EditableDiffBase`；变体为
  `CommittedNetworkSource::Editable { state, diff_base }`。均从 crate 根导出。
- **差异基线构造**：`EditableDiffBase::try_new(lfca_bytes, origin)` 计算一次 SHA-256 与
  长度，核对与宿主刚构建出的修订 origin 对应（失配返回 `EditableDiffBaseMismatch`），并记录
  该修订的 `NetworkRevisionId`。因此编辑来源的 `CommittedNetworkSource::network_revision()`
  取差异基线绑定的修订，现有修订比对逻辑不变；各入口再以目标修订 origin 核对字节对应。
- **只读借出**：沿用 `TrafficWorld::committed_source()`，编辑变体的两个成员经只读访问器
  取得，不提供替换入口。
- **错误变体**：`InstallError::EditableDiffBaseMismatch`（安装与快照恢复共用交通准备路径，
  恢复经 `SnapshotRestoreError::Install` 报告）；`CutoverError::TargetEditableDiffBaseMismatch`
  （同修订换根与跨修订直移）；`SnapshotRestoreError::SourceKindMismatch`、
  `SnapshotRestoreError::UnexpectedSourceBinding { field }` 与
  `SnapshotRestoreError::EditableSourceBindingMismatch`。
- **`LFRS` 7**：`SourceKind::Editable = 2`；`EditableSourceBinding { state_digest,
  state_byte_length, module_count }` 作为根表末尾字段 `source_editable`；schema 位于
  `schemas/runtime-snapshot/v7`，README 逐项绑定。
- **宿主重编译辅助入口：不提供生产入口**。`laneflow-bevy` 只在开发依赖中使用编译器，
  生产依赖中也没有同时依赖编译器、`laneflow-format` 与 `laneflow-static-network` 的
  crate；为此让 Adapter 引入编译器生产依赖得不偿失。宿主用现有入口组合重编译路径
  （§4）；`laneflow-runtime` 的编辑来源测试提供这条组合路径，并证明同构建同参数下 LFCA
  逐字节可复现。
- **测试**：构造与规范排序、摘要固定向量、各入口的差异基线对应关系、四种来源转换、
  快照编辑来源往返与拒绝面。认证分支、端到端恢复与 editable load 初值由 #529 交付。
