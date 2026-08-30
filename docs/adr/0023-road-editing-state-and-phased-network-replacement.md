# ADR 0023：道路编辑状态权威与分阶段路网替换

**状态**: Accepted（#296 FlatBuffers G1 Pass）<br>
**日期**: 2026-08-10<br>
**适用范围**: 可视化道路编辑、程序化道路生成、道路编辑状态、编译器来源、道路建成后的
再次修改、不可变路网修订与来源持久化边界<br>
**扩展**: ADR 0020、ADR 0021；重新打开 ADR 0020 中“Geometry 文档是 production 主要
编制语言、编辑器默认持久化 Geometry 文档”的具体格式选择，不改变权威来源模块图、
编译器中间表示或不可变路网修订决策<br>

> **后继决策（2026-08-18）**：ADR 0025（Accepted；#300 G1 Pass）保留本文
> 道路编辑状态权威和 A → C 候选替换，但把目标静态镜像改为进程内
> `SharedNetworkRevision`。working/candidate 状态不进入存档；可编辑世界只保存已成功进入
> Runtime 的 committed `RoadEditingState`。没有对应道路编辑状态的发布 LFCA 世界只能作为
> runtime-only 来源，以已认证 asset reference 重载；其晋升与 Runtime 修订切换由 #302 原子
> 提交。
>
> **后继决策**：ADR 0029 取代本文 §2.1 的静态路线来源形状；当前生产
> `format_version = 3` 根表保存 23 个可构造 Identity 声明向量及 owner-local
> `conflict_zone_regions`，无 `static_routes` 字段，field id 连续至 28。
> 道路编辑状态权威、A → C 候选替换与 FlatBuffers 编码选择继续有效；本文旧来源
> 形状只作历史背景。

**关联文档**:

