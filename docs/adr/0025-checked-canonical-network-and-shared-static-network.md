# ADR 0025：受检规范路网与共享静态路网修订

**状态**: Accepted<br>
**日期**: 2026-08-18<br>
**适用范围**: LFCA 到目标态交通运行时/Spatial 的静态数据构建、内存布局、共享生命周期、
不可变路网修订、玩家道路编辑与保存边界<br>
**部分取代**: ADR 0020 中 target/profile-specific 静态镜像文件、稳定镜像 ABI、
`StaticImageDescriptor`、静态镜像完整性清单、mmap/chunk 验证、镜像摘要/长度和
跨 target 镜像变体决定；ADR 0021 中 `TrustedStaticImage`、镜像摘要快照绑定和
“镜像切换”作为当前目标命名的决定。保留编译器拥有静态路网、LFCA、不可变路网修订、
Traffic/Spatial/每世界可变状态分层、稳定身份索引和失败关闭切换原则。<br>

**关联文档**:

- `0020-compiler-owned-static-network-and-static-image.md`
- `0021-city-simulation-game-traffic-foundation.md`
- `0023-road-editing-state-and-phased-network-replacement.md`
- `0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- `0028-integer-millimeter-traffic-geometry.md`
- `0029-retire-precompiled-static-route.md`
- `../design/shared-static-network.md`
- `../design/network-compiler.md`
- `../design/portable-canonical-artifact.md`

> **#496 / ADR 0028**：交通热列为整数毫米。**#498 / ADR 0029**：移除静态路线。对象版本按 #284 格式登记更新，只承认
> `formatVersion = 5` 的受检 LFCA；路网不含 `StaticRoute`。公开 API 不带世代后缀；
> 读器拒绝 `formatVersion != 5`。
>
> LFCA 的 `65,536` 行与 `16,777,216` bytes 是单个规范 table chunk 上限；一个
> `SharedNetworkRevision` 必须支持至少 `1,000,000` 个现实混合稳定静态实体。chunk 只属于
> 可移植 LFCA/LFSM/LFSD 容器，不恢复本文否决的 target-specific 静态镜像、mmap ABI、
> 多修订拼城或 Runtime 分区身份。
> 该容量必须由 `LF-COMP-SINGLE-NETWORK-1M-v2` 从 official source 真实编译到
> `SharedNetworkRevision`；不得只用手工 LFCA 或 reader/builder 测试代替 compiler/emitter。

## 背景

ADR 0020 把可移植规范制品和 target/profile-specific 静态镜像设计为同一 Canonical LIR
的两个持久化后端。目标静态镜像原计划通过稳定 ABI、外部 descriptor、完整性分块、
mmap/借用视图和跨 target 重建，消除生产启动时的重复构建。

#298/#299 随后交付了 LFCA/LFSM/LFSD v1、LFCP v2、共享后发射检查和最小发布闭合。
本 ADR 决策时 LFCA v1 单对象硬上限是 `16,777,216` bytes；当时最高级 P100 生产工作负载的 LFCA
为 `2,286,569` bytes（约 `2.29 MB` / `2.18 MiB`），完整 compiler + emitter 中位数约为 `39 ms`，三对象独立 SHA-256
约为 `2.4..2.7 ms`。这些证据没有证明最低产品硬件上的最终城市加载成本已经通过，
但也不支持在 Traffic Runtime 访问模式尚未实现前先冻结第二套持久化 ABI、目标平台
矩阵和完整性协议。

产品真正需要的是：用户可以加载道路、在编辑器中预览和确认建造、让当前世界继续运行、
在候选成功后切换到新路网修订，并只保存已经进入 Runtime 的道路状态。Runtime 仍需要
SoA/CSR、密集句柄、预计算索引和热冷分离；这些收益不要求相同内存布局必须落盘。

因此 #300 把“目标静态镜像”拆成两个不同问题：

1. LFCA 继续承担持久化、可移植、规范发布和来源审计；
2. 新的共享静态路网修订只承担进程内 Runtime/Spatial 性能数据和多世界共享。

## 决策

### 1. 不交付独立静态镜像制品或 ABI

#300 v1 不定义、写入、发布或读取静态镜像文件，不新增稳定线格式、文件头、节目录、
target/profile selector、镜像摘要/长度、`StaticImageDescriptor` 或
`StaticImageIntegrityManifest`。

同时不交付：

- mmap 或跨进程页共享；
- target-specific static image 的 flat chunk / Merkle 完整性方案；portable table chunk
  与逐 chunk SHA-256 不构成第二套 Runtime 镜像；
- Spatial/冷页按需验证；
- Windows/Linux/ARM/Web 等镜像变体矩阵；
- 静态镜像内容寻址安装或磁盘缓存；
- Rust 原生结构布局、第三方 archive bytes 或 packed arena 的外部兼容承诺。

LFCA/LFCP v2 与宿主认证 manifest 继续是发布内容的持久化和真实性入口。共享静态路网
不是可手改事实源、存档、缓存身份或发布对象。

### 2. 新增进程内共享静态路网层

目标 crate 名称为 `laneflow-static-network`。它位于 `laneflow-format` 与目标
`laneflow-runtime`/Spatial 之间，消费受检 LFCA view，拥有字段私有、不可变的类型化
连续数组，并返回 `SharedNetworkRevision`。

```text
laneflow-compiler ──────────────> laneflow-format
laneflow-static-network ────────> laneflow-format
laneflow-static-network ────────> laneflow-static-contract
laneflow-runtime ───────────────> laneflow-static-network
laneflow-spatial ───────────────> laneflow-static-network
```

`laneflow-static-network` 不依赖 compiler-private LIR/emitter、当前 Core 对象图、Data、
Adapter、文件系统或宿主存档容器。目标 Runtime/Spatial 不解析 LFCA，也不反向拥有静态
normalization。

### 3. 发布加载与玩家编辑共用一条 LFCA 构建路径

共享静态路网构建器只接受由 `laneflow-format` 定义、字段私有的
`CheckedCanonicalNetworkInput` capability。该能力已经完成 framing、registry、
直接值域、实际摘要/长度和 `NetworkRevisionId` 闭合；builder 不接受裸 `&[u8]`、
彼此分离的 view/digest/revision 参数或调用方自报 revision。

两类入口只在 admission 之前不同：

- 发布加载：宿主先认证 LFCP v2 / manifest 绑定，再由 `laneflow-format` 的单对象检查
  对 exact LFCA 建立该 capability；
- 玩家编辑：compiler 在同进程发射 LFCA，并由 `PostEmissionCheckedBundle` 直接派生
  同一 capability，不执行内容存储或磁盘发布事务。

两者随后进入同一个 `laneflow-static-network` 计数、分配、填充和闭合路径。v1 不允许
从 Validated Canonical LIR 直接建立第二个 Runtime backend；如果 LFCA 中转经真实城市
测量成为阻断，必须重新进入 G1 决定融合实现如何保持单一投影。

`laneflow-static-network` 是官方受检 LFCA 的性能型 Runtime loader，不是 compiler 的
独立语义验证器。compiler 继续唯一拥有 `identityFields -> StableId128`、规范点列到
segment length/cumulative/tangent/up 等派生语义；发布 LFCA 的产品接受还必须先经过
LFCP/manifest admission，玩家编辑 LFCA 则来自同进程
`PostEmissionCheckedBundle`。builder 使用已经受检并绑定修订的声明值建立连续 Runtime
数据，只闭合 Runtime 会直接依赖的计数、顺序、引用、范围、双射、Traffic/Spatial
结构关系和调用方预算。它不重新编码 Identity v1 前像、不重新执行 BLAKE3，也不从 points
逐值重演 compiler 的 segment 冻结算法；这些派生的功能正确性由 compiler known vectors、
后发射检查和 compiler -> LFCA -> shared-network 端到端测试负责。

### 4. `SharedNetworkRevision` 物理拆分、逻辑绑定

根对象至少包含：

```text
SharedNetworkRevision
  origin LFCA digest + exact length
  networkRevisionDerivationVersion + NetworkRevisionId
  contract versions
  partitionPlanningHintsDerivationVersion
  required SharedTrafficNetwork
  required SharedIdentityIndex
  required PartitionPlanningHints
  optional SharedSpatialNetwork
