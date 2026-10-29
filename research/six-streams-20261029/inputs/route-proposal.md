# #806 路线可执行性设计候选与最小依赖

状态：本地设计草案，不是 G1 Accepted，不授权生产实现，不关闭 #806。本文只给反例、依赖判断、最小切片和未执行的验证方案。

审阅修订：Codex 于 2026-10-26 收紧 #806-A 的观察范围和完成标准，并与几何侧对齐最终拓扑/修订绑定。原稿保存在 [审阅前副本](../review-originals/route-proposal.md)；统一接口与后续边界见 [协同说明](../coordination.md)。本次只修订文档，没有执行下列验证。

- 冻结源码：`78f0d66c53431175de57d13e626d6a88231246c3`。
- 主 checkout 开工状态：`research/1071-sccache-evaluation@60f5c4705c059f45e700e5024b0aba0f257c4739`，`git status --porcelain` 为空；未切换、重置、清理或更新它。
- 独立工作树：`E:/projects/worktrees/790-806-route-analysis/laneflow`，detached HEAD 固定为上述冻结提交。
- Issue #806、#237、#273 于 2026-10-26 只读核实。候选报告仅作为待复核输入，不作为已接受设计。
- 文中“事实”指冻结源码、合同、已有证据或 Issue 的直接内容；“推导”指未执行的逻辑分析；“建议”与“未验证”均不代表实现或运行结论。

## 1. 结论与推荐顺序

1. **事实 + 推导：仅放宽车道筛选不能产生可执行路线。** 当前展开器要求下一跳的 `from_lane` 等于已经到达的 lane；仅放宽搜索会让搜索与物化不一致，连同物化一起放宽则会拼出没有对应连接的边链。替换或追加 lane 也分别破坏机动路径匹配或缺少真实横向转移。[C1] [R1]
2. **建议：默认保留 LuST 的来源道路/路口/连接映射和原车道连续流，以 #237 的最小换道子合同承接固定、预制过渡。** 必需的是相邻关系、许可、源/目标物理占用、目标前后车安全、进入仲裁、原子提交、车尾清空与重放，不是潮汐车道、自由在线选道或全量 `ResolvedLanePlan`。[X1] [F1] [I2]
3. **建议：先做 #806-A 真实反例和分阶段诊断，再做一个双车道路段的可执行闭环。** 前者现在可独立开展；后者等待所选执行能力及同一最终拓扑。诊断完成或小夹具通过都不是 #806 的全网交付。
4. **条件备选：固定区域完整重编为合流/分流连接单元。** 通用 `Junction`/`Conflict` 提供了可复用机制，但全部直行与斜向流都必须进入门、冲突和下游清空合同；还需独立接受合流/分流的间隙、优先级、车道可用性及派生负载语义。它不是已接受或已证明安全的方案。[J1] [J3] [K2]

## 2. 完整逻辑反例

### 2.1 输入确实包含 B0 和 B1

下面是测试应构造的逻辑输入，不是仓库中已经存在或本轮已经运行的夹具。`A0`、`B0` 等是本文的逻辑简称；实施时使用明确的来源声明，不把它们当作最终稳定身份的派生规则。

| 对象 | 必须真实声明的内容 |
| --- | --- |
| 外部车道 | `A0` 一条；`B0`、`B1` 两条同向车道；`C0` 一条。DUE 只写道路序列 `A B C`，不写 internal edge，车辆类型和出发窗口合法。 |
| B 的几何 | `B0`: `(6816.88,5727.52) → (6836.88,5727.52)`，长度 20 m；`B1`: `(6816.88,5730.52) → (6836.88,5730.52)`，长度 20 m。两者是不同来源 lane，不是同一个 ID 的别名。 |
| A→B | 只有 `P1 = [A0, I1, B0]`。`A0` 长 20 m，`I1` 长 10 m，三者沿同一方向首尾相接。 |
| B→C | 只有 `P2 = [B1, I2, C0]`。`I2`: `(6836.88,5730.52) → (6840.88,5727.52)`，长度 5 m；`C0` 从该终点继续直行 20 m。 |
| 其他声明 | `I1`、`I2` 为真实 internal lane；路口归属、internal 续接/终端 connection、节点引用完整。`P2` 可绑定一个信号门，`tl`、`linkIndex`、相位状态位置、门和停止线相互一致。不得借未知 lane、缺失引用或非法 XML 制造失败。 |

冻结的归一化实现会从来源 connection 解析入口、完整内部链和出口，并产生 `ManeuverPath`；因此测试能且应该先直接断言 `B0`、`B1` 与上述两条路径确实存在。[C4] `P2` 的门必须位于 `path.edges()[transition_index]` 对应的边上，不能只改路线数组而保留旧绑定。[C6] [J2]

当前 `can_complete` 只沿 `hop.from_lane == lane_index` 继续；`materialize_path` 也只选择 `hop.from_lane == current_lane`，再追加该路径的全部内部边和出口边。[C1] 因而在本反例中：

1. 从 `A0` 只能经 `P1` 到 `B0`；
2. 在道路 `B` 上，当前搜索状态仍是 lane 0；
3. `B→C` 的唯一路径要求从 lane 1 出发，搜索不能完成；
4. 这不是 `B1` 不存在，也不是道路 `B→C` 不可达，而是缺少从 `B0` 到 `B1` 的已证明可执行横向转移。

### 2.2 四种“修补”分别缺什么

