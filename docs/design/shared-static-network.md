# 共享静态路网

**文档状态**: Review<br>
**最后更新**: 2026-08-30<br>
**适用范围**: `laneflow-static-network`、受检 LFCA admission、共享静态路网构建、
Traffic/Identity/Spatial 内存数据、Runtime-facing 访问与资源/性能验收<br>
**关联文档**: `../adr/0025-checked-canonical-network-and-shared-static-network.md`、
`network-compiler.md`、`portable-canonical-artifact.md`、
`compiler-post-emission-check-and-minimal-publication-closure.md`、
`road-editing-source-and-geometry-frontend.md`

## 1. 结论

`laneflow-static-network` 把受检 LFCA 顺序转换为性能优先、不可变、可由多个 world 共享的
`SharedNetworkRevision`，不交付静态镜像文件或 ABI。一个根对应一个逻辑路网修订；物理
chunk 对 builder 透明，typed ordinal、StableId 与 CSR 覆盖完整逻辑表，不出现
`(chunk,row)` 公共身份。交通热列以 `u32` mm 与 mm/s 为权威；有折线时边长从弧长 round
并写回 IR，headless 使用准入毫米，不从 Spatial 反推。

## 2. 职责与依赖

| 组件                      | 拥有                                                                                                   | 不拥有                                                        |
| ------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------- |
| `laneflow-format`         | LFCA framing/registry/value checks、后发射 digest/length/revision binding、字段私有受检输入 capability | Runtime layout、每世界状态                                    |
| `laneflow-static-network` | Runtime 结构闭合、typed dense data、identity 索引、planning hints、可选 Spatial、共享生命周期          | compiler 派生复验、来源/LIR 语义、文件发布、cutover、动态状态 |
| `laneflow-runtime`        | fixed tick、参与单元、动态通行定义、每世界状态与执行计划                                               | LFCA 解析、静态 normalization、Spatial geometry               |
| `laneflow-spatial`        | 规范位姿采样和 Spatial session scratch/output                                                          | Traffic authority、道路编制状态                               |
| Adapter/宿主              | LFCP/manifest 认证、资产/存档 I/O、引擎生命周期和表现                                                  | 静态路网语义、tick authority                                  |

依赖箭头表示左侧依赖右侧：

```text
laneflow-compiler ──────────────> laneflow-format
laneflow-static-network ────────> laneflow-format
laneflow-static-network ────────> laneflow-static-contract
laneflow-runtime ───────────────> laneflow-static-network
laneflow-spatial ───────────────> laneflow-static-network
Adapter ────────────────────────> runtime/spatial public API
```

不得让 `laneflow-static-network` 依赖 current `laneflow-core`、`laneflow-data`、
`laneflow-compiler`、Adapter 或文件系统。

## 3. 构建输入与 admission

### 3.1 单一规范输入

`CheckedCanonicalNetworkInput<S>` 定义在 `laneflow-format`，而不是
`laneflow-static-network`。这样 format 可以构造不可伪造的能力，同时保持
static-network → format 的单向依赖。语义形状为：

```rust
pub struct CheckedCanonicalNetworkInput<S> {
    // all fields private to laneflow-format
    checked_lfca_source: S,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
}
```

该草图冻结语义，不承诺最终 Rust 字段或 lifetime 拼写。`laneflow-format` 至少提供两条
构造路径：

```rust
pub fn check_canonical_network_input<S>(
    lfca: S,
    limits: FormatLimits,
) -> Result<CheckedCanonicalNetworkInput<S>, CanonicalNetworkInputError>
where
    S: BoundedReReadableObjectSource;

impl<L, M, D> PostEmissionCheckedBundle<L, M, D> {
    pub fn canonical_network_input(self) -> CheckedCanonicalNetworkInput<L>;
}
```

单对象函数服务已通过宿主 admission 的发布 LFCA；bundle accessor 服务同进程 compiler
候选。两者必须复用同一内部 LFCA 检查/绑定实现。能力必须满足：

- object kind 精确为 LFCA；
- digest 和 exact length 从实际 bytes 计算；
- `NetworkRevisionId` 已从 LFCA semantic payload 重算并与 claim 比较；
- 来源已通过 `laneflow-format` framing、registry、chunk 与直接值域检查；
- 字段私有、无调用方传入 source/digest/revision 的公共构造器；公开 accessor 不授予
  重建该 capability 的能力。

#### LFCA admission

只承认 LFCA/LFSM/LFSD `4/3/3`：受检输入字段私有，digest / exact length /
`NetworkRevisionId` 闭合；
object kind 精确为 LFCA，chunk directory、chunk digest、连续逻辑范围与当前 registry
全部预检。LFSM `canonicalArtifactFormatVersion` 必须等于所绑 LFCA 的
`formatVersion` / `canonicalFormatVersion`。reader 只接受一组 exact versions。
`FormatLimits` 是 wire hard limit 之外的调用方对象、chunk、输出、scratch 与 peak budget；
正式产品 profile 必须覆盖一百万现实混合静态实体门禁。
公开 API 不带世代后缀，不得把米列读成毫米。

对象预检结果本身不证明跨表引用、row ordering 或真实性，因此不能直接作为共享静态
路网成功结果。`laneflow-static-network` 必须继续完成 §7 的构建闭合；发布内容
是否被产品/宿主接受，则由 LFCP/manifest admission 在调用前决定。

该构建闭合是 Runtime 结构闭合，不是独立 compiler 语义复验。compiler 拥有
`identityFields -> StableId128` 和规范 points -> segment 派生值；builder 接受受检 LFCA
中已声明的 StableId、length/cumulative/tangent/up，只检查 Runtime 索引、引用、范围和
component 结构所需的不变量。它不重新哈希 Identity 前像，也不从 points 重演完整几何
冻结。发布路径以先行 LFCP/manifest admission 为前提，本地编辑路径以同进程
`PostEmissionCheckedBundle` 为前提。

