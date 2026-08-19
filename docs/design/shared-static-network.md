# 共享静态路网

**文档状态**: Accepted（#300 G1 Pass）<br>
**最后更新**: 2026-08-19<br>
**适用范围**: `laneflow-static-network`、受检 LFCA admission、共享静态路网构建、
Traffic/Identity/Spatial 内存数据、Runtime-facing 访问与资源/性能验收<br>
**关联文档**: `../adr/0025-checked-canonical-network-and-shared-static-network.md`、
`network-compiler.md`、`portable-canonical-artifact.md`、
`compiler-post-emission-check-and-minimal-publication-closure.md`、
`road-editing-source-and-geometry-frontend.md`

## 1. 结论与状态

#300 v1 不交付静态镜像文件或 ABI。它交付一个新目标 crate
`laneflow-static-network`，把受检 LFCA 顺序转换为性能优先、不可变、可由多个 world
共享的 `SharedNetworkRevision`。

本文是已接受的 G1 实现输入。#439 / PR #436 已形成受检 LFCA 到共享路网的基础投影：
根唯一所有权、Identity、LaneEdge、Spatial，以及把普通后继与 `ManeuverPath.edges`
合并成完整可执行 CSR 和带 path/transition/gate/waiting 上下文的机动候选。#440 单独闭合
剩余 Runtime 静态关系，#441 在最终字段集合与 #301 production kernel 上单独记录资源/
性能证据；#300 保持父级跟踪项，不由 #436 自动关闭。目标 Runtime 仍不存在，当前生产
路径仍是 Traffic v0.10 / SpatialPackage v0.1 / Data / Core。

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

`CheckedCanonicalNetworkInputV1<'a>`（目标名称）定义在 `laneflow-format`，而不是
`laneflow-static-network`。这样 format 可以构造不可伪造的能力，同时保持
static-network → format 的单向依赖。语义形状为：

```rust
pub struct CheckedCanonicalNetworkInputV1<'a> {
    // all fields private to laneflow-format
    value_checked_lfca: ValueCheckedObjectView<'a>,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
}
```

该草图冻结语义，不承诺最终 Rust 字段或 lifetime 拼写。`laneflow-format` 至少提供两条
构造路径：

```rust
pub fn check_canonical_network_input_v1(
    lfca: &[u8],
    limits: FormatLimits,
) -> Result<CheckedCanonicalNetworkInputV1<'_>, CanonicalNetworkInputError>;

impl<'a> PostEmissionCheckedBundleV1<'a> {
    pub fn canonical_network_input(self) -> CheckedCanonicalNetworkInputV1<'a>;
}
```

单对象函数服务已通过宿主 admission 的发布 LFCA；bundle accessor 服务同进程 compiler
候选。两者必须复用同一内部 LFCA 检查/绑定实现。能力必须满足：

- object kind 精确为 LFCA v1；
- digest 和 exact length 从实际 bytes 计算；
- `NetworkRevisionId` 已从 LFCA semantic payload 重算并与 claim 比较；
- view 已通过 `laneflow-format` registry 和直接值域检查；
- 字段私有、无调用方传入 view/digest/revision 的公共构造器；公开 accessor 不授予
  重建该 capability 的能力。

`ValueCheckedObjectView` 本身不证明跨表引用、row ordering 或真实性，因此不能直接作为
共享静态路网成功结果。`laneflow-static-network` 必须继续完成 §7 的构建闭合；发布内容
是否被产品/宿主接受，则由 LFCP/manifest admission 在调用前决定。

该构建闭合是 Runtime 结构闭合，不是独立 compiler 语义复验。compiler 拥有
`identityFields -> StableId128` 和规范 points -> segment 派生值；builder 接受受检 LFCA
中已声明的 StableId、length/cumulative/tangent/up，只检查 Runtime 索引、引用、范围和
component 结构所需的不变量。它不重新哈希 Identity 前像，也不从 points 重演完整几何
冻结。发布路径以先行 LFCP/manifest admission 为前提，本地编辑路径以同进程
`PostEmissionCheckedBundleV1` 为前提。

### 3.2 两类来源，同一 builder