| 做法 | 逐步推导（均未执行） | 仍缺的能力/准确边界 |
| --- | --- | --- |
| 只放宽 `can_complete` | 搜索可把 `P2` 当作后继并返回成功；但 `materialize_path` 仍拒绝在 `B0` 选择从 `B1` 出发的路径，最终不能交付路线。 | 搜索与物化合同不一致；没有边链输出，更没有换道执行。 |
| 同时放宽两处筛选 | 可拼出 `[A0, I1, B0, I2, C0]`；但 `B0→I2` 不是 `P1` 的出口后续，也不是已声明的普通 successor 或机动转移。按当前注册顺序，预期先在边对检查得到 `Disconnected`。 | 缺少 `B0→I2` 的真实拓扑和完整机动路径；该错误类别是推导，不是实测。 |
| 把 `P1` 出口的 `B0` 替换成 `B1` | `[A0, I1, B1, …]` 在当前图中通常先因 `I1→B1` 无连接而失败。即使另行补连接越过首层检查，`P1=[A0,I1,B0]` 的完整前缀仍不匹配，进入 `ManeuverMismatch`。 | 不能把一个物理出口改名为另一个 lane；两种失败阶段不能混写成一个必然错误。[R1] |
| 在 `B0` 后追加 `B1` | `[A0,I1,B0,B1,I2,C0]` 在没有 `B0→B1` 连接时得到 `Disconnected`。补上普通 successor 后，两个机动路径可以分别完整匹配，因而注册可能成功。 | 若 `B0/B1` 是完整平行路段，这只是从 `B0` 末端跳到 `B1` 起点，不是路段中间换道；即使把几何画成连续，也仍缺多车安全证明。 |
| 插入一条普通斜向边 | 必须先把 `B0/B1` 在真实过渡边界拆成上游、过渡区和下游，例如 `[B0_up,T01,B1_down]`，并补齐普通连接和最终长度；不能复用两条完整平行边再斜接。 | 连续中心线和可注册边链仍不覆盖斜线与直行流的交叉占用、目标前后车、源车尾及同拍竞争。注册成功不能补足这些条件。[O1] [F1] |

### 2.3 四个结论层级不能混用

| 层级 | 本反例的结论 |
| --- | --- |
| 道路可达 | `A→B`、`B→C` 的来源 connection 均存在，按道路对投影可达。它可由诊断单独计算，不等于 lane 级可达。 |
| 路径物化 | 当前严格搜索失败；放宽筛选只能得到上表所述不完整或未证明的边链，不能命名为 executable route。 |
| 运行时可注册 | Runtime 先检查相邻边是普通 successor 或机动转移候选，再要求完整 `ManeuverPath` 匹配和内部边覆盖。[R1] 某些补连接后的伪换道可能通过，但这只证明静态序列合同。 |
| 多车安全执行 | 路线设计明确不把几何碰撞、跟车安全距离或当前通行权作为注册证明；普通跨分支运动也没有额外的世界几何碰撞检查。[D1] [F1] 必须另行验证动态占用、间隙、仲裁、清空和重放。 |

因此，**即使某个替代边链成功注册，也不能据此宣称安全换道**。注册只编译本世界的路线及出现项；实际遍历仍由该有序边序列和车辆游标决定。[D1] [D2]

## 3. 既有测试缺口与准确测试设计

### 3.1 已核实的缺口

`diagnostic_mode_skips_route_expansion_but_keeps_population` 的注释声称“`x→y` 仅 `0→0`，`y→z` 仅 `1→1`”，但 XML 中 `y` 只有 `y_0`，实际下一跳也是 `fromLane="0" toLane="0"`；来源中反而存在 `x_0→y_0→z_0` 的连续候选。所以注释中的跨 lane 反例并未被构造出来。对 fail-fast 结果，测试只调用 `is_err()`；它不能据此把错误归因于路线展开。其余断言确实检查诊断模式保留一条人口且不产 `routes.toml`，但不能弥补反例失真。[C2]

**未验证：本轮没有运行该测试，不断言它实际返回什么错误，也不猜测无关几何、信号或编译阶段是否出错。** 上述结论仅来自 XML 与断言的静态核对。

### 3.2 #806-A 应构造的反例与连续对照

建议由同一个显式夹具构造器生成两种输入，避免靠注释或偶然字符串替换表达语义：

- **缺口输入**：严格使用 §2 的 `P1=[A0,I1,B0]` 和 `P2=[B1,I2,C0]`，不再添加 `B0→C`。
- **连续合法对照**：增加真实内部边 `I0: (6836.88,5727.52) → (6840.88,5727.52)`，长度 4 m，以及完整路径 `P0=[B0,I0,C0]`；同步更新路口 internal 成员、终端 connection 和所需控制声明。此时存在不需要换道的 `[A0,I1,B0,I0,C0]`，应精确物化为该序列。`P2` 仍可保留，以覆盖同一路口内的多 lane 路径。

建议按以下顺序断言，任何前置失败都不能被改写成“换道缺口”：