- `0020-compiler-owned-static-network-and-static-image.md`
- `0021-city-simulation-game-traffic-foundation.md`
- `0022-authoring-curve-and-canonical-polyline-error-budgets.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `0029-retire-precompiled-static-route.md`
- `../design/shared-static-network.md`
- `../design/road-editing-source-and-geometry-frontend.md`
- `../design/network-compiler.md`

## 背景

#296 原 G1 把严格 UTF-8 JSON Geometry 文档冻结为生产道路编制语言。后续实现和审阅
为 JSON 数字词法、Schema 数学整数语义、来源行列与精确资源计量付出了显著复杂度。
产品复核进一步确认，选择 JSON 时依赖的两个前提并不存在：

- 玩家、设计师和第三方不会手工编辑道路来源文本；
- 原始文本 Git diff 不是道路审阅或协作界面，产品需要的是画布选择与语义差异。

LaneFlow 的实际生产入口首先是可视化编辑器，同时需要游戏初始化时的程序化道路生成。
道路建成通车后仍必须能够修改，但这不自动要求首版就在已建道路上暴露原始曲线控制柄。
运行时只应消费规范折线和不可变路网修订；编辑器又必须保存足以重建当前道路的编制
信息，否则后续只能从运行时折线猜测用户意图，也无法平滑演进到候选调整与影响预览。

因此需要先冻结产品权威和分阶段行为，再选择来源持久化编码。已经投入的 JSON 实现成本
不参与该长期决策，只影响清理和交付排期。

## 决策

### 1. 道路编辑状态是生产编制权威

道路编辑状态（Road Editing State）是城市项目或存档中可继续编辑、可重放的道路
编制事实。它至少保存：

- 当前道路走向定义及其稳定编制键；
- 道路宽度、横断面、车道组织、路口连接和已支持静态规则；
- 模块、导入、来源沿袭和编辑器画布选择所需的稳定关系；
- 由程序化生成器产生并经玩家接受后的实际道路结果。

程序化生成器的构建标识、参数、seed 和输入摘要属于来源沿袭；它们不能单独替代已经
落地且可能被玩家继续修改的实际道路编辑状态。

道路编辑状态不保存：

- 全部鼠标点击、拖拽轨迹或无限撤销历史；
- 共享静态路网内存布局、运行时 ordinal、车辆或其他每世界可变状态；
- Adapter 路面网格、三角形、材质、碰撞体或细节层次；
- 只为编译中间步骤生成、可以重算的 HIR/MIR/LIR 临时值。

### 2. 有类型道路编辑模型先于物理编码

可视化编辑器、游戏内编辑器、程序化生成器、官方 SDK 和 importer 共享同一有类型道路
编辑模型。该模型定义字段、单位、稳定身份、引用和不变量；编译器接收经受检构造的具体
来源模块，并沿共同 Typed AST → HIR → MIR → validated canonical LIR 管线降阶。

磁盘或跨进程编码是道路编辑状态的版本化物理表示，不拥有第二套道路语义。#296 FlatBuffers G1
评估了 size-prefixed FlatBuffers、有界记录流 + Protobuf 和严格 JSON 三个产品候选；
产品负责人于 2026-08-10 选择候选 A：按模块保存的 size-prefixed FlatBuffers。城市项目
由多个模块 blob 和显式导入图组成、Rust 内存布局与 portable canonical artifact 不作为
生产来源格式，这些边界不因物理编码选择而改变。

### 2.1 已选择的 production 编码：按模块 FlatBuffer

根表保存精确格式版本、唯一模块头、一组不分配 `StableId128` 的道路走向定义，以及按
Identity v1 稳定声明分组的有类型向量；owner-local 值留在 owner table。当前生产
`format_version = 3` 为 23 个可构造 Identity 向量及 owner-local
`conflict_zone_regions`，无 `static_routes` 字段。额外道路走向向量保存当前曲线编制
事实，但不创造静态路网身份。编译器先检查 size
prefix、完整长度和 `LFRE`，再使用
有界 verifier 验证 offset/table/vector/string/union，之后直接从借用 view 预检并降阶。
正常编译路径不形成整模块 wire decode 对象图，也不需要为逐记录释放自建 framing。

模块 blob 是加载和原子保存边界；稳定 authoring key、typed property path 和规范值为
后继来源差异与 LIR 影响差异提供输入。FlatBuffers buffer 只读；编辑器在 LaneFlow 有类型道路编辑模型
上修改，保存时重建当前有界模块，不使用实验性 reflection 原地扩缩 string/vector。
来源字节摘要只绑定精确重放内容；实体身份、语义等价和路网修订仍从有类型模型与
canonical LIR 派生，LaneFlow 不用 bytes equality 替代语义比较。

### 2.2 Rust 生成代码 `unsafe` 边界

官方 `flatc` 的 Rust 生成物包含实现 offset 跟随与标量访问的 `unsafe`。这不是直接拒绝
FlatBuffers 的理由：workspace `unsafe_code = forbid` 的产品意图是禁止 LaneFlow 手写
代码随意引入未证明的不安全操作，而不是假装第三方 runtime 或受控生成物没有
`unsafe`。

生成绑定因此放入私有、`publish = false` package；道路编辑生产调用图中的手写 crate
继续继承 `unsafe_code = forbid`，编译器只调用 verifier 驱动的安全 root/accessor，不能调用
`_unchecked`。固定版本 `flatc`、checked-in 生成物、clean regeneration diff、生成路径
外的 `unsafe`/lint-exception 扫描、fuzz 和依赖升级复审共同构成审计边界。ADR 0024
另行冻结的 `laneflow-format` 单一只读 mmap 小岛不进入此 wire 调用图。这个 package
不是新产品层、公共库或前端插件接口。

#296 同时交付第一方 Rust 的字段私有、有类型构造与写入能力，满足游戏初始化时的
程序化道路生成；它不向调用方公开 generated wire table，也不建立第二条编译入口。
完整 C++/C# SDK、引擎事务封装和编辑器 UI 属于后继交付，但继续消费同一 `.fbs` 与字段
语义，不能另建来源格式。

道路编辑来源 v1 的 `AccessRule` 只暴露 LaneEdge、LaneGroup、RoadSection 与
ManeuverPath target；不暴露当前 runtime 尚不可执行的 FacilityBand target。对应枚举
数值在 v1 保留为无效，后继若形成产品能力必须提升来源格式版本，而不是让 writer/UI
先写入一个必然 capability-unavailable 的规则。

### 3. JSON 的未选状态与兼容边界

旧 Geometry JSON 尚未发布为稳定产品契约，也未被选择，因此不获得读取兼容、双写、
自动迁移或隐藏 fallback 承诺。旧实现和校准证据保留在 Git/GitHub 历史中，用于说明
取舍和回归算法；production 只允许已选择的 FlatBuffers 来源编码与一条官方读取路径。

清理只移除 JSON 词法、对象 shape 和文本行列等格式复杂度。曲线求值、
stationing、offset、规范 `f32` 量化、确定性位置/方向目标、身份和领域语义验证仍然必要，
不能把它们误计为 JSON 债务。

ADR 0022 的 B1 决定只改变首轮几何质量承诺：FlatBuffers v1 与第一方 writer/reader 可以
用于内部完整验证，但在后续产品复核前不进入公开 schema publication，也不承诺长期城市
存档兼容。B1 不是隐藏 JSON fallback，未来若需要连续硬误差保证，必须以新的来源/几何
语义版本重新进入 G1；在 B1 尚未提升为正式存档语义前不承担迁移兼容。

### 4. 道路修改按 A → C 分阶段交付

近期 A 阶段采用建造、拆除和整体替换模型：

1. 用户或生成器在道路编辑状态中形成候选道路；
2. 编译器完整验证候选并产生可供 #298 比较的规范 LIR 与来源映射输入；
3. 失败时旧道路和旧路网修订保持不变；
4. 成功后由 #302 冻结的安全边界把新不可变路网修订原子切入；
5. 首版允许由宿主显式进入维护暂停模式，具体车辆、分区、建筑和其他城市对象迁移策略
   由对应后继 G1 冻结。

A 阶段不承诺在已建道路上直接显示并拖拽原始曲线控制柄。用户仍然可以修改道路，只是
通过整体替换或拆除重建完成。

长期 C 阶段在相同内部事务上增加：

- 从当前道路走向定义建立候选调整；
- 在提交前显示道路、路口、分区、建筑和交通状态的影响预览；
- 对未改变语义的实体保持稳定身份，对真实删除/新建明确更换身份；
- 验证成功后原子替换，失败或取消时不影响当前运行修订；
- 在产品证据需要时增加局部/增量编译和更丰富的运行时迁移。

### 5. 保存当前道路走向，不保存交互历史

道路走向定义（Road Alignment Definition）是重建当前已建道路所需的几何编制描述。
v1 只接受直线与三次 Bé塞尔曲线。圆弧、螺旋线或其他 importer/generator 曲线必须在
写入 v1 前转换为这两种 segment。第一方 converter 对自身原始 primitive 与输出 cubic
使用固定网格观测，观测最大值超过所选 2/5/10 cm B1 目标即拒绝转换，并把分布/最坏参数
作为 caller-owned conversion report 返回；该 report 不写入 `.lfre`、不进入摘要，也不由
#296 compiler 复验，第三方 primitive evaluator 不成为 LaneFlow source 语义。该观测不
构成原始 primitive 到 cubic 的连续硬证明。v1 不保存原始 primitive 语义。若后续已发布
版本增加 curve union，必须提升来源格式版本；是否迁移由该次产品/G1 决定，未发布 B1
fixture 只 clean-regenerate。保存走向定义不
等于运行时保留曲线求值器，也不等于 UI 必须暴露全部控制参数。

常见道路 taper 由 lane/facility 在一个 corridor station 区间内的线性起止宽度表达；
中心线横向位置继续由横断面顺序和宽度前缀唯一派生，不增加自由 offset 曲线或第二份
几何权威。

编译器继续把走向定义离散为规范 `f32` 折线。LIR、共享静态路网、Traffic Runtime 和
Spatial 不保存或重建编制控制点；Adapter 的网格细分和车辆物理也不能反向成为道路
长度、station 或拓扑权威。

### 6. 来源审阅使用画布与语义差异

道路审阅、冲突定位和协作合并以稳定实体、属性路径、画布选区和语义差异为主。物理
编码可以提供字节范围用于损坏诊断，但不要求以人工行列、文本 patch 或原始文件 diff
作为产品界面。

区域/模块级协作和实体级并发编辑都属于来源编码与编辑器契约。v1 以模块 blob 为加载和
原子保存边界，以一稳定声明一有类型条目、稳定 identity 与 typed property path 为后继
来源差异提供输入；它不把二进制 bytes diff 当作协作协议。#296 A 阶段只提供该定位和
#298 的规范 LIR 差异输入，不实现 authoring-only `RoadEditingSourceDiff`。C 阶段由后继
Delivery Issue [#345] 交付来源差异/冲突合并，并与 #298 的路网影响差异明确分层。
宿主负责跨模块保存事务，LaneFlow 不拥有整个城市存档容器。

## 来源编码选择记录

候选按以下产品需求评估：

1. 官方编辑器、游戏初始化生成器、SDK 和 importer 能否可靠读写；
2. 是否以原生整数、枚举、向量和有界集合表达道路字段，避免文本数字词法成为语义层；
3. 是否支持区域/模块级局部加载以及稳定实体级差异/合并；
4. 是否具有明确的 schema 演进、未知字段、版本拒绝与一次性迁移政策；
5. 是否能从不可信或损坏存档失败关闭，并在分配前执行长度和基数预检；
6. 是否能稳定关联实体/属性/画布选择，形成来源诊断和源映射；
7. 是否适合 Rust 编译器和预期宿主语言，且依赖、许可证、最低 Rust 版本与维护成本可控；
8. 是否在代表性城市项目上满足保存、加载、编译、局部编辑和协作的资源预算。

人类可直接阅读、手工修改和原始文本 Git diff 不再是阻断性门槛。

产品负责人根据上述产品需求，于 2026-08-10 选择 `LF-ROAD-EDITING-SOURCE-v1` 使用按
模块 size-prefixed FlatBuffers。具体 bytes、schema 闭合规则、公共入口、资源顺序、
候选矩阵和 workload 以 `../design/road-editing-source-and-geometry-frontend.md` 第 9 至
10 节为 G1 输入：

- FlatBuffers 提供 C++、C#、Rust 等工具链共用的有类型 schema、原生标量和 verifier；
- size prefix、file identifier 和自定义 `VerifierOptions` 让输入在语义 lowering 前失败
  关闭，借用 view 避免 Protobuf 整模块/逐记录 decode 对象与分配；
- 官方 Rust 生成物中的 `unsafe` 进入私有生成绑定边界，道路编辑 production 调用图中的
  手写 crate 不放宽 `unsafe_code = forbid`，只允许受检安全入口；
- FlatBuffers 的只读 buffer 不承担编辑模型职责；模块保存仍是有类型模型重建 blob；
- 有界记录流 + Protobuf 的主要优势是 generated Rust 无 `unsafe`，主要代价是自定义
  framing、逐记录对象分配与 wire shape 规避；
- 严格 JSON 的主要优势是人工可读和文本工具生态，但这些收益当前不在主要产品路径，
  数字词法、shape、行列和分配账本成本仍然存在；
- Cap'n Proto、rkyv/postcard、CBOR/MessagePack 与全自定义格式没有进入最终三选一。

该选择明确接受 A 的两项代价：保存时重建当前有界模块，以及长期维护受控的官方 Rust
generated `unsafe` 审计边界；不接受 B 的自定义 framing/decode allocation/schema 规避，
也不接受 C 的文本数字和 parser 复杂度。选择依据是整体产品/架构适配，不宣称已经通过
FB/PB 同 workload 的定量性能竞赛；G2 仍必须证明 A 满足 LaneFlow 的绝对资源预算。

## 后果

正向后果：

- 产品入口、道路可再次修改与不可变运行时修订不再互相矛盾；
- A 阶段可以较低产品复杂度交付，并为 C 阶段复用同一候选编译/原子替换事务；
- 编辑器和生成器不会形成两套道路语义；
- 来源格式可以针对结构化编辑、局部加载、协作与受控资源选择，而不让既有实现成本
  代替产品收益；
- 运行时继续只持有规范折线和静态表，不承担编辑器状态成本。

额外收益是 FlatBuffers verifier 和只读 view 使损坏诊断、逐实体
lowering 和编译器资源计量不需要第二套 offset parser 或整模块 wire decode 对象。

成本与风险：

- #296 原 G1/G2、PR #332 的 JSON parser/Schema/fixture/校准实现不能直接进入 G3，须按
  已选择的 FlatBuffers 边界清理或迁移格式无关能力；
- A 需要维护 `.fbs`、固定 `flatc`、私有生成 package、模块重建与 generated `unsafe`
  审计；
- 当前道路走向定义会增加城市项目/存档的静态编辑数据，但不增加稳态运行时镜像热数据；
- C 阶段的影响预览、身份保持和城市对象迁移仍需要编辑器、城市游戏层、#298 与 #302
  等后继能力，不能由 #296 单独宣称完成；
- 若未来确需对外开放第三方可手工编制格式，应以独立产品需求和新 Gate 建立，不恢复旧
  JSON 兼容包袱。

## 已排除边界与未选编码

### 因旧实现已完成而默认继续 JSON

沉没成本只能影响排期，不能替代产品要求。JSON 的人工可读与文本 diff 价值在目标用户
路径中不存在，因此拒绝“默认沿用”的推理；候选 C 未被选择。

### A 阶段只保存运行时折线

这能减少短期来源数据，却会丢失当前道路的编制意图，使 C 阶段只能重新拟合曲线并产生
身份、station 和几何漂移。A 必须保存当前走向定义，但不保存完整交互历史。

### 直接实现完整 C 阶段

它会把 #296 同时扩张为编辑器 UI、城市对象影响分析、#298 语义差异、#302 路网修订
迁移和游戏规则项目，无法形成可独立验收切片。采用 A → C。

### 把 portable canonical artifact 当作可编辑来源

规范制品面向发布、独立验证和运行时重建，已经丢失或规范化部分编制意图。它不能替代
道路编辑状态，也不能获得反向 authoring authority。

### 因 Rust 生成代码包含 `unsafe` 而直接拒绝 FlatBuffers

这混淆了 LaneFlow 手写代码政策与固定第三方生成物的事实。生成的 offset/accessor 实现
确实需要严格审计，但可以用私有 generated package、道路编辑手写 crate `forbid`、安全
verifier 入口、固定生成器、再现检查和 fuzz 形成窄边界；因此不能仅凭出现 `unsafe` 关键字淘汰
更符合产品矩阵的编码。

### 在 FlatBuffer blob 上直接原地完成道路编辑

基础 FlatBuffers view 是只读的，变长 string/vector 的反射式 resize 仍是实验能力。把
物理 buffer 当作编辑模型会让产品语义、校验和版本迁移泄漏到 offset 操作。v1 明确在
LaneFlow 有类型模型上编辑，并在模块保存时重建 buffer。

### 使用单一 Protobuf 根对象

它的 mutable object API 最直接，但会先构造整个模块的生成对象图，难以在现有编译器
资源硬上限内逐项失败关闭。LaneFlow 已有独立有类型编辑模型，不需要为了编辑便利再把
generated Protobuf 对象树作为第二模型。

### 未选候选 B：使用有界记录流加 Protobuf payload

它能逐记录预检和释放，但必须长期维护自定义 framing、每记录 decode 分配以及为降低
对象放大而引入的 packed key/parallel array 约束。产品负责人选择以 FlatBuffers
verifier + 借用 view 和更少自定义协议为优先，接受受控 generated `unsafe`，因此不选择 B。

## 治理与实施

1. #296 FlatBuffers G1/G2 Pass 已记录；旧 Geometry JSON G1/G2 与校准证据只作历史。
2. 产品负责人已选择 FlatBuffers 来源编码候选；本 ADR 与
   `road-editing-source-and-geometry-frontend.md` 必须精确
   回写被选编码、未选候选、兼容/删除清单、schema、依赖和 workload。
3. G2 实现拆为可独立审阅的 PR 系列；任何偏离已接受产品/格式/资源边界的变更返回 G1。
4. 实现、资源校准和外部审阅均绑定各自 exact head；最终集成切片通过前不得进入 G3。
5. #298 继续只消费 validated canonical LIR；#302 继续拥有运行时修订切换；编辑器 UI、
   城市对象影响预览和迁移策略若超出既有 Issue，必须拆出独立 Delivery Issue。
6. `RoadEditingSourceDiff` 与 authoring-only 冲突合并已登记为独立后继 Delivery Issue
   [#345]；未实现时只能陈述稳定来源定位能力，不能宣称 C 阶段已覆盖来源差异。

[#345]: https://github.com/illusion-tech/laneflow/issues/345