### 3.2 两类来源，同一 builder

```text
发布资产 exact LFCA
  -> LFCP v2 / authenticated manifest admission
  -> laneflow-format checked binding
  ┐
  ├-> CheckedCanonicalNetworkInput
  │   -> count -> allocate -> fill -> closure
  │   -> SharedNetworkRevision
  ┘
RoadEditingState
  -> compiler -> in-memory LFCA
  -> PostEmissionCheckedBundle
```

本地道路编辑不安装 LFCA/LFSM/LFSD/LFCP，不调用 content store/manifest，也不把 LFCA
写入存档。构建成功结果不借用任何输入 backing；runtime-only 调用方可以释放 LFCA。
可编辑 session 为后续 `PortableDiffBase::Artifact` 保留的 exact LFCA 由 editor/#302 作为
`EditableDiffBase` 单独拥有，不进入 `SharedNetworkRevision`，也不改变 builder 的借用边界。

两条路径都只接受 `formatVersion = 4` 的受检输入。

## 4. 根修订与 component

目标公共形状：

```rust
pub struct SharedNetworkRevision {
    origin: CanonicalNetworkOrigin,
    traffic: SharedTrafficNetwork,
    identity: SharedIdentityIndex,
    planning_hints: PartitionPlanningHints,
    spatial: Option<SharedSpatialNetwork>,
}
```

`CanonicalNetworkOrigin` 保存 LFCA digest、exact length、revision derivation version、
`NetworkRevisionId`、Runtime 恢复/切换需要的静态 contract versions，以及不进入 LFCA/
`NetworkRevisionId` 的 `partitionPlanningHintsDerivationVersion`。它不是信任锚，也不是可
序列化 descriptor。digest/length 服务来源审计和同字节快速路径；语义兼容以
`NetworkRevisionId` 及 #302 冻结的 contract/identity closure 为准。

根和 component 的字段私有。公共 API 至少提供：

- `network_revision()`；
- `canonical_origin()` 的只读 fixed metadata；
- `traffic()`、`identity()`、`planning_hints()`；
- `spatial() -> Option<&SharedSpatialNetwork>`；
- component 的计数、typed handle/range 和批量 slice accessor。

只有 `Arc<SharedNetworkRevision>` 是可独立保留、克隆和跨 worker 传递的共享所有权句柄。
component 由根直接拥有；公共 accessor 只返回绑定根生命周期的共享借用，不返回 component
`Arc`、owned component 或可重新组合的 token。component 不公开构造器或 `Clone`，调用方因此
不能替换单个 component、延长单个 component 的所有权、修改数组或构造跨修订组合。若异步
Spatial/诊断任务需要延长生命周期，必须克隆根 `Arc`，再从该根借用对应 component。

根类型与只读借用必须是 `Send + Sync`；具体 auto-trait、accessor 返回类型以及 component
不可独立取得 owned value 的 API 形状由 G2 compile-time/API tests 固定。

## 5. `SharedTrafficNetwork`

### 5.1 逻辑内容

Traffic component 必须提供以下 Runtime 静态事实：

- typed entity counts 与 dense handle domain；
- LaneEdge/owner/member/topology 和 successor/predecessor ranges；
- Junction/Movement/ManeuverPath/Gate/WaitingZone/StopLine 静态关系；
- speed、access、vehicle profile、signal/parking 等已支持静态规则；
- 机动路径、门、等待区及其转移候选（路线出现项不进共享根，由
  `register_route` 编进每世界表；ADR 0029）；
- Runtime 需要的反向关系、candidate ranges 与 worker-count-neutral execution constraints；
- Traffic/Spatial 共用的 edge/frame ordinal contract。