```

Traffic、Identity、Planning Hints 和 Spatial 可以在 builder 内物理独立分配，但最终由
同一根修订直接拥有并绑定同一个 `NetworkRevisionId`。公共 API 只允许从根借用 component；
不返回可独立保留、克隆或重新组合的 component 所有权句柄。Spatial 缺失是合法的无图形
构建模式；Spatial 存在不保证 LaneEdge geometry 存在。只有非空 LaneEdge geometry 才必须
完整覆盖规范 edge/frame，并与 Traffic 使用同一 typed ordinal；facility-only、profile-only
和 frame-only Spatial 都是合法能力组合。不得独立安装或切换其中一个 component。

`SharedIdentityIndex` 是所有构建模式必需的冷索引，提供 typed ordinal → `StableId128`
和按 `(EntityKind, StableId128)` 排序的反向查找。它服务动态通行定义、存档/快照恢复和
跨修订迁移，不进入逐交通参与单元 fixed tick。

LFCA 不保存分区提示 payload。`laneflow-static-network` 按根中记录但不进入 LFCA/
`NetworkRevisionId` 的 `partitionPlanningHintsDerivationVersion`，从受检规范关系与 execution
contract 确定性派生 `PartitionPlanningHints`。它保存 worker 数无关、可以忽略或重建的
成本/边界提示，不保存最终 partition、worker assignment、动态负载、world seed 或每世界
执行计划；算法升级只提升该非语义 derivation version，不得改变精确执行结果。

### 5. Runtime 性能是内存布局的第一目标

v1 的默认物理策略是：

- 热字段 SoA；
- 热/温/冷数据分离；
- 一对多关系使用 CSR、flat range 或等价连续表示；
- flag/稀疏布尔状态优先使用有界 bitset；
- 全部 Runtime 静态引用使用类型化密集 `u32` handle/range；
- 静态 route/path/gate/waiting occurrence、反向关系和执行约束预计算；
- tick/pose 热路径不做字符串、StableId、hash、路径匹配、重复结构验证或逐实体动态分派；
- 不形成每实体一个 heap object/`Arc`/trait object 的对象图。

共享边界只使用 `Arc<SharedNetworkRevision>`；根直接拥有 Traffic/Identity/Hints/Spatial
component，component 再独占其内部数组。每个 world 或异步读任务只克隆共享根，随后从
同一根借用 component 和 slices/ranges，不执行逐 component 或逐元素引用计数。Runtime、
Spatial 与 Adapter 的安装 API 不接受独立 component handle；#302 必须从单一活动修订
aggregate 发布绑定同一根的只读 snapshot/facade，避免旧 Traffic 与新 Spatial 组合。

不冻结 Rust 字节布局、字段排列、padding、allocation address、SIMD 宽度或 cache-line
大小。具体 AoS/SoA/AoSoA 分组、alignment、prefetch 和 target-specific SIMD 只能由
#300 构建证据与 #301 production tick/pose kernel 的真实测量决定；调整这些内部实现不
触发资产迁移或格式版本。

### 6. 构建器有界、拥有输出且失败无副作用

构建器执行：

1. 检查调用方 limits、LFCA kind/binding 和所请求 Spatial 模式；
2. 顺序统计各 typed column、CSR payload、identity 和 hint 的精确逻辑数量；
3. 在分配最终数组前用 checked arithmetic 计算调用方可见预算；
4. 为每个最终数组最多保留一次容量并顺序填充；
5. 检查 row key/ordinal、跨表引用、range、声明 StableId 双射、Traffic/Spatial edge
   结构对齐和 execution-constraint closure；
6. 成功后封存为字段私有的 owned slices，并返回根修订。

失败、取消或预算不足不返回部分 component、不修改现有 world、不写磁盘，也不保留
builder registry。成功结果不借用 LFCA/LFSM/LFSD backing；runtime-only 调用方可以在构建
完成后释放 LFCA。可编辑 session 为下一次 `PortableDiffBase::Artifact` 保留的 exact LFCA
由 editor/#302 单独拥有，不进入共享根。

构建期允许同时存在输入 LFCA、最终 component arrays 和有界 scratch，不建立第二棵完整
owned LFCA/LIR 对象图。安全 Rust 实现不得为了“精确一次分配”引入手写 `unsafe`；容量、
累计分配和返回时 retained bytes 必须按实际 allocator/API 行为记录。

上述闭合不增加第二套 compiler 派生实现。尤其不得为了把 Runtime loader 提升为独立
validator，在每次加载或确认建造时重新哈希全部 Identity 前像、重新计算全部 segment
向量或保留验证专用对象图。若未来产品允许未经官方 compiler/admission 的第三方 LFCA，
其额外验证策略由独立 G1 决定，不能无证据进入当前玩家建造延迟。

### 7. 道路编辑只在确认建造后构建候选

鼠标拖动、曲线调整和编辑器预览只修改 working `RoadEditingState`/预览表现，不触发
完整 LFCA 或共享静态路网构建。

用户确认建造后，compiler 可以在后台从候选道路编辑状态完整编译 LFCA，并构建一个新的
`SharedNetworkRevision`。v1 允许全量重建；不预设数百毫秒输入事件队列、FIFO、
latest-wins scheduler、局部/增量构建或当前数组原地 mutation。

候选准备期间当前 committed 修订继续运行。候选成功后由 #302 在固定步进安全边界把
新共享修订、迁移后的每世界动态状态、对应 `CommittedNetworkSource` 与 editable 来源必需的
optional `EditableDiffBase` binding 作为同一个原子提交；玩家确认建造时该 source 是 committed
`RoadEditingState`，发布修订更新时可以是 `PublishedLfcaReference`。失败时全部保持旧值。

### 8. 存档只保存活动修订的已提交来源

共享静态路网数组不进入城市存档。存档只读取已经进入 Runtime 活动 aggregate 的
committed network source，并由上层 Save Manifest 绑定同一时点的 Runtime snapshot。
可编辑世界保存 committed `RoadEditingState`；只从发布 LFCA 启动的 runtime-only 世界保存
宿主作用域的不透明持久 asset key，并以 LFCA digest/length/`NetworkRevisionId` 形成
`PublishedLfcaReference`。key 不充当信任锚、存档不复制 LFCA；加载时重新认证现有资产、
读取并构建。

发布资产若要启用道路编辑，宿主必须同时提供对应的 committed `RoadEditingState`，重新编译
后以 `NetworkRevisionId`/contract closure 证明语义相符，再由 #302 用该 exact LFCA 构建
新根并执行同修订来源 rebase：root/source/diff-base binding 原子替换、动态状态保持不变。
成功前不得启用编辑，也不得从 LFCA 逆向猜测 authoring-only 曲线。editable 活动 aggregate
在 editor/#302 侧保留与当前根 origin 精确一致的 LFCA 作为 `EditableDiffBase`；下一候选用它
发射 LFSD，提交后 target LFCA 原子成为新 base，失败继续保留旧 base。存档不保存该 base；
加载 RoadEditingState 时先重编译并建立新 session 的根/base，再恢复同 revision snapshot。
候选或 working draft 尚未进入 Runtime 时，保存继续捕获旧 committed 来源；退出不会恢复
未提交候选。

#302 冻结 Runtime snapshot/cutover 的 exact binding。旧的
`originStaticImageDigest/originStaticImageByteLength` 不再存在；快照与切换使用
`NetworkRevisionId`、LFCA origin digest/length、静态契约版本和稳定身份索引，不持久化
内部 dense ordinal、地址、layout 或 partition plan。

LFCA origin digest/length 是来源审计与同字节快速路径，不单独定义静态语义兼容；从
committed 道路状态重编译后，只要 `NetworkRevisionId` 与 #302 冻结的契约版本/身份
闭合相容，compiler provenance 导致的 LFCA exact-bytes 变化不应被误判为跨修订迁移。

### 9. 性能证据不建设第二套证明平台

共享静态路网的资源证据至少记录：

- LFCA/LFSM/LFSD candidate exact bytes、各 component logical/retained bytes；
- count/build/closure 墙钟与累计分配；
- builder 成功后不借用三对象，runtime-only LFCA 可释放；
- 发布构建的 LFCA + candidate arrays + scratch 峰值，以及可编辑构建的 current root +
  retained base LFCA + target LFCA/LFSM/LFSD + candidate root + scratch 峰值；
- 最小 headless、facility-only、profile/frame-only、full lane Spatial 和当前最大合法产品场景；
- 2/8/32 worlds 的 shared-static 增量；
- identity 双向 lookup 与 Runtime-facing 连续 range traversal 的描述性成本。

这些结果进入可复现的测量证据，不新增 benchmark crate、JSON/Schema 协议或常驻
性能服务。没有最低产品硬件与完整 Runtime kernel 时不虚构绝对毫秒 SLA；
布局选择必须由 production tick/pose 访问路径证明，没有该证据不能声称城市级
Runtime 性能通过。

## 后果

正面后果：

- 用户加载、编辑、运行、保存只维护 committed 道路状态或已认证 LFCA asset reference、
  LFCA 和内存 Runtime 数据的清晰职责，不再维护第二种持久化性能制品；
- Runtime 仍获得连续、密集、可共享的性能布局；
- 字段分组和 SIMD 可以随真实 kernel 优化，不受已发布 ABI 锁定；
- headless 不构建 Spatial，Traffic/Spatial 又不能错配修订；
- 玩家本地编辑不执行内容存储或磁盘发布事务。
- loader 不重复 compiler 的 Identity/几何派生工作，确认建造后的候选延迟保持线性、
  数据搬运和 Runtime 结构闭合优先。

成本与风险：

- 每次进程加载或确认建造都要执行一次 LFCA → shared static network 转换；
- 构建期会短暂同时持有 LFCA 与最终 typed arrays；道路切换还会同时持有当前和候选修订；
- 不提供跨进程 mmap 页共享；
- 具体最佳 SoA/AoSoA 分组仍须以真实 Runtime kernel 访问模式验证；
- Runtime loader 信任官方受检 LFCA 中已经由 compiler 派生的 StableId 与 segment 数值；
  compiler known vectors、后发射检查和端到端测试若缺失，会形成产品正确性回归风险；
- 如果未来最低产品硬件证明重建成本不可接受，需要另行设计非权威持久化缓存，
  不能把本实现的 Rust 内存布局直接写盘。

## 被拒绝的替代方案

### 继续交付 target/profile-specific 静态镜像 ABI

当前证据没有证明磁盘读取或 LFCA 转换成本足以抵消第二套格式、平台矩阵、完整性协议和
迁移负担；同时会在 Runtime kernel 出现前冻结布局，拒绝。

### Runtime 直接在 LFCA RowV1/FieldV1 上执行

LFCA 是规范发布格式，顺序 cursor 适合检查和构建；保留的 ordinal 随机访问是 O(n)，
变长字段和通用 tag/value header 不适合 fixed-tick 热路径，拒绝。

### 玩家编辑从 Validated LIR 直接构建、发布内容从 LFCA 构建

这会形成两个 Runtime backend，并要求长期证明完全等价。v1 接受几十毫秒级已有中转
证据并保持单一路径；真实城市测量成为阻断时再重新决策。

### 为降低峰值内存原地修改当前静态数组

这会破坏并行读取、失败回滚、共享和固定步进内单修订原则，拒绝。

### 把内部 Rust 结构直接序列化为缓存

它会重新引入未版本化 ABI、padding/target/依赖漂移和不可信读取问题；缓存必须另行设计，
拒绝当前实现顺带写盘。