1. **夹具自证**：解析和 `normalize_junctions` 成功；`B` 的外部 lane 集合恰好包含 0、1；两条关键路径的完整 entry/internal/exit 与 `path_by_connection` 精确相等；DUE 不含 unknown/internal road edge。
2. **直接路线阶段**：对缺口输入直接调用路线构建函数，匹配 `Error::SumoModel`，并精确匹配当前源码给出的 `no complete lane-level path for road route ["A", "B", "C"]`，或在实现结构化诊断后匹配稳定的 `lane-continuity-gap` 类别。不得再用任意 `Err` 或宽泛子串代替阶段断言。[C1]
3. **组合入口阶段**：fail-fast 转换必须返回与上述直接路线构建相同的具体路线错误；诊断模式则保留同一入选记录、`routes_toml == None`，且不因跳过展开而跳过 unknown/internal road-edge 校验。[C3]
4. **对照成功**：连续输入得到精确的完整边链，路线目录和人口绑定保持稳定；不能因存在 `B1` 就隐式改选它，也不能删除失败记录或从候选尾部补位。
5. **负例分域**：unknown lane、unknown road、DUE 直接引用 internal edge、缺失道路 hop 分别留在来源/道路可达错误域，不得全部标成可通过换道修复。

以下是端到端观察框架，不代表 #806-A 单个切片必须实现全部阶段。#806-A 直接负责前四行；静态编译只记录实际到达的结果，未到达时标为“未执行”。Runtime 注册、准入和固定步进由后续独立验证负责；不适用须有明确理由，不能用来替代缺失的执行证据。

| 阶段 | 应报告的事实 | 不得推断的结论 |
| --- | --- | --- |
| 来源准入 | XML、lane/connection 引用、人口记录是否合法 | 合法来源不证明车道级路线存在。 |
| 道路 hop 可达 | 固定 DUE 道路序列中每个相邻道路对是否有来源路径 | 不把“同 road 任意 lane”当作连接。 |
| 当前 lane 连续搜索 | 稳定定位首个不可完成 occurrence，列出已到达 lane 集合与下一跳所需 lane 对 | `lane-continuity-gap` 只表示需要进一步调查横向转移，不证明许可、距离或安全可行。 |
| 完整路径物化 | 最终入口、内部边、出口、重复 occurrence 和稳定选择结果是否完整 | 不把道路级投影或部分边链写入正式路线目录。 |
| 静态构建与修订 | 最终几何、控制绑定、LFCA/共享修订是否成功 | 不用旧修订或旧长度验证新路线。 |
| Runtime 注册与车辆准入 | 分别记录 `RouteError`、注册表原子性、profile/cursor/生成准入结果 | 注册成功不等于生成或执行成功。 |
| 固定步进执行 | 记录完成、正常等待、拒绝进入和首个安全失败 | 正常等待不是非法路线；小夹具完成不等于全网完成。 |

当前生产错误没有完整的阶段分类，`Error::SumoModel` 同时承载多种失败；因此若 #806-A 需要机读统计，建议增加 converter 内部的窄诊断分类，而不是从任意错误文本猜原因。是否改变公开诊断格式另行评审，不在本草案预设新 schema。

为保持首片可独立完成，将 #806-A 分成两个有明确边界的子步骤：A0 只修正反例、连续对照及准确阶段断言；A1 才在内部诊断或测试入口增加逐记录的窄分类。两者都不得放宽 `can_complete` / `materialize_path` 的正式成功条件。A1 的分类/输出设计未明确前，不把它混入 A0。

## 4. 两条执行路线比较

### 4.1 默认：保留原车道连续流，补最小真实换道能力

建议保留原 LuST 道路、路口及 source connection 的含义，不把原路口悄悄改写成新合流区。对固定需求可在出发前选好完整 `Route`，使用预制的连续横向过渡；首版不需要运行中自由改 Route，也不需要任意位置生成横向轨迹。

但“预制”只减少选路和几何自由度，不减少安全义务。当前普通 hop 在没有门时会直接允许进入下一边，不能拿普通 successor 充当换道准入。[K1] 当前车身占用又只沿本车路线向车尾展开，目标车道在进入共享边之前不会自动感知换道车。[O1] [O2] [F1] 因而仍须 §5 的最小 #237 子合同及其实现。

### 4.2 条件备选：固定区域完整编制为受控连接单元

`Junction` 的通用定义允许 merge/diverge；同一 Junction 的多条 `ManeuverPath` 可以共享内部边，`Conflict` 已有路线出现项、组合资源仲裁、下游区间、车尾清空和持久化机制。[J1] [K2] [K3] [S1] 这使其成为值得研究的备选，而不是可以立即使用的默认实现。

最低编制范围必须包括：

- 把固定区两侧的源/目标 lane 切成真实上游、连接区和下游边，给出最终身份、连续几何、整数毫米长度及来源映射。
- 至少完整声明 `B0_up→S00→B0_down`、`B1_up→S11→B1_down` 和允许开放的 `B0_up→T01→B1_down`。若开放 `1→0` 或其他斜向路径，也必须逐条完整声明；不能只给换道车一条斜线而让直行车无门、无流、无冲突覆盖。这里的“直行”只穿过新增固定区，不得新增原 source 路口没有的 `B0→C` 等权限。
- 每条相关路径的 `ParticipantStream`、进入门、停止线、等待区、`ConflictZone`/`ConflictPassage`、entry/clearance 锚点和下游物理区间。冲突覆盖必须保守包含车身扫掠区、合流点和车尾，而不是只给 diagonal edge 建一个占用桶。
- 目标车道已进入和尚未进入共享边的车辆、源车道后车、同拍竞争、相反方向斜穿、相邻固定区及下游阻塞。缺失的关系不会被 Runtime 从几何自动发现。[F1] [J1]
- 每个受控进入点的 `ManeuverGate` 必须绑定 `fromEdge` 上的停止线；无适用 policy/规则时门会 `DenyAndStop`。因此匹配的 `WorldPolicySelection` 和让行/间隙规则是运行前置，不是可省略配置。[J2] [K1]
- 非空冲突 cells 必须携带非空 downstream 组合资源；资源检查同时考虑 committed 和 earlier-staged claims。不能只声明冲突区而省略下游清空。[K2]