显示名、来源位置、规范身份 field-tag/value 前像、LFSM/LFSD 和 compiler provenance 不进入
Traffic retained data。#439 已交付其中的 Identity 基数、LaneEdge 热列/可执行 CSR 与
机动候选；其余字段与关系的逐项归属由 [§13](#13-440-剩余-runtime-关系闭包) 冻结，不得用
本节总清单代替 #440 G1。

### 5.2 默认物理策略

- 热字段独立连续 column；
- 一对多关系用 `RangeU32 { start, len }` + flat payload；
- typed handle 是 `#[repr(transparent)]` 的 `u32` 新类型或等价零开销表示；
- count/range 创建时 checked，成功对象内不保存无效 sentinel；
- 静态枚举/flag 使用最窄且可直接比较的封闭表示；
- 逐 tick 批量访问必须可借用连续 slice；
- 单元素便利 accessor 必须可内联，不能隐藏 hash、scan 或 allocation。

AoS、SoA 和 AoSoA 的精确组合不在 G1 形成 ABI。G2 必须先以 SoA/热冷分离为基线；若
选择 AoS/AoSoA，须用 #301 相同 production access kernel 说明 cache/SIMD 收益和 retained
memory 代价。

## 6. Identity、planning hints 与 Spatial

### 6.1 `SharedIdentityIndex`

每个稳定声明/可寻址派生实体必须恰有：

- typed ordinal → `StableId128` 正向表；
- 按 `(EntityKind, StableId128)` 严格排序的反向表；
- round-trip 双射。

这里的双射以 LFCA 中由 compiler 声明的 `StableId128` 为键。builder 必须核对
`CanonicalIdentity` 与对应实体行的 kind/typed ordinal/StableId 一致，但不消费
`identityFields` 重新编码或哈希；Identity v1 派生算法由 compiler known vectors 与
后发射/端到端测试负责。

反向查找允许 binary search 或经证据支持的紧凑索引；不得默认使用每 world `HashMap`。
身份索引不进入 steady tick，但不能从 headless 或 Spatial 构建模式裁掉。

### 6.2 `PartitionPlanningHints`

LFCA v1 不保存分区提示 payload。`laneflow-static-network` 是 v1 提示的唯一派生 owner：
它按 `partitionPlanningHintsDerivationVersion` 标识的确定性函数，从受检 LFCA 的规范关系、
拓扑和 execution contract 派生 worker-count-neutral 资源/边界/成本信息。compiler 只拥有
并发所需的静态语义与规范关系，不再从 LIR 向不可见的旁路传递提示。

提示不是语义权威：Runtime 可以忽略或按同一根中的静态数据重建，但不能把最终
partition/worker assignment 写回共享对象。相同 LFCA、相同 derivation version 和相同构建
选项必须产生 accessor-visible 内容相等的提示；算法升级提升 derivation version，但不改变
LFCA、`NetworkRevisionId` 或存档兼容性，也不得改变精确执行结果。

### 6.3 `SharedSpatialNetwork`

空间保留构建选项 `SpatialBuildOption` 只区分 `Omit` 与 `RetainAvailable`。
`SpatialBuildOption::Omit` 不分配 geometry retained arrays；`RetainAvailable` 在 LFCA
`spatialPresent=0` 时返回 `None`，为 `1` 时返回
`Some(SharedSpatialNetwork)`，但 presence 不等同于车道位姿能力。component 至少表达：

- direction profile、canonical frame 与实际存在的 geometry capability；
- 可选 lane-pose 子视图：edge-aligned geometry handle、flat canonical `f32` points、
  cumulative arc/segment/sampling ranges，以及按 LFCA（`formatVersion = 4`）
  冻结容差谓词闭合的 Traffic 边长；
- 独立的 FacilityBand geometry 子集与连续 points/ranges。

`LaneEdgeGeometry` 为空而 profile/frame/`FacilityBandGeometry` 任一存在是合法状态；此时
`lane_pose() -> None`，不能伪造空的全覆盖 edge ranges。`LaneEdgeGeometry` 非空时才要求与
Traffic LaneEdge 同基数、同 ordinal；frame 必须精确闭合。长度先把
`lengthMillimetres` 换成米，再复用
[附录 A.1](portable-canonical-artifact.md#a1-lfca-table-registry) 的冻结谓词：
`abs(length_mm/1000 - f64(arcLength)) <= max(0.01 m, 1.0e-6 * max(length_mm/1000,
f64(arcLength))) + 0.0 m`。不得要求整数毫米边长与 `f32` 弧长在米制上精确相等。需要车辆位姿的消费者通过
`require_lane_pose()`/等价 capability check 取得稳定的 unavailable 错误；facility-only、
profile-only、frame-only LFCA 本身仍可成功构建。编辑器预览 geometry 不属于该 component。

## 7. 构建算法与闭合

### 7.1 阶段

```text
CheckedCanonicalNetworkInput
  -> admission / options / limits
  -> pass A: exact logical counts and budgets
  -> allocate final columns and flat payloads
  -> pass B: fill in canonical ordinal order
  -> cross-table / identity / Traffic-Spatial closure
  -> seal owned components
  -> SharedNetworkRevision
```

算法起点是 `formatVersion = 4` 的受检输入。不得靠 `formatVersion` 隐式把米列当成毫米。

pass A/B 都按 LFCA wire order 线性遍历，不使用保留的 O(n) ordinal random-access API 形成
O(n²) 构建。实现可以融合不影响精确预算或错误语义的子 pass，但不得增加第二个 projection。

### 7.2 必需闭合

builder 必须闭合 owner/member、access/profile、signal、parking 与冲突静态关系。路线出现项
只在每世界 `register_route` 编译。实体计数存在不等于字段或关系已经进入 Runtime。

成功前至少检查：

- section/table/row kind 与 expected LFCA registry 一致；
- canonical row key/ordinal 严格排序且无重复；
- typed ordinal/count/range 全部适配 `u32` 并用 checked arithmetic；
- entity、owner/member、topology 和 static-rule 引用落在正确 typed domain；
- `CanonicalIdentity` 与连续 kind `1..=23` 的稳定实体形成完整双射；
- forward/reverse indexes round-trip；
- 机动路径/门/等待区 range 无 gap、overlap 或跨 owner 错配；
- execution contract versions 与派生 constraint graph 一致；
- Spatial presence、edge coverage、frame、可执行连接端点 gap、长度和 Traffic cross-index 一致；
- 输出 logical/retained budget 没有 checked overflow。

这些检查保护 Runtime 数据结构，不重新执行来源、HIR/MIR/LIR 的编译语义，也不建立
第二套 validator/receipt/证明平台。特别地，builder 不从 `identityFields` 重算 StableId，
不从 points 重算每个 segment 的 length/cumulative/tangent/up；compiler 对这些声明值的
派生正确性由 known vectors、后发射检查和 compiler -> LFCA -> shared-network 端到端测试
覆盖。builder 仍须拒绝会让已构建 Runtime 索引、范围、CSR、Traffic/Spatial component
内部自相矛盾的输入。

### 7.3 失败与取消

错误至少稳定区分：

- input kind/binding/contract mismatch；
- Spatial presence/table contradiction 或调用方要求的 lane-pose capability unavailable；
- count/range/arithmetic overflow；
- reference/identity/order/closure mismatch；
- caller output/retained/scratch budget exceeded；
- cancelled；
- allocation failure（按 Rust/平台可稳定表达的边界）。

任一错误都不返回部分 component 或根 `Arc`、不修改现有 revision、不写文件或安装对象。
取消点可以放在 pass 边界和有界批次之间；#300 不拥有编辑器任务队列或调度策略。

## 8. 所有权、并发与切换接口

一个 world 保存 `Arc<SharedNetworkRevision>`；Runtime worker 在 tick 开始前取得该修订的
借用，整个 tick 只访问同一个 revision。Runtime、Spatial 与 Adapter 不接受可独立安装的
component handle：Traffic/Identity/Spatial 访问从同一根借用，或从 #302 发布的、内部保留
该根 `Arc` 的 `RuntimeRevisionSnapshot`/等价只读 facade 借用。旧 snapshot 可以继续完成在途
读取，但其动态状态、Traffic 与 Spatial 始终绑定自己的旧根，不能与新根的 component 配对。
component 内没有锁、原子字段或 interior mutability；运行时可变 arrays、控制器时钟、占用
和执行计划全部 per world。

#300 不实现 cutover。它只保证候选是完全拥有、不可变、可独立丢弃的对象，并暴露：

- origin/revision/contract metadata；
- Identity round-trip；
- Traffic/Spatial component compatibility。

#302 在固定步进安全边界把 root handle、迁移后的动态状态、对应
`CommittedNetworkSource` 和 editable 来源必需的 optional `EditableDiffBase` binding 作为一个
活动修订 aggregate 原子替换；玩家确认建造时该 source 是 committed `RoadEditingState`，
发布修订更新时可以是 `PublishedLfcaReference`。它只从该 aggregate 发布 revision-bound
snapshot/facade，并负责事件恰一次和旧修订退休；旧修订在最后一个 Runtime/Spatial/Adapter
根 borrow/`Arc` 退出后释放。

## 9. 道路编辑与保存

```text
Working RoadEditingState + editor preview
  --用户确认建造-->
compiler -> in-memory LFCA -> checked input -> candidate SharedNetworkRevision
  --#302 成功提交-->
CommittedRoadNetwork {
  CommittedNetworkSource = RoadEditingState | PublishedLfcaReference,
  Arc<SharedNetworkRevision>,
  Optional<EditableDiffBase> = exact LFCA when source is RoadEditingState
}
```

- preview 不触发 LFCA/shared-network build；
- 确认建造触发一次完整候选；
- 当前 revision 在候选期间继续运行；
- G1 不假设构建慢于玩家下一次确认，也不设计高频 FIFO/latest-wins 系统；
- 保存只读取已经进入活动 Runtime aggregate 的 `CommittedRoadNetwork`；working/candidate
  不保存，也不序列化共享数组；
- `CommittedRoadNetwork` 的来源是 `CommittedNetworkSource`：可编辑世界保存 committed
  `RoadEditingState`；只从发布 LFCA 启动的 runtime-only 世界保存宿主作用域的不透明持久
  asset key，并连同 LFCA digest/length/`NetworkRevisionId` 形成 `PublishedLfcaReference`。
  key 不充当信任锚，存档不复制 LFCA，加载必须重新认证现有资产；
- 发布资产若要启用道路编辑，宿主必须同时提供对应的 committed `RoadEditingState`，重新
  编译后以 `NetworkRevisionId`/contract closure 证明语义相符，再由 #302 执行同修订来源
  rebase：从重编译 exact LFCA 构建新根，原子替换 root/source/diff-base binding，而动态状态
  保持不变。rebase 成功前不得启用编辑，也不允许从 LFCA 逆向猜测 authoring curve/source
  state；
- editable 活动 aggregate 在 editor/#302 侧保留与当前根 `CanonicalNetworkOrigin` 精确一致的
  `EditableDiffBase` LFCA bytes。下一次发射必须把它传给 `PortableDiffBase::Artifact`；候选
  提交后 target LFCA 原子成为新 diff base，失败则继续保留旧 base。缺失或 origin 错配时
  不得发射/提交 LFSD；
- 加载 editable 来源时重新编译和构建；加载 published reference 时重新认证并读取被引用的
  LFCA 再构建。editable 加载先以新编译 LFCA 建立根与 diff base，再按相同 revision/contract
  规则恢复 snapshot；资产缺失/错配时失败关闭，不把 diff base、工作态 LFCA 或内部数组缓存
  写入存档。

## 10. 性能与资源契约

### 10.1 不变量

- O(input rows + relation items + output items) 工作；
- 按逻辑表和 chunk 规范序扫描；chunk 只控制输入窗口，不建立完整 LFCA owned object graph；
- 不形成完整 LFCA/LIR owned object graph；
- 每个最终 column/flat payload 最多一次 reserve，不发生几何级扩容；
- builder scratch 由调用方限额约束；
- 成功后静态 retained data 与输入 backing 分离；是否为 editable diff 保留 exact LFCA 由
  `CommittedNetworkSource`/cutover owner 决定；
- 多 world 只增加固定 root handle/per-world mutable state，不复制 N-byte static payload；
- 无逐实体 `Arc`、`Box<dyn Trait>`、字符串、hash table 或 runtime validation registry。

安全 Rust 可以使用 `Vec::with_capacity`/预填充/`into_boxed_slice` 等一次 reserve 路径；
不得为了未证明的 allocator 峰值收益引入手写 `unsafe` 初始化。报告区分 logical bytes、
requested capacity、累计分配、returned retained bytes 和进程 RSS，不能互相冒充。

### 10.2 一次性交付测量

本节由 #441 独立验收；它不得反向扩大 #439 的功能范围，也不得在 #440 的最终静态字段集合
或 #301 production traversal kernel 就绪前用临时布局冒充最终证据。

最小 headless、facility-only、profile/frame-only、full lane Spatial 和最大合法产品场景
分别记录；必须按 `10000` / `100000` / `1000000` 个现实混合稳定静态实体
三档执行，同一百万档不能拆成多个 `SharedNetworkRevision`：

- LFCA/LFSM/LFSD exact bytes；
- Traffic/Identity/Hints/Spatial logical 与 retained bytes；
- count、allocate/fill、closure 与总 build 墙钟；
- allocation/reallocation count 与累计字节；
- scratch peak、builder 成功后不借用三对象、失败 retained 为零；
- 发布构建的 LFCA + candidate arrays + scratch，以及可编辑切换的 current root + retained
  base LFCA + target LFCA/LFSM/LFSD + candidate root + scratch 两类共存峰值；若 G2 用窄化
  capability 证明 LFSM/LFSD 可在 builder 前释放，仍须分别报告后发射检查峰值和构建峰值；
- 2/8/32 worlds 的新增 shared-control bytes；
- Identity lookup 和至少一个 #301 production contiguous traversal kernel。

结果保存在 PR/Gate 评论或现有验证参考中，不创建新 schema/benchmark product。数值只作
描述性证据；没有最低产品硬件和完整 #301 kernel 时不声明 Product Pass。

## 11. 测试矩阵

| 类别         | 必需覆盖                                                                                                                                     |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 输入能力     | 非 LFCA、版本/contract/revision/digest/length 错配；单对象与 bundle 两种构造内容等价；字段私有能力不可伪造                                   |
| headless     | Traffic/Identity/Hints 存在，Spatial 为 `None`，geometry retained 为零                                                                       |
| Spatial 变体 | facility-only、profile/frame-only 成功且 `lane_pose=None`；非空 lane geometry 才完整覆盖并可批量采样；长度差处于容差内、恰等于容差和超出容差 |
| 引用合法性   | typed domain 越界、错误 owner、range overflow/gap/overlap、重复 row/key                                                                      |
| Identity     | 连续 kind `1..=23` 的稳定实体声明 StableId 双射、正反 round-trip、同名不同 owner；派生 known vectors 由 compiler 覆盖                        |
| 确定性       | 同一 LFCA + hints derivation version fresh build 内容相等；不比较 Rust padding/地址/字节                                                     |
| 资源         | caller limit、失败无 retained、三对象真实 lifetime、editable base + target bundle + 双 root 峰值                                             |
| 共享         | 2/8/32 worlds 不复制 component payload；per-world mutable arrays 仍独立                                                                      |
| 修订绑定     | accessor 只返回根生命周期借用；无 owned/Clone component；Runtime/Spatial facade 保留同一根                                                   |
| 保存来源     | editable RoadEditingState 与 published LFCA reference 均只绑定活动根；缺失/错配来源失败关闭                                                  |
| 差异基线     | published→editable 同修订 rebase；exact base LFCA 与 root origin 一致；提交成功/失败时 base 原子更替/保留                                    |
| 并发         | `Send + Sync`、多 reader、tick/snapshot 内单 revision；无内部 mutation                                                                       |
| 性能         | SoA/CSR baseline 与 #301 production access kernel；AoS/AoSoA 变更需比较证据                                                                  |

任意原始字节、截断、错位、oversized 和 FieldV1 结构 fuzz 继续由 `laneflow-format` 覆盖；
#300 只补充能够通过格式直接值域但不能构成功能正确 Runtime 结构的 cross-table mutation，
不把自洽伪造输入或独立 compiler 复验扩展为当前产品目标。

## 12. G1 非目标与返回条件

非目标：

- 静态镜像线格式、descriptor、integrity manifest、mmap、cache；
- Runtime tick、Spatial session、Adapter binding 和 #302 cutover 实现；
- Runtime snapshot/save container 格式；
- 局部/增量 shared-network build；
- target-specific SIMD 已选实现；
- G1 数值产品 SLA 或新性能证明框架。

出现以下情况必须返回 G1：

- production Runtime 需要 LFCA 中不存在的新静态语义；
- 需要第二条 LIR → Runtime backend；
- 需要持久化/跨进程 static cache 或稳定 FFI/raw-layout ABI；
- component 不能保持 immutable/`Send + Sync`；
- 为达到目标必须引入手写 `unsafe`、最终 partition/worker assignment 或每世界状态；
- #301 实测证明已冻结 accessor 无法支持性能所需布局演进。

## 13. #440 剩余 Runtime 关系闭包

**文档状态**: Accepted（#440 G1 Pass；G2 实现按本附录闭合）<br>
**适用范围**: `SharedTrafficNetwork` 在 #439 基础投影之后仍需闭合的 LFCA v1
compiled relation / 实体字段；不改变 ADR 0025、§1–§12 的根所有权、admission、
Spatial 基线或性能证据职责。

#439 已完成 G4，Delivery PR #436 已合入 `main`：受检输入、根唯一
`Arc<SharedNetworkRevision>`、22 类实体 Identity 双射、LaneEdge 长度/限速、普通后继
与 `ManeuverPath.edges` 合并后的 successor/predecessor CSR、机动路径/转换/门/等待区
候选，以及可选 Spatial 车道/设施带几何。#440 只补齐 Runtime 后续消费所需、且尚未投影
的静态事实，并提供 production accessor。#441 仍独占系统化性能证据；#301 仍独占 tick /
每世界状态。

### 13.1 已锁定产品决策

1. **覆盖切面 = v0.10 切换完整集。** 投影当前 Core 已经从 Traffic JSON 消费的全部
   静态事实，使 #301 去掉 current JSON 时不必再开第二次静态投影。不投影
   `#282` 等待区占用求解、`#284` 冲突裁决、`#237` 动态车道用途。
2. **热路径不进 UTF-8；设施 kind 用有类型列。** `Movement` 的 directed entry/exit
   approach key 与 `AccessRule.regulation` 的司法管辖/version/source 仍留在
   LFCA/LFSM。`regulation` 是当前 Core 的审计 provenance，v1 不参与准入计算；切换后
   Traffic 不提供等价 `regulation()` 查询。`RoadSection`/`FacilityBand` 必须保留与当前
   `FacilityKind` 等价的紧凑类型列：seed kind 用封闭代码；`x-` 自定义 token 进入冷
   intern 表，行上只存 intern id。不得把原文 `kind_id` UTF-8 做成每行热列。
3. **停车几何全部进 Traffic。** `ParkingSpace` 的入口/出口边与 progress、lateral、
   heading、length、width 进入 `SharedTrafficNetwork`，使 headless 也能完成当前
   Core 已有的泊位绑定。#440 不把泊位 pose 扩进 #439 Spatial 基线。
4. **不新建 Junction approach 实体。** 只闭合 `Junction → Movement → ManeuverPath`
   与 `JunctionInternalEdge`。approach key 不进 Traffic；若日后 `#237`/`#301` 需要
   分组，再用 ordinal 派生。
5. **共享 resolved 准入平面是查询权威。** builder seal 时按当前 Core
   `AccessRegistry` 语义（参与者继承、target 特异性、priority）生成稀疏
   `(edge, class)` 与 `(path, class)` 平面，查询为 O(1)。规则表只保留归因/
   审计所需的胜者 rule handle 与原始 target/effect/classes/priority，不能作为查询时
   全表扫描或每 world 重建的权威。无约束单元必须用与成功对象内无效 handle 区分的
   表示，不得伪造 typed handle。同 effect、同特异性、同 priority 的并列规则，
   **不再承诺 Traffic JSON 声明顺序**：builder 只看见 LFCA，LIR/LFCA 已按 Identity v1
   规范顺序排列 AccessRule。并列归因与歧义配对一律使用 LFCA AccessRule canonical
   ordinal（较小者胜出 / 作为 first_rule）。不得为保留 JSON 顺序而新增 LFCA 字段或
   读取 LFSM。测试必须按 LFCA 表序构造并列规则，不得用 JSON 文件顺序当 oracle。
6. **本附录是关系投影的设计事实源。** 任何已登记关系都必须按本节明确的目标 component
   投影或明确禁止，不得因首个消费 kernel 暂时不用而丢失。

### 13.2 32 个 compiled relation role

角色代码与 `SourceRelationRole` / LFCA A.5 一致。Traffic 关系必须投影为密集 typed handle
与 SoA/CSR/flat range，并在 seal 前闭合；Spatial 关系只进入可选空间 component。

| 角色 | 名称                                 | 归属         | 说明                                                                                     |
| ---- | ------------------------------------ | ------------ | ---------------------------------------------------------------------------------------- |
| 1    | `LaneEdgeSuccessor`                  | Traffic 投影 | 可执行 successor/predecessor CSR；内部边从普通后继中剔除                                 |
| 2    | `RoadCorridorElement`                | Traffic 投影 | `RoadSection` 或 `FacilityBand` 的有序并行成员，不得压成单链                             |
| 3    | `RoadSectionLane`                    | Traffic 投影 | `RoadSection → AuthoringLane`                                                            |
| 4    | `AuthoringLaneEdge`                  | Traffic 投影 | 编制车道覆盖链；准入从 LaneGroup/RoadSection 走到 LaneEdge 的必经边                      |
| 5    | `LaneGroupMember`                    | Traffic 投影 | `LaneGroup → AuthoringLane`；AccessRule 四域包含 LaneGroup                               |
| 6    | `JunctionMovement`                   | Traffic 投影 | `Junction → Movement`；不派生 approach 实体                                              |
| 7    | `MovementManeuverPath`               | Traffic 投影 | `Movement → ManeuverPath`                                                                |
| 8    | `ManeuverPathEdge`                   | Traffic 投影 | `SharedManeuverNetwork` 连续边 range                                                     |
| 9    | `JunctionInternalEdge`               | Traffic 投影 | 路口内部边排他属主，并从普通 CSR 剔除                                                    |
| 10   | `ManeuverPathGate`                   | Traffic 投影 | 机动路径 gate range                                                                      |
| 11   | `ManeuverPathWaitingZone`            | Traffic 投影 | 机动路径 waiting range                                                                   |
| 12   | `StopLineManeuverGate`               | Traffic 投影 | StopLine 及其反向门集合                                                                  |
| 13   | `ParkingFacilityVirtualEntry`        | 领域静态投影 | virtual pool 的有序入口 anchor                                                           |
| 14   | `ParkingFacilityVirtualExit`         | 领域静态投影 | virtual pool 的有序出口 anchor                                                           |
| 15   | `JunctionConflictZone`               | 领域静态投影 | Junction 到 zone 的规范集合                                                              |
| 16   | `JunctionParticipantStream`          | 领域静态投影 | Junction 到 stream 的规范集合                                                            |
| 17   | `SignalControllerGroup`              | Traffic 投影 | 固定时制控制器拥有的信号组                                                               |
| 18   | `SignalControllerPhase`              | Traffic 投影 | 有序相位                                                                                 |
| 19   | `SignalPhaseState`                   | Traffic 投影 | LFCA 编码在 `SignalPhase` 实体 `RecordVector`，不是 A.5 元组；投影为 group+aspect 连续表 |
| 20   | `ManeuverGateSignalGroup`            | Traffic 投影 | 门到信号组的 indication 绑定；不引入法规/冲突权威                                        |
| 21   | `ParkingSpaceFacility`               | Traffic 投影 | 可选停车设施归属                                                                         |
| 22   | `ParkingSpaceEntry`                  | Traffic 投影 | 入口边；progress 见实体字段                                                              |
| 23   | `ParkingSpaceExit`                   | Traffic 投影 | 出口边；progress 见实体字段                                                              |
| 24   | `ParticipantClassExtends`            | Traffic 投影 | 单继承父类                                                                               |
| 25   | `AccessRuleTarget`                   | Traffic 投影 | 仅 LFCA 已冻四域：LaneEdge / LaneGroup / RoadSection / ManeuverPath                      |
| 26   | `AccessRuleParticipantClass`         | Traffic 投影 | 规则选择的参与者类别集合                                                                 |
| 27   | `VehicleProfileParticipantClass`     | Traffic 投影 | 车型唯一类别                                                                             |
| 28   | `CanonicalFrameLaneEdgeGeometry`     | Spatial 投影 | Spatial 基线，不进 Traffic                                                               |
| 29   | `CanonicalFrameFacilityBandGeometry` | Spatial 投影 | Spatial 基线，不进 Traffic                                                               |
| 30   | `ParticipantStreamManeuverPath`      | 领域静态投影 | stream 的唯一 ManeuverPath                                                               |
| 31   | `ParticipantStreamConflictPassage`   | 领域静态投影 | 有序 passage 与 entry/exit payload                                                       |
| 32   | `CanonicalFrameConflictZoneRegion`   | 领域静态投影 | 可选 2.5D region，只进 Spatial                                                           |

### 13.3 实体字段

Identity 正反表覆盖登记表修订 3 的连续 kind `1..=23`。本表只冻结
Traffic retained 标量/向量；
未列出的 UTF-8、身份前像、来源位置一律不投影。

| 实体                | 本切片进入 Traffic 的字段                                                                        | 明确不进入                                                              |
| ------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `RoadCorridor`      | `reference_section`；元素 range（角色 2）                                                        | 无额外显示名                                                            |
| `RoadSection`       | `road_corridor`；车道 range（角色 3）；有类型 `FacilityKind`（seed 代码或冷 intern id）          | 原文 `kind_id` UTF-8 热列                                               |
| `AuthoringLane`     | `road_section`；`edge_chain`；可选 `lane_group`                                                  | 无                                                                      |
| `LaneEdge`          | 长度/限速/CSR、**可选**编制属主、内部边属主与停止线反向                                          | 强制全覆盖属主；一边多条停止线                                          |
| `Junction`          | `movements` range                                                                                | 任何 approach 实体或 UTF-8 key                                          |
| `Movement`          | `junction`；`maneuver_paths` range                                                               | `directed_entry_approach_key`、`directed_exit_approach_key`             |
| `ManeuverPath`      | #439 已有 movement/edges/gates/waiting                                                           | 无                                                                      |
| `ManeuverGate`      | `maneuver_path`、`transition_index`、`stop_line`、signal-control 标签与可选 `SignalGroup`        | 法规或冲突字段                                                          |
| `WaitingZone`       | `maneuver_path`、`entry_gate`、`release_gate`、`max_occupancy`                                   | 占用账本、队列、#282 runtime 状态                                       |
| `StopLine`          | `lane_edge`；机动门 range                                                                        | 无                                                                      |
| `SignalGroup`       | `controller`；机动门反向 range                                                                   | 无                                                                      |
| `SignalController`  | `offset_ms`、`cycle_duration_ms`；group/phase ranges                                             | 感应/自适应程序                                                         |
| `SignalPhase`       | `controller`、`duration_ms`；`(SignalGroup, aspect)` 状态表；seal 派生的累计互斥 `end_offset_ms` | 未冻结的 aspect 扩展；每 tick 对 duration 前缀求和                      |
| `ParkingFacility`   | 车位反向 range、virtual capacity、有序 virtual entry/exit anchors                                | 把 virtual capacity 展开为伪泊位或内部路网                              |
| `ParkingSpace`      | 可选 facility；入口/出口边与 progress；lateral、heading、length、width                           | 编辑器预览几何                                                          |
| `LaneGroup`         | `road_section`；成员 range                                                                       | 动态 lane-use overlay                                                   |
| `FacilityBand`      | `road_corridor` 属主句柄；有类型 `FacilityKind`（seed 代码或冷 intern id）                       | 原文 `kind_id` UTF-8 热列；几何留在 Spatial；不得成为 AccessRule target |
| `ParticipantClass`  | 可选 parent；`depth`、`subtree_enter`、`subtree_exit`                                            | 显示名                                                                  |
| `AccessRule`        | 目标 kind+ordinal、`effect`、参与者类别 range、`priority`；seal 后的共享稀疏准入平面             | `regulation` 全部 UTF-8；查询时扫描规则表                               |
| `VehicleProfile`    | `participant_class` 与全部跟驰数值列                                                             | 显示名                                                                  |
| `CanonicalFrame`    | 仅维持 Identity/基数；几何在 Spatial                                                             | Traffic 不复制 frame 点列                                               |
| `ConflictZone`      | Junction 属主、反向 stream CSR                                                                   | 几何生成的行为成员、运行时 claim/grant                                  |
| `ParticipantStream` | Junction、ManeuverPath、有序 passages、派生 admission Gate 与 Gate coverage                      | 从 Spatial overlap 重新推断 passage                                     |

`ConflictZoneRegion` 只进入可选 `SharedSpatialNetwork`：按 zone ordinal 形成唯一可选 region，
保存 canonical frame、规范 ring 与高度范围；缺失 region 不影响 headless 冲突静态语义。

### 13.4 反向索引与闭合

除 LFCA 实体行上已有的属主标量外，本切片还必须在 seal 前建立并 round-trip 下列
索引。成功对象内不得保存无效 typed handle sentinel。

- `LaneEdge → AuthoringLane`：**可选/稀疏**。合法 LFCA 允许 LaneGraph 边不属于任何
  `AuthoringLane.edgeChain`（RoadSection/LaneGroup 是 overlay，不是全覆盖）。未覆盖边
  必须返回明确缺失，不得拒绝制品或伪造属主。
- `AuthoringLane → LaneGroup`：可选；仅当该编制车道是组成员时存在。
- `LaneEdge → Junction`：**可选**；仅内部边（角色 9）有属主。
- `LaneEdge → StopLine`：**可选一对一**。当前 Core `SignalRegistry::stop_line_for_edge`
  与「一边至多一条停止线」不变量必须保留；缺失为合法，重复属主在 seal 前失败关闭。
  不得靠扫描全部 StopLine 实现该查询。
- `StopLine ↔ ManeuverGate`；
- `ManeuverGate ↔ SignalGroup`（门侧绑定仍可选）；
- `ParkingSpace ↔ ParkingFacility`（facility 仍可选）；
- `Junction ↔ ConflictZone`、`Junction ↔ ParticipantStream`、
  `ManeuverPath ↔ ParticipantStream` 与 `ConflictZone ↔ ParticipantStream passage`；
- 路线出现项反向 **不再建立**（ADR 0029）；机动路径/门/等待区仍按自身 owner range 闭合。

一对多关系使用 `RangeU32 + flat payload`。可选一对一反向使用并行 presence/bitset 或
等价稀疏列，不得用 `0` 冒充有效 ordinal。tick 向 accessor 返回连续 slice 或实体 View；
不得在成功对象上保留哈希表、字符串、全表扫描或重复验证。准入查询对越界 ordinal 返回
缺失，`Unconstrained` 只表示本修订内无适用规则。派生执行索引里，**相位累计边界**
必须从 owner-local slice 在 seal 时计算。路线距离、下一受控门与限速下降转换不在
共享根，由 `register_route` 写入本世界 compiled 表（ADR 0029）。有界距离使用
`Finite(u32)` / `BeyondFinite`，不得把溢出写成非有限浮点。构建期 intern 可用有序表；
成功对象只保留冷 intern 列。失败不返回部分根。

共享准入平面与规则表一起在 seal 前闭合：同一 LFCA 的 `(edge, class)` / `(path, class)`
裁决必须与当前 Core `AccessRegistry` 在 **以 LFCA AccessRule ordinal 为声明序** 时的
unconstrained/decided 语义一致，包括胜者 rule 归因。平面是共享静态数据，不能推迟到
每 world 可变状态。

下列列由已冻结静态事实在 seal 时派生，供 tick 热路径直接借用，不得每 tick 扫描，
也不得复制进每 world 可变状态：

- **信号相位累计边界**：每个 `SignalPhase` 保留控制器内累计互斥 `end_offset_ms`
  （当前 Core `ResolvedSignalPhase.end_offset_ms`）。`populate_runtime_state` 用
  `partition_point` 解析活动相位；末相位 `end_offset_ms` 必须等于 controller
  `cycle_duration_ms`。只留 `duration_ms` 会迫使每 tick 前缀求和或每 world 重建。

路线执行索引 **不**由 seal 生成，也 **不**进入共享根。`register_route` 在本世界
compiled 表物化分段前缀、后缀距离、受控 hop 链与限速下降转换（ADR 0029）。tick 读
本世界索引，边长 / 门 / 限速值仍读共享根热列。有界距离同型（`Finite(u32)` /
`BeyondFinite`）。

`PartitionPlanningHints` 默认保持 #439 的边邻接度数公式。若实现要把路口
边界权值纳入 worker 数无关提示，必须提升
`partitionPlanningHintsDerivationVersion`，并证明不改变精确执行结果；不得把最终
partition/worker 写入共享对象。本 G1 不把提示算法升级当作完成条件。

### 13.5 本切片测试增量

在 §11 矩阵上，#440 Delivery 至少补齐：

- 走廊元素并行属主：同一走廊交错 `RoadSection`/`FacilityBand` 不得被线性化为单类型链；
- 未覆盖 LaneEdge：不属于任何 `AuthoringLane.edgeChain` 的合法边构建成功，编制属主反向
  为缺失；内部边才有 Junction 属主；
- 有类型 `FacilityKind`：seed kind 往返等于当前 Core 解析结果；自定义 `x-` token 只出现在
  冷 intern 表；
- 准入四域、`ParticipantClass` 区间编码与 **resolved 平面**：LaneGroup/RoadSection 目标
  可通过覆盖链落到 LaneEdge；错误域（含 FacilityBand target）失败关闭；unconstrained、
  target 特异性、priority 必须与当前 Core 平面一致；同 effect 并列的胜者/歧义配对按
  LFCA AccessRule ordinal，不得用 JSON 声明序当 oracle；查询不得扫描规则表；
- `LaneEdge → StopLine`：无停止线的边构建成功且反向缺失；一边两条停止线失败关闭；
- 信号 program：controller offset/cycle、phase duration、group aspect 与门绑定
  往返相等；累计 `end_offset_ms` 与 `partition_point` 相位边界精确等于当前 Core；
  不得出现冲突裁决字段；
- 路线执行索引：共享根不含路线；`register_route` 后的距离/受控门查询由每世界
  compiled 边序号证明（ADR 0029）；
- 停车：关系加几何标量在 headless 下可绑定；取消/失败不留根；
- 合法 UTF-8 冷字段（approach key、`regulation`、原文 `kind_id`）存在于 LFCA 但不出现在
  Traffic 热路径 accessor；
- 损坏的跨表引用、错误 owner、range overflow/gap/overlap。

### 13.6 本切片非目标与返回条件

非目标沿用 §12，并额外明确：

- 不重做 #439 的受检输入、Identity、LaneEdge CSR、机动候选或 Spatial 基线；
- 不重算 StableId 前像或 segment 几何派生，不建第二套验证器；
- 不实现 #301 tick / Spatial session / Adapter，不实现 #302 切换；
- 不交付 #441 性能证据；
- 不冻结 ABI、mmap、cache 或平台 SIMD。

除 §12 所列情形外，出现以下情况也必须返回 G1：

- 生产 Runtime 需要把已排除的 UTF-8 冷字段（含 `regulation` 审计查询）或 approach
  实体变成切换完成条件；
- 停车激活证明必须依赖 Spatial 泊位 pose，而不能使用本切片 Traffic 几何列；
- AccessRule 需要第五个 target domain 或时变 overlay 才能完成 v0.10 切换；
- 准入查询必须在 tick/绑定期扫描规则表，或每 world 重建 resolved 平面才能保持当前
  Core 语义；
- 必须把 Traffic JSON 声明顺序而不是 LFCA AccessRule ordinal 当作并列规则归因权威，
  才能完成 v0.10 切换；
- 固定时制相位解析必须每 tick 扫描/累加 `duration_ms`，或每 world 重建累计边界；
- 必须把路线边序列或出现项写回共享根才能完成 tick 或 #302 快照。