```text
发布资产 exact LFCA
  -> LFCP v2 / authenticated manifest admission
  -> laneflow-format checked binding
  ┐
  ├-> CheckedCanonicalNetworkInputV1
  │   -> count -> allocate -> fill -> closure
  │   -> SharedNetworkRevision
  ┘
RoadEditingState
  -> compiler -> in-memory LFCA
  -> PostEmissionCheckedBundleV1
```

本地道路编辑不安装 LFCA/LFSM/LFSD/LFCP，不调用 content store/manifest，也不把 LFCA
写入存档。构建成功结果不借用任何输入 backing；runtime-only 调用方可以释放 LFCA。
可编辑 session 为后续 `PortableDiffBase::Artifact` 保留的 exact LFCA 由 editor/#302 作为
`EditableDiffBase` 单独拥有，不进入 `SharedNetworkRevision`，也不改变 builder 的借用边界。

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
- StaticRoute 及 route/path/gate/waiting occurrence；
- Runtime 需要的反向关系、candidate ranges 与 worker-count-neutral execution constraints；
- Traffic/Spatial 共用的 edge/frame ordinal contract。

显示名、来源位置、规范身份 field-tag/value 前像、LFSM/LFSD 和 compiler provenance 不进入
Traffic retained data。

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
  cumulative arc/segment/sampling ranges，以及按 LFCA v1 冻结容差谓词闭合的 Traffic
  edge/length；
- 独立的 FacilityBand geometry 子集与连续 points/ranges。

`LaneEdgeGeometry` 为空而 profile/frame/`FacilityBandGeometry` 任一存在是合法状态；此时
`lane_pose() -> None`，不能伪造空的全覆盖 edge ranges。`LaneEdgeGeometry` 非空时才要求与
Traffic LaneEdge 同基数、同 ordinal；frame 必须精确闭合，长度必须复用 LFCA v1
[附录 A.1](portable-canonical-artifact.md#a1-lfca-table-registry) 的冻结谓词：
`abs(edgeLength - f64(arcLength)) <= max(0.01 m, 1.0e-6 * max(edgeLength,
f64(arcLength))) + 0.0 m`。不得把它收紧为 `f64` Traffic length 与 `f32` geometry arc length
精确相等。需要车辆位姿的消费者通过
`require_lane_pose()`/等价 capability check 取得稳定的 unavailable 错误；facility-only、
profile-only、frame-only LFCA 本身仍可成功构建。编辑器预览 geometry 不属于该 component。

## 7. 构建算法与闭合

### 7.1 阶段

```text
CheckedCanonicalNetworkInputV1
  -> admission / options / limits
  -> pass A: exact logical counts and budgets
  -> allocate final columns and flat payloads
  -> pass B: fill in canonical ordinal order
  -> cross-table / identity / Traffic-Spatial closure
  -> seal owned components
  -> SharedNetworkRevision
```

pass A/B 都按 LFCA wire order 线性遍历，不使用保留的 O(n) ordinal random-access API 形成
O(n²) 构建。实现可以融合不影响精确预算或错误语义的子 pass，但不得增加第二个 projection。

### 7.2 必需闭合

#439 只完成其 Issue 明列的基础投影；下列尚未投影的 owner/member、access/profile、signal、
parking、StaticRoute/occurrence 等 Runtime 必需关系由 #440 逐项盘点并闭合。实体计数存在不
等于字段或关系已经进入 Runtime。

成功前至少检查：

- section/table/row kind 与 expected LFCA registry 一致；
- canonical row key/ordinal 严格排序且无重复；
- typed ordinal/count/range 全部适配 `u32` 并用 checked arithmetic；
- entity、owner/member、topology、route occurrence 和 static-rule 引用落在正确 typed domain；
- `CanonicalIdentity` 与 22 种稳定实体/可寻址派生实体形成完整双射；
- forward/reverse indexes round-trip；
- StaticRoute/occurrence/range 无 gap、overlap 或跨 owner 错配；
- execution contract versions 与派生 constraint graph 一致；
- Spatial presence、edge coverage、frame、长度和 Traffic cross-index 一致；
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

最小 headless、facility-only、profile/frame-only、full lane Spatial 和当前最大合法产品场景
分别记录：

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
| Identity     | 22 种稳定实体与派生实体的声明 StableId 双射、正反 round-trip、同名不同 owner；派生 known vectors 由 compiler 覆盖                            |
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