还需特别注意：`road-junction-model.md` 仍把 merge/diverge 的 gap/priority/lane availability 列为独立 G1 范围。现有通用 Conflict 原语可以复用，不等于已有经过接受的合流/分流 profile 或完备安全证明。[J3]

### 4.3 固有语义变化与推荐

| 维度 | 默认：原车道连续流 + 最小换道 | 备选：固定受控连接区 |
| --- | --- | --- |
| 拓扑与身份 | 保留原道路/路口连接语义，仅增加经接受的横向过渡及必要分段；最终边链仍须重新物化。 | 新增或重编 Junction/Movement/ManeuverPath/Gate/Waiting/Conflict，切分原 lane；来源与最终身份明显改变。 |
| 排队与容量 | 原车道连续流保持；进入换道前的硬停和间隙会改变局部排队，不能把源/目标双投影重复计为两辆车或两份物理容量。 | 直行车同样进入门与冲突仲裁；固定区长度、等待区运营上限、实际车长/间距和下游存储共同改变队列容量。不能把拆分后的边数相加当作新增道路容量。 |
| 等待行为 | 目标间隙不足、许可拒绝或源/目标资源未清空时，必须在进入前等待，恢复后再进入。 | 还可能因全流受控和保守串行化增加等待；直行与斜向同拍竞争必须确定性裁决。 |
| 距离与时长 | 过渡实际长度和运动决定行程时间；不能沿用旧 SUMO 长度或人为假定零时长的横向跳转。 | 新增几何、门等待和资源持有会改变通过时间与行程时间；固定 tick 不因此改变，信号相位时长也不得被静默重标定。 |
| 工作负载 | 目标是保留 pinned 人口和 LuST 来源映射；新产生的可执行结果仍需完整验证。 | 固定换道位置、路径集合、容量和时长均构成派生负载语义，必须单独接受，不能声称复现原任意位置换道。 |
| 当前状态 | 最小 #237 子合同尚未接受/实现。 | 未接受、未验证；现有 Conflict 可复用不代表端到端能力已经交付。 |

**推荐：先用 #806-A 独立暴露缺口；默认随后冻结并实现 #237 的最小能力，在双车道路段验证固定预制过渡。只有在明确接受上述派生负载变化、完成独立 G1 并证明全部冲突覆盖后，才选择固定连接区。** 不得为了让路线通过而在两条方案之间静默切换。

## 5. #237 / #273 最小依赖矩阵

### 5.1 默认方案真正需要的 #237 子合同

#237 的 2026-07 历史候选明确尚未通过 G1；其中的旧 ADR 编号不能指代当前同号文档。当前 [ADR 0019](https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/adr/0019-waiting-zone-conflict-right-of-way-authority.md#L1-L6) 是 Waiting/Conflict 通行权决策，[ADR 0020](https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/adr/0020-compiler-owned-static-network-and-static-image.md#L1-L24) 是编译器拥有的静态路网，不应从历史候选编号直接提取现行接口。[I2]

| 必要合同 | 首版最小内容 | 当前可复用事实与缺口 |
| --- | --- | --- |
| 物理相邻 | 同向 lane 的稳定链身份、方向、共同纵向范围、实际相邻边界；LuST 必须显式重建多 lane 横断面。 | `RoadSection` 的 ordered lanes 与 `RoadCorridor.elements` 是既有事实源；当前 LuST 每条 approach lane 独立成单 lane section，不能按 ID、SUMO index 或空间接近猜邻接。[X1] [C5] |
| 跨越许可 | 边界标线事实与 policy 许可分开；按参与者类别、方向和路线需求判断，未知/拒绝失败关闭。 | 既有 AccessRule 可复用组合模式，但它不是现成的横向邻对许可；静态 allow 也不覆盖运动安全。[X1] |
| 单一路线权威与固定过渡 | 出发前确定完整实际边链，预制过渡有连续几何、明确边界和最终长度；首版不要求活动车改 Route。 | `Route` 与游标可继续拥有实际遍历；若过渡是带中心线的真实边，可复用 lane-pose。不能以“换道”名义隐式重排或替换路径。[D1] [D2] |
| 源/目标物理占用 | 换道期间同一车辆、同一位姿在源和目标车道具有明确的占用投影；车身扫掠区、重复 occurrence 和长车跨边均有定义。不得重复计数车辆。 | 当前占用按物理边分桶，车身仅沿本车路线回溯；不同边即使几何相交也不会自动共享占用。[O1] [O2] |
| 目标前后车与源后车 | 进入前证明目标 leader/follower 的领先与滞后间隙、制动和最小间距；考虑尚未进入共享边的接近后车，并保护源车道后车直到车尾清空。 | 当前 Following 不搜索未选分支或其他 incoming branch。生成准入的双方向证明可参照，但不等于 tick 中的换道证明。[F1] |
| 进入仲裁与硬停 | 在固定边界前设置可停住的进入点，原子取得源/目标所需资源；同拍竞争按稳定顺序裁决，并和 Following、Signal、Waiting、Conflict、Parking 使用同一安全求解顺序。 | 普通无门 hop 当前直接允许通过；Conflict 的 committed + earlier-staged claim 机制可复用，但不能另建一套安全权威。[K1] [K2] |
| 原子提交与车尾清空 | 车辆状态、资源持有和事件作为同一提交生效；拒绝时保留已提交旧状态，源/目标资源持续到相应车尾清空，完成后无残留占用。 | 现有 Conflict 以真实车长判断 `rear >= clearance`，可复用其清空和提交原语；普通斜边本身没有这些语义。[K3] |
| 保存、恢复与重放 | 过渡阶段、路线位置、资源预约、等待和清空事实使用稳定引用完整保存/重建；恢复不重复 grant/事件，重放与命令序列得到相同状态摘要。 | 路线已按 `StableId128` 序列保存，机动/Waiting/Conflict 状态已有持久化；若新增换道状态，必须由 G1 决定快照、恢复、摘要和版本影响，不能只存内存。[S1] |

首版明确排除：潮汐/可逆车道、动态 lane-use 全集、自由在线选道、通用活动车改 Route 入口、任意位置材料化以及全量 `ResolvedLanePlan`。如果未来选择运行中改变计划，再单独设计路线替换和状态迁移，不能把它偷偷加入最小前置。

### 5.2 哪些工作现在可做，哪些等待具体能力

| 工作项 | #237 关系 | #273 关系 | 现在能否独立开展 |
| --- | --- | --- | --- |
| #806-A 合法反例、阶段诊断、连续对照和现行图上的确定性搜索 | 不需要其完成；诊断不冒充换道能力 | 不需要 | **可以**，包括固定逻辑夹具和内部诊断设计/实现评审。 |
| 本来就连续合法的 lane 路径物化与注册检查 | 不需要 | 不需要 | **可以**；最终全网结果仍需同一最终修订。 |
| 默认的固定预制过渡、保留原车道连续流 | 需要 §5.1 子合同及对应生产能力，不是只等设计 Issue 关闭 | 固定边界、显式端点和预制几何时不需要 | 等最小能力；先可独立准备合同与负例。 |
| 固定区域完整受控连接单元 | 不先验依赖整个 #237；不足的横向能力仍须回到相应子合同 | 固定边界方案不需要 | 等独立 G1、完整冲突覆盖和派生负载接受。 |
| 任意位置动态横向轨迹、跨 lane 位置映射 | 需要相应换道语义 | **构成依赖**：需要共享 stationing 等能力 | 不属于首版。 |
| 段级边界 identity 成为必要输入 | 依所选边界合同 | **构成依赖**；现行整边界合同明确推迟段级锚定 | 不属于首版。[X1] [I3] |
| #790 保留连接的局部几何与最终边链接口 | 不自动阻塞 | 不自动阻塞 | 可与 #806-A 并行，最后必须汇合到同一拓扑。 |

#273 明文不阻塞整边界 lane-use/lane-change 与预制 transition 首版。只有当任意纵向位置材料化或共享 stationing 成为必要输入时，才把它登记为真实依赖；不得用长度百分比或最近点投影代替它。[I3]

## 6. 需要与 Claude 对齐的共同接口

以下是跨线待对齐问题，不是已经约定的新公开类型或 schema。未联系 Claude，也未修改其产物。

| 必须共同冻结的输入 | 需要几何/编制线提供的具体内容 | 路线侧消费与拒绝条件 |
| --- | --- | --- |
| 来源与最终边链 | 固定 source revision；每个来源 road/lane 键到最终有序 `LaneEdge` 链的显式映射；拆分、删除、合成边的稳定身份、方向和边界角色。 | 每个道路 occurrence 都从该映射物化。source lane ID 只作来源键，不按字符串、lane index 或运行时 ordinal 猜最终身份；零/多义映射失败关闭。 |
| 完整机动路径 | 每个保留 source connection 对应的最终入口、完整有序内部边、出口及路径身份；共享边的唯一归属；如选固定连接区，还要全部直行/斜向路径。 | 只选择最终拓扑中的完整路径，不覆盖其入口/出口，也不从信号或名称反推路径。路径必须来自同一份最终 `NormalizedTopology`。 |
| 长度与可用道路段 | 最终规范中心线、编译提交的整数毫米边长、两端被路口/改造消耗的范围、剩余可用段及固定过渡边界。 | 不用旧 SUMO `length` 或长度百分比换算跨 lane 位置；不把两条不同弧长当作共享 stationing。边界不足或重叠时明确拒绝/保留未决。 |
| 横向过渡材料 | 合法相邻边界、许可依据、固定过渡的端点/边链、源/目标占用映射；若选固定连接区，则为完整流、冲突和下游资源。 | 诊断发现的 lane pair 只是需求，不自动成为可执行过渡。默认方案与固定连接区不得混用材料。 |
| 控制锚点 | `StopLine` 所在最终边、`ManeuverGate` 的最终 path/`transition_index`、信号组和来源 link、Waiting/Conflict 的 entry/clearance、停车 anchor 的重新绑定。 | 不沿用旧 gate index 或停止线；新增过渡不得绕过原信号，split 后不得把停车位置按百分比搬移。缺失、歧义或绑错边均拒绝。 |
| 修订与制品绑定 | 同一最终拓扑产生的 LFCA/`SharedNetworkRevision`、`NetworkRevisionId`、必要的 LFCA digest/length 和合同版本；明确几何改动后的重建时点。 | 路线目录、人口、Runtime 与 Spatial 消费同一最终修订；不以旧路线注册、旧长度或旧运行证据替代。需要同一 `Arc` 的场合不能只比较 `NetworkRevisionId`。[U1] |

当前 `convert_static_with_due` 在路线分支先归一化，静态制品阶段又归一化一次。现行相同输入下这不证明未来拆边后仍一致；后续可以内部多遍求解，但只能发布一份最终权威拓扑，同时供路线和静态发射消费。[C3]

还需共同回答：**哪些入口/出口会被 #790 移动？移动后每条 source lane 还剩哪些明确可用边界？哪些共享边的路径需要整体重绑？固定过渡能否完全落在显式边界内而避免引入共享 stationing？** 这些问题未解决前，不能把路线搜索接到猜测的最终图上。

路线仍按 ADR 0029 作为每世界可变状态经 `register_route` 注册，不能为了让 LuST 路线进入静态制品而恢复 `StaticRoute` 或另造第二套路线权威。[D2]

## 7. 最小切片与接受/拒绝测试

### 7.1 #806-A：真实反例与分阶段诊断（建议立即独立开展）

**范围**：A0 为 converter 的真实反例、连续对照、准确阶段断言与必要证据注释；A1 为后续内部诊断，输出带 `population_rank`、vehicle ID、road occurrence、source connection、到达/所需 lane 对的记录。两者都不改变 Runtime 安全条件、正式成功路径、人口 seed、人口截断或 Adapter。

**A0 完成边界**：明确声明且确认 B0/B1 均存在；缺口输入在直接路线阶段及 fail-fast 组合入口返回准确的路线错误；连续对照产生精确边链；诊断模式保留相同人口且不产路线目录。未知来源引用与道路 hop 缺失分别验证，不用任意 `Err` 代替阶段断言。编译或信号前置若意外失败，单列其真实错误，不把它算作路线缺口覆盖。

**A1 完成边界**：在 converter 可观察范围内区分来源错误、道路不可达、lane 连续性缺口与物化不完整；按实际调用链记录静态构建结果或“未执行”。Runtime 注册、车辆准入、运行等待不属于 A0/A1 的完成要求，未运行即明确标为“未执行”，交给 #806-B 或另行安排的合法小夹具验证。

**共同约束**：正式转换仍拒绝不完整路线，不发布删减/补位后的 `routes.toml`。A0 不修改生产代码；A1 涉及的内部诊断实现另按确定的范围推进。任何一项完成都不关闭 #806，也不产生换道能力或全网通过结论。

### 7.2 #806-B：一个双车道路段的可执行闭环（等待明确前置）

默认前置为：§5.1 所需合同已接受并具备最小实现；固定 `0→1` 过渡有合法邻接/许可、真实最终几何与长度；两条原车道直行路径和下游受控路口均来自同一最终修订；源/目标占用、前后车安全、进入仲裁、提交/清空及快照合同已绑定。首版不做自由在线选道、任意位置换道或全网推广。

建议的闭环至少包含：

- 一辆使用固定过渡的车、两条车道的直行车，以及能产生目标前车、目标快速后车和源后车的受控输入；
- 一个下游信号化路口，验证过渡前后的停止线、门和 route occurrence；
- 可构造下游阻塞、同拍进入、长车尾和重复 occurrence 的场景；
- 在进入过渡前、过渡中和车尾清空阶段保存/恢复并重放。

若另选固定连接区，必须把它作为不同实验方案重新冻结静态拓扑、policy、冲突资源、派生负载语义及验收，不得在默认切片中直接替换。

### 7.3 接受与拒绝矩阵（全部为建议，未执行）

| 测试组 | 接受条件 | 必须拒绝或不得误判为通过的情况 |
| --- | --- | --- |
| 反例与对照 | §3 的夹具自证、精确路线错误和连续边链全部成立，人口/顺序稳定。 | 任意 `Err`；`B1` 不存在；把 unknown/internal 引用归类为换道缺口；跳过失败人口。 |
| 注册与原子性 | 合法完整路径成功注册，失败前后路线登记表及已有有效路线不变。 | 只放宽筛选的断链；替换出口导致 `ManeuverMismatch`；绕过内部边覆盖；把注册成功写成安全证明。 |
| 单车与位姿 | 单车沿完整最终边链到达，中心线/长度/进度连续，原信号门和停止线正确。 | 凭空换 lane、重复距离、从完整平行 `B0` 末端跳至 `B1` 起点、绕红灯；只凭单车成功验收换道。 |
| 目标与源车道安全 | 目标前/后车、快速接近后车、源后车的间隙不足时进入前停让；安全后可前进。 | 只检查目标前车；进入共享边后才首次感知目标后车；以同一 tick 未提交位移充当其他车的实际位置。 |
| 同拍与多车 | 直行/斜向同拍进入、连续两辆过渡都有确定性结果，资源不足者等待，恢复后继续。 | 只给斜向车建资源而漏掉直行；以遍历先后或 worker 调度决定结果；出现物理重叠。 |
| 车尾与下游 | 长车资源保持到相应车尾清空；下游堵塞时不进入，释放后无残留占用/预约。 | 车头离开即释放；只检查 diagonal edge；无 downstream 的非空 Conflict bundle；把正常等待记为非法路线。 |
| 固定连接区专项 | 所有开放的直行/斜向流都有门、停止线、流、冲突和清空覆盖，policy 明确，容量/等待/时长偏差被记录。 | 缺 policy/规则时不得默许通过（现行门应 `DenyAndStop`）；缺任一直行流、只画冲突区域、把单车通过当完整覆盖。 |
| 持久化与确定性 | 过渡中/Clearing 快照恢复后，与连续运行及重放的规范状态和事件一致；不同 worker 配置下规范结果一致；重复 occurrence 不串用资源。 | 丢失横向阶段、重复 grant/事件、使用旧 ordinal/修订、恢复后残留或提前释放资源。 |
| 结构负例 | 未知邻接、非法许可、缺失最终映射、路径歧义、控制错绑、无法证明制动/清空的输入均失败关闭或保持不可交付。 | 通过放宽注册、安全距离、几何检查或冲突覆盖来使夹具“通过”。固定区过短若已进入运行，也必须停让而非强行穿越。 |

### 7.4 全网交付仍是后续边界

只有所选执行语义和共同接口均通过，才在 #790 形成的同一最终 LFCA/`SharedNetworkRevision` 上进行全网确定性物化、逐条注册和真实 TOPO/DEMAND 验收。分别保留道路可达、可物化、可注册和按既定 workload 窗口完成执行的计数及失败原因；不能用 #806-A 的诊断数量或 #806-B 的小夹具结果替代。

当前已有证据是 v2 入选 10,000 条中 9,344 条不能按现行车道级算法展开；9,350 属于旧 SHA 人口，两者差值不是修复效果。10,592 个候选是先过滤、排序、截断的候选全集，不能用截断尾部替换失败入选记录。本轮没有新运行，因此不提供新的失败数、成功率、容量、等待或时长数据。[P1] [P2]

## 8. 未执行的验证方案与命令模板

以下命令**全部未执行**，仅供后续对应实现切片获准后使用；占位测试过滤器必须替换为实际测试。它们不是本轮结果，也不授权现在构建或运行全网转换。

```bash
# #806-A：修正后的既有验收测试与新增路线阶段测试。
cargo +1.99.1 nextest run --locked -p laneflow-lust-converter diagnostic_mode_skips_route_expansion_but_keeps_population
cargo +1.99.1 nextest run --locked -p laneflow-lust-converter '<new-route-stage-test-filter>'

# #806-B：双车道、竞争、清空、快照和重放测试；按实际测试名替换。
cargo +1.99.1 nextest run --locked -p laneflow-runtime '<new-two-lane-test-filter>'

# 最终全网阶段：固定 LuST 来源、独立且尚不存在的输出目录；沿用既有入口。
# 运行后必须报告新证据，不能把既有 9,344 改写成本次新运行结果。
LUST_SOURCE_DIR='<pinned-LuST-checkout>' \
LUST_MIGRATION_OUTPUT_DIR='<fresh-output-directory>' \
cargo +1.99.1 test --release --locked -p laneflow-lust-converter --lib \
  convert::population::migration::pinned_lust_population_migration \
  -- --exact --ignored --nocapture
```

最后一个入口是现有 v2 人口/路线诊断入口，不是完整可执行路网验收；最终全网还需按届时接受的 TOPO/DEMAND 合同运行完整构建、注册、运行与确定性验证。[P1]

## 9. 基线差异、实际操作与未验证范围

以下为原作者的操作记录。Codex 本次修订只核对源码/合同和更新文档，未实现或执行 A0/A1/#806-B。

- 冻结提交至主 checkout 开工 HEAD 的专项比较中，任务指定的 converter `routes.rs`/`acceptance.rs`/`topology.rs`/`output/emit.rs`、Runtime `tables.rs`/`occupancy.rs`/`conflict_tick.rs`，以及 ADR 0029、route-system、cross-section-access、road-junction-model、Conflict 相关合同均无差异。区间其他变更属于 #1071 研究/治理文档及 CI 措辞；未把它们静默纳入新基线，也未扩展为全仓审计。
- 实际创建了 detached 独立工作树，读取根 `AGENTS.md` 指定入口、governance/core-design/pre-1-0 Skills、相关 ADR/design、冻结源码和既有报告；使用 `git show`/`git grep <SHA>` 核对冻结对象，并以 `gh` 只读核实三个 Issue 及 #237 历史评论。
- 除独立工作树检出外，内容交付只写本文件。未修改 Rust、测试、正式 ADR/design、依赖、Claude 产物或 GitHub 状态；未创建 PR、未提交/推送、未联系 Claude。
- 未启动构建，未运行任何测试、转换、仿真或长时间运行，未产生新统计。反例中的错误类别、拟议夹具、方案可行性、双车安全、容量/等待/时长影响与全部接受测试均为推导或未验证建议。
- 特别未证明：既有失真测试的实际错误来源；固定连接区的完整冲突覆盖和合流 profile 是否足够；#790 后可用于过渡的真实路段范围；pinned LuST 的最终可注册/安全执行比例。上述问题必须留给相应实现与验证阶段，不得以本草案关闭 #806。

## 证据索引

所有源码/合同链接均指向冻结提交；Issue 链接是 2026-10-26 的实际只读来源。

- [C1] `tools/laneflow-lust-converter/src/convert/routes.rs:83-230`，搜索、筛选、物化与错误文本。
- [C2] `tools/laneflow-lust-converter/src/convert/acceptance.rs:375-435`，既有测试注释、XML 与断言。
- [C3] `tools/laneflow-lust-converter/src/convert/topology.rs:103-193,255-317`，两阶段调用、人口保留和空 successor。
- [C4] `tools/laneflow-lust-converter/src/convert/junction.rs:31-127,205-260,313-399`，来源路径解析、完整序列与归一化输出。
- [C5] `tools/laneflow-lust-converter/src/output/emit.rs:337-455`，每条 approach edge 独立 corridor/section 与单 lane 成员。
- [C6] `tools/laneflow-lust-converter/src/convert/signals.rs:219-285`，逐 lane 停止线、路径查询和 `transition_index = 0`。
- [P1] `research/issue-1053-lust-population-v2/README.md:10-40`，v2 人口、9,344/9,350 口径与既有验证入口。
- [P2] `research/issue-1053-lust-population-v2/baseline.json:23-36`，既有机器固定计数。
- [R1] `crates/laneflow-runtime/src/kernel/tables.rs:397-540,1141-1178`，连通、完整路径匹配及失败顺序。
- [D1] `docs/design/route-system.md:81-195,348-369`，实际遍历、注册边界、失败原子与出现项。
- [D2] `docs/adr/0029-retire-precompiled-static-route.md:57-96,170-203`，每世界路线、注册入口和快照边界。
- [X1] `docs/design/cross-section-access.md:154-219,747-782`，相邻事实源、整边界、#237 消费与横向几何边界。
- [J1] `docs/design/road-junction-model.md:107-259,800-828`，通用 Junction、完整路径及编制安全责任。
- [J2] `docs/design/road-junction-model.md:457-524`，Gate、transition 与 StopLine 绑定。
- [J3] `docs/design/road-junction-model.md:985-1001`，Conflict 与 merge/diverge 的独立 G1 范围。
- [O1] `crates/laneflow-runtime/src/kernel/occupancy.rs:79-101,1521-1569`，物理边占用桶。
- [O2] `crates/laneflow-runtime/src/kernel/tables.rs:1487-1512`，车身沿本车路线展开。
- [F1] `docs/design/vehicle-following.md:258-289`，未选分支、目标前后车与跨分支 overlap 边界。
- [K1] `crates/laneflow-runtime/src/kernel/tick.rs:4500-4594`，普通 hop、门许可与无 policy/规则时失败关闭。
- [K2] `crates/laneflow-runtime/src/kernel/conflict.rs:3645-3711`，组合资源约束及 committed/earlier-staged 检查。
- [K3] `crates/laneflow-runtime/src/kernel/conflict_tick.rs:923-981,6944-7043`，车尾清空和下游预检。
- [S1] `crates/laneflow-runtime/src/admin/snapshot.rs:160-289`，路线、机动、Waiting、Conflict 与下游状态的稳定引用。
- [U1] `docs/design/traffic-runtime-shared-consumption.md:227-287`，同一修订、路线注册和车辆准入。
- [I1] [Issue #806](https://github.com/illusion-tech/laneflow/issues/806)，问题陈述与未接受的“任意 lane”修复方向。
- [I2] [Issue #237 历史候选评论](https://github.com/illusion-tech/laneflow/issues/237#issuecomment-5089376313)及[候选修订](https://github.com/illusion-tech/laneflow/issues/237#issuecomment-5091077469)，明确不是 G1 Pass。
- [I3] [Issue #273](https://github.com/illusion-tech/laneflow/issues/273)，横向几何目标及不阻塞整边界/预制过渡首版的范围。

[C1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/tools/laneflow-lust-converter/src/convert/routes.rs#L83-L230
[C2]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/tools/laneflow-lust-converter/src/convert/acceptance.rs#L375-L435
[C3]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/tools/laneflow-lust-converter/src/convert/topology.rs#L103-L193
[C4]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/tools/laneflow-lust-converter/src/convert/junction.rs#L31-L127
[C5]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/tools/laneflow-lust-converter/src/output/emit.rs#L337-L455
[C6]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/tools/laneflow-lust-converter/src/convert/signals.rs#L219-L285
[P1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/research/issue-1053-lust-population-v2/README.md#L10-L40
[P2]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/research/issue-1053-lust-population-v2/baseline.json#L23-L36
[R1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/kernel/tables.rs#L397-L540
[D1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/route-system.md#L81-L195
[D2]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/adr/0029-retire-precompiled-static-route.md#L57-L96
[X1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/cross-section-access.md#L154-L219
[J1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/road-junction-model.md#L107-L259
[J2]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/road-junction-model.md#L457-L524
[J3]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/road-junction-model.md#L985-L1001
[O1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/kernel/occupancy.rs#L79-L101
[O2]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/kernel/tables.rs#L1487-L1512
[F1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/vehicle-following.md#L258-L289
[K1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/kernel/tick.rs#L4500-L4594
[K2]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/kernel/conflict.rs#L3645-L3711
[K3]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/kernel/conflict_tick.rs#L923-L981
[S1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/crates/laneflow-runtime/src/admin/snapshot.rs#L160-L289
[U1]: https://github.com/illusion-tech/laneflow/blob/78f0d66c53431175de57d13e626d6a88231246c3/docs/design/traffic-runtime-shared-consumption.md#L227-L287
[I1]: https://github.com/illusion-tech/laneflow/issues/806
[I2]: https://github.com/illusion-tech/laneflow/issues/237#issuecomment-5089376313
[I3]: https://github.com/illusion-tech/laneflow/issues/273
