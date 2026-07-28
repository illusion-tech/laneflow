# 路网编译器与数据供应链范式

**文档状态**: Accepted（#291 G1 冻结核心范式与四个决策点；L2 细节为方向性，待 L2 G1）<br>
**最后更新**: 2026-07-28<br>
**适用范围**: #291 冻结的编译器时代数据供应链范式、L1/L2 编译器架构、身份派生契约、source map 与诊断、双产物与独立校验器、双 SSOT 过渡期纪律、canonical 产物退役条件与路线图<br>
**实现状态**: 未实现；L1 拓扑生成器由 #292 跟踪，L2 几何编译器待 #291 G1 后拆分

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0018-multimodal-cross-section-and-access-overlay.md`
- `../adr/0019-waiting-zone-conflict-right-of-way-authority.md`
- `data-format.md`
- `data-loading.md`
- `cross-section-access.md`
- `waiting-zone-conflict-right-of-way.md`
- GitHub: #72、#199、#236、#237、#281–#285、#291、#292

## 1. 目标、状态与非目标

### 1.1 目标

本文冻结编译器时代数据供应链的完整范式：

- 编译器时代定义：几何文档为 authoring SSOT、确定性多 pass 编译管线为数据生产方式、独立校验器为预言机、双产物分别服务治理与加载；
- L1 拓扑生成器（scenario builder DSL）与 L2 几何编译器（曲线几何文档 → LaneGraph）的能力边界；
- 身份派生契约（identity stability）：LaneEdge/Junction/Movement/ManeuverPath 等身份从输入稳定锚点派生的规则；
- source map 与诊断架构：产物 → 几何文档的追溯，rustc 级诊断质量；
- 数据供应链范式：信任边界上移（loader verifier 化）、双产物、双 SSOT 过渡期纪律、canonical 产物退役条件；
- 验证架构：编译器 golden test、性质测试、独立校验器、端到端管线验证；
- 与 #72（城市级性能分层）、#236（非机动车/步行）、#237（动态车道）及编辑器愿景的关系；
- 路线图与风险登记。

### 1.2 当前生产

| 范围             | Current production                                                                    |
| ---------------- | ------------------------------------------------------------------------------------- |
| 数据生产方式     | 手写 canonical JSON + `tools/laneflow-corridor-generator`（内部 TOML 生成器）          |
| 治理产物         | Traffic v0.10 / SpatialPackage v0.1 / ScenarioManifest v0.1 canonical JSON            |
| 加载路径         | `laneflow-data` JSON loader（version header → strict DTO → 转换 → Core 规范化）        |
| 验证场景构造     | `tests/support/` Rust 代码手工拼装展开拓扑（约 4800 行）                                |
| 编译器           | 不存在；L1 由 #292 跟踪，L2 待拆分                                                      |

退役条件（§10.4）触发前，现有 canonical JSON 管线继续服役，其 canonical、provenance、immutable publication 纪律（`data-format.md`、`data-loading.md`）继续有效。

### 1.3 非目标

- 不实现编辑器前端（引擎插件/游戏内编辑 UI）、不实现城市模拟器（#72 题域）。
- 不改变 Core runtime 行为与公开 API；LaneGraph/Traffic schema 语义不变，变化的是数据的生产方式。
- 不立即废除现有 canonical JSON 管线；退役以显式条件触发。
- 不承诺 OSM 导入与二进制镜像格式的具体选型（依赖 #72 的规模与消费方证据）。
- 不冻结车道宽度、横向几何（延续 cross-section-access.md 的边界）。
- 不恢复"手写展开拓扑"作为长期场景生产方式。

## 2. 背景与动机

### 2.1 现状痛点（实证）

- **边界控制代码**：`laneflow-data` 维持约 2500 行边界控制代码——474 行手写 strict DTO（`wire.rs`）、`non_null_option` null 语义防御（`timeWindows` 显式 null 会绕过 capability guard）、`access_priority_lexeme` 数值字面量词法保留（`serde_json::Number` 的 f64 归一化丢精度）、2037 行转换校验（`lib.rs`）。约束存在四份人工同步副本：JSON Schema 文件、wire DTO、loader 转换、Core 构造器。
- **场景构造代码**：`crates/laneflow-core/tests/support/` 约 4800 行 Rust 代码直接拼装展开拓扑对象，route/gate/phase 引用一致性靠作者小心；组合一致性错误在展开产物层面才暴露。
- **migration 覆盖缺口**：atomic migration 只覆盖数据文件；场景构造代码不受保护，v0.9→v0.10 演进已演示一次全量手工适配（#281 的 occurrence 语义落地）。
- **迁移成本事故**：#144 的 f32 数值迁移因性能门槛失败而完整回退，证明"数据为一等公民"时代的迁移成本可以杀死变更。
- **拓扑密度趋势**：#281 已交付 multi-Gate/Waiting static；#282–#285（暂停中）将把 occurrence、zone-stream passage 等拓扑密度继续推高；#236/#237 排队在后。手写展开拓扑的成本曲线不可持续。
- **城市级愿景**：#72 要求几何编辑、OSM 导入、增量编译与高效加载能力。

### 2.2 先例

- SUMO：netconvert/netedit 与仿真内核同仓，几何→拓扑编译是成熟范式；其教训是重跑 netconvert 身份全漂移，下游配置生态长期靠 fragile 补丁维持——身份派生契约（§8）是本文的第一优先级。
- Java：javac 与 JVM bytecode verifier 的关系——编译器是自己人，但加载器仍验证产物（手改、版本错配、存储损坏、第三方编译器），防线降级而非拆除。
- reproducible builds：验证方式从"审查产物字节"升级为"独立重编译产出同 hash"，可验证性更强。

## 3. 编译器时代范式

### 3.1 两次反转

**数据角色反转**：Traffic JSON 从"源代码"变为"object code"——仍是治理产物与评审对象，但不再是 authoring 对象；几何文档才是长期资产，产物可再生。

**信任边界上移**：现在的信任边界在"JSON ↔ Core"，loader 层层设防；编译器时代的信任边界上移到"人类意图 ↔ 编译器"，"编译器 ↔ Core"降级为 verifier 关系——从"防御任意恶意输入"降级为"验证友军产物"。

### 3.2 编译概念族迁移

| 编译概念        | 数据供应链对应物                                                  |
| --------------- | ----------------------------------------------------------------- |
| 源代码          | 几何文档（git 长期资产，评审对象）                                  |
| 编译器          | L1 拓扑生成器 / L2 几何编译器                                      |
| object code     | canonical JSON（治理 SSOT，退役条件触发前）                        |
| 优化产物        | 二进制加载产物（派生物，内嵌源 JSON 的 SHA-256）                    |
| linker/loader   | `laneflow-data` loader                                            |
| bytecode verifier | loader 校验（版本配对 + hash + 结构 fast-path）                   |
| source map      | 产物 → 几何文档追溯（§9）                                          |
| 增量编译        | 编辑影响域分析                                                    |
| AOT 常量折叠    | 编译期横断面展开/准入预解析/connection 几何预计算                   |

编译期常量折叠是性能的真正大头：加载时计算全部前移到编译期，终态产物直接携带答案；城市级规模（#72）下加载产物可为 Core 内存镜像，加载趋近 mmap。

### 3.3 终态数据流

```text
authoring  编辑器 / L1 DSL / OSM 导入器
             ↓ 几何文档（git 长期资产，评审对象）
compile    L1 → L2，多 pass 纯函数管线（§7）
             ↓ canonical JSON（治理 SSOT）＋ 二进制镜像（派生物）
             ↓ ＋ source map ＋ provenance（编译器版本/SHA-256）
verify     独立校验器（§11 预言机）→ loader（verifier：版本配对＋布局）
             ↓
runtime    Core 规范化结构（零改动）
             ↺ 热编辑：编辑 → 增量编译 → patch → 状态迁移事务
```

## 4. 角色与职责

- **几何文档**：唯一 authoring SSOT；人可评审、可序列化、可撤销；自身演进通过编译器前置 upgrade pass（§6.4），与编译器同 PR 发布。
- **编译器**：数据生产工具；多 pass 纯函数；单向依赖 Core 数据类型；不进入 runtime 依赖链。
- **独立校验器**：与编译器独立实现的预言机，只与编译器共享约束 SSOT；消费产物并裁决其合法性。
- **loader**：verifier 化——版本配对、hash 校验、结构 fast-path 校验、内存布局；不再承担"防御任意恶意输入"的边界控制。
- **Core**：零改动；只消费规范化结构。
- **编辑器**（远期，#72 愿景）：几何文档的交互前端；编译内核在 LaneFlow 仓库，交互壳在产品仓库。

## 5. L1/L2 能力边界

### 5.1 L1 拓扑生成器（#292）

- 输入：代码 DSL（builder API，享受类型检查与参数化）；
- 输出：展开后的 Core 输入结构；
- 域：合成形状（正交路口、直路段、规则阵列）；
- 交付：展开引擎、identity 派生 L1 子集、内置校验（复用 runtime 校验代码）、1–2 个既有场景迁移试点、golden test；
- 价值：吸收 tests/support 场景构造的展开复杂度；schema/拓扑演进时单点适配；服务 #282–#285 恢复时的拓扑密集验证。

### 5.2 L2 几何编译器

- 输入：几何文档（曲线 centerline + 横断面模板 + 设施附着）；
- 输出：Traffic/Spatial 产物（双产物形态见 §10.2）；
- 新增能力：曲线几何、交叉口 connection 自动成形（自动 + 覆写混合）、junction 推断与确认、编辑影响域增量编译、source map；
- 前置：L1 交付、#291 G1 冻结几何文档模型。

### 5.3 边界矩阵

| 能力                     | L1   | L2   |
| ------------------------ | ---- | ---- |
| 展开引擎与 identity 契约 | ✓    | 继承 |
| 代码 DSL 输入            | ✓    | ✓    |
| 曲线几何文档输入         | —    | ✓    |
| connection 自动成形      | 合成域 | ✓  |
| source map               | 简化 | ✓    |
| 增量编译                 | —    | ✓    |
| 双产物 emit              | —    | ✓    |
| OSM 导入                 | —    | 共享 L2 内核，独立切片 |

## 6. 几何文档模型（L2）

### 6.1 四要素

- **centerline 为骨架**：一条路 = 一条带高程的三维曲线 + 附着属性；
- **横断面模板附着**：消费 ADR 0018 的 RoadCorridor/RoadSection/FacilityBand/FacilityKind/AccessRule 模型；"加一条非机动车道" = 改横断面模板参数，不重画几何；
- **junction 半显式**：centerline 端点在容差内交汇推断为节点候选，**连接与否显式确认**；ADR 0017 的"立体交叉不因俯视几何相交自动成 Junction"落实为文档的表达能力——两条 centerline 三维相交时必须区分"互通"与"跨越"，默认不连接；
- **设施与规则附着**：信号挂载点、停车位、限速、AccessRule 时间/地区 overlay、WaitingZone/ConflictZone 声明（ADR 0019），全部作为 centerline/横断面/junction 上的附着属性，不产生第二份手绘数据。

### 6.2 曲线表示（#291 G1 已冻结边界）

G1 冻结：L1 直线 + 规则几何；L2 采用 curve segment 弧长采样抽象，具体段类型集合待 L2 G1 以 numeric-representation 纪律下的确定性/性能验证决策。候选：直线-圆-回旋线三单元（clothoid，道路工程标准缓和曲线，曲率随弧长线性变化、真实路网保真最高；Fresnel 积分无解析式，需固定阶数级数 + 误差预算）vs 贝塞尔（控制点多项式，计算廉价、交互直观，游戏行业惯例；曲率非线性且不能精确表圆）vs 混合段类型模型（道路设计软件实际形态）。产物恒为折线（SpatialPackage `centerline.points`），曲线表示只存在于文档与编译器内部，runtime 数值成本为零。

### 6.3 authority

几何文档是唯一 authoring SSOT；Traffic 产物是编译产物，永不手改（硬约束，消除 round-trip 问题）。

### 6.4 文档演进（migration 的转移）

几何文档自身演进（v1→v2）通过编译器前置 upgrade pass：旧文档进、新文档出，与编译器同仓同 PR 发布。产物 migration 被重编译替代（可再生的免迁移）；不可再生的仅 runtime 存档/snapshot 与几何文档本身——存档迁移走既有 snapshot 纪律，文档迁移走 upgrade pass。

## 7. 编译管线 pass 架构

每个 pass 纯函数、独立单测、golden test；pass 间不丢 source span。

```text
parse      文档 → 内存图（曲线/模板/附着物）
normalize  几何清理：端点 snap、重叠检测、容差归一
junction   节点推断 + 连接确认（产出 Junction 候选）
expand     横断面模板 → 车道级几何（LaneEdge centerline 偏移生成）
connect    交叉口 connection 生成（lane-to-lane + 过渡曲线）
identify   按 ADR 0017 分配 Junction/Movement/ManeuverPath 身份（§8）
facilities 信号/停车/WaitingZone/ConflictZone 输入生成
validate   内置校验（复用 runtime 校验代码，见 §11）
emit       canonical JSON ＋ 二进制镜像 ＋ source map ＋ provenance
```

`connect` 是算法密度最高的 pass：movement 分类（左/直/右/掉头）、车道配对（含车道数不等的 merge/diverge 策略）、平滑过渡曲线；默认策略 + 单 connection 手工覆写（覆写作为文档一部分持久化，重编译不丢）。

## 8. 身份派生契约（identity stability）

### 8.1 原则（#291 G1 已冻结）

身份 = 纯函数（稳定锚点），#291 G1 冻结的锚点组合：

- **LaneEdge**：corridor UUID + 方向 + 车道序号；
- **Junction**：几何文档显式 UUID；
- **Movement**：junction UUID + 进/出 corridor UUID + movement 类别；
- **ManeuverPath**：movement 身份 + 出/入 edge 身份 + occurrence 序号。

编译过程**不得**引入自增序号、哈希随机性、迭代顺序依赖；验证 = golden test（逐字节）+ "无关改动不变性"契约测试。

### 8.2 契约要求

- 同一几何文档输入 → 逐字节一致产物（golden test）；
- 无关改动（未受影响区域）不得改变既有身份；
- 受影响集合精确可计算（增量编译与运行时 patch 的前提）。

### 8.3 为什么是第一优先级

identity 是数据兼容性地基，与 schema immutable 纪律同级：增量编译、运行时热编辑、AccessRule/信号配置引用、存档兼容全部站在它上面；一旦发布不可回头。SUMO 的身份漂移教训是本契约的反例。

## 9. source map 与诊断

- 产物携带 source map：每个 LaneEdge/Movement/ManeuverPath 可追溯到几何文档的哪个 centerline/junction；source map 的键即 §8 身份。
- 诊断质量对标 rustc：带 span（哪条 centerline 的哪一段）、带原因（如"横断面不连续：上游 3 车道下游 2 车道且无过渡段"）、带建议。
- 两类错误分离：authoring 错误（编译期，指向几何文档元素）vs 产物损坏/版本错配（加载期，面向运维）。
- 编辑器诊断体验：错误标注在编辑器画布而非产物行号。

## 10. 数据供应链范式

### 10.1 信任边界与 loader verifier 化

loader 从"防御任意恶意输入"降级为 verifier：版本配对（产物 provenance 记录编译器版本与约束 SSOT 版本，过旧产物显式拒绝并要求重编译）、hash 校验、结构 fast-path 校验、内存布局。现有 2500 行边界控制代码随"JSON 作为输入格式"退役而大幅收敛。

### 10.2 双产物

- **canonical JSON**：治理产物——schema 演进、provenance、PR 评审 diff、golden test、诊断。退役条件触发前保持 SSOT 地位；JSON 的松散成本在编译器时代归零（机器生成、约束单一来源在编译器、emitter 精确控制数值渲染）。
- **二进制加载产物**：派生物——内嵌源 JSON 的 SHA-256；地位永远不是 interchange format（与 corridor-generator 内部 TOML 同一条禁令）；选型（rkyv / capnp / flatbuffers）推迟到 #72 的规模与消费方证据。

### 10.3 双 SSOT 过渡期纪律

几何文档与 canonical JSON 同时在库期间，两权威必漂移，纪律二选一并写死：

- 产物完全 CI 生成不入库（干净，失去产物 diff 评审可见性）；
- 产物入库 + CI 强制"重编译 hash 一致"门禁（lock 文件模式，保留评审可见性）。

本文推荐后者：治理评审需要看见下游变化。#291 G1 已按推荐冻结——产物入库 + CI 强制重编译 hash 一致门禁；CI 机制细节在 L2 G1 细化。

### 10.4 canonical 产物退役条件

全部满足方可启动 superseding ADR：

1. L2 编译器 G4 完成（含 identity 契约、source map、确定性 golden test）；
2. 独立校验器在位（与编译器独立实现的预言机）；
3. 二进制镜像 loader 在位（版本配对 + hash 校验 + 结构 fast-path），格式选型有 #72 证据支撑；
4. publication 三元组（几何文档版本 + 编译器确定性构建 hash + 产物 hash）流程冻结并经一个完整 schema 演进周期实弹验证；
5. 存量资产归档方案冻结（v0.9/v0.10 immutable publication 登记保留为历史，不删除）。

退役后 JSON 退位为 export 格式（调试/互操作/外部工具），保留但无治理权威。

### 10.5 远期选项：SSOT 上移完全体

产物不再入库，SSOT 只剩几何文档；评审对象为文档 diff，产物仅留 hash 进 golden test；发布渠道照常发布产物。对治理文化冲击最大，作为 §10.4 之后的演化方向记录。

## 11. 验证架构

```text
编译器 golden test   同输入 → 逐字节同输出（确定性 + identity 稳定性）
编译器性质测试       任意几何输入 → 输出必过独立校验器
独立校验器           与编译器独立实现，只共享约束 SSOT（预言机）
loader 契约测试      版本配对、损坏拒绝
端到端               几何文档 → 编译 → 加载 → runtime 行为
回归基线             固定复杂路口集（未来含 OSM 导入区域）作为演进回归
```

**独立校验器**是编译器时代必须新增的角色，回答单点污染风险：手写数据时代错误是局部的，编译器 bug 是全局系统性污染。校验器与编译器独立实现（共享的只有约束 SSOT），充当测试预言机（多实现互验、wasmtime oracle fuzzing 的先例）。"编译器放行的数据 runtime 必然接受"靠校验器与 runtime 消费同一份代码保证，不靠信任编译器。

## 12. 与现有 SSOT 的关系

- **ADR 0001**：编译器是"LaneFlow 数据的生产工具"（authoring-tool 切片），属 LaneFlow 生态，不扩大项目范围；城市模拟器若立项为独立仓库的下游消费者。
- **ADR 0017**：Junction/Movement/ManeuverPath 身份体系与立体交叉语义是 identify pass 与几何文档 junction 模型的输入。
- **ADR 0018**：横断面模型是 expand pass 的展开输入，不重冻结。
- **ADR 0019**：WaitingZone/ConflictZone 声明作为几何文档附着物；#282–#285 恢复后以生成场景承担拓扑密集验证。
- **data-format.md / data-loading.md**：canonical、provenance、immutable publication 纪律继续有效至 §10.4 触发。
- **#72**：城市级性能分层是二进制镜像选型与零拷贝加载的证据来源；本架构是 #72 愿景的数据供应链侧。
- **#199**：Core API no-regret constraints 约束编译器对 Core 的消费方式。
- **#236 / #237**：非机动车/步行 traversal 与动态车道是编译器的未来负载；其 G1 结论作为几何文档横断面/设施模型的输入。
- **corridor-generator**：已是"内部 DSL → canonical JSON → production loader 校验"的模式雏形；L1/L2 是该模式的正式化与扩展，其"内部 TOML 不得成为 interchange format"的禁令原则延续到二进制产物。

## 13. 路线图

```text
阶段 0（当前）  canonical JSON 管线继续服役；新功能开发暂停
阶段 1         #291 G1：冻结 L1/L2 边界、identity 契约、几何文档模型、双产物与退役条件
阶段 2         #292 L1 拓扑生成器（G0 已记录，Blocked by #291 G1）
阶段 3         #282–#285 恢复（L1 就位后，验证场景由编译器生成）
阶段 4         L2 几何编译器（几何文档、connection 成形、source map、增量编译）
阶段 5         双产物与独立校验器；二进制镜像 loader
阶段 6         §10.4 退役条件评估 → superseding ADR → canonical 产物退位
远期          编辑器前端、OSM 导入、SSOT 上移完全体（#72 愿景侧）
```

#282–#285 恢复条件：#292 G4 完成；恢复时其拓扑密集验证场景改由编译器生成承担（暂停是换序而非取消）。#288（热/冷错误拆分）为独立性能卫生切片，暂停与恢复不依赖本路线，恢复时复核热路径结论是否受场景构造方式变化影响。

## 14. 风险登记

| 风险                       | 影响                                   | 缓解                                                         |
| -------------------------- | -------------------------------------- | ------------------------------------------------------------ |
| 编译器单点污染             | bug 造成全局系统性数据污染             | 独立校验器（预言机）；golden/性质测试；回归基线              |
| 双 SSOT 过渡期漂移         | 几何文档与产物不一致                   | §10.3 入库 + CI hash 门禁（推荐）或产物不入库                 |
| 通行权停留 static 层       | #282–#285 暂停期间 runtime 能力缺口    | 已登记；L1 交付后优先恢复；暂停为换序                         |
| 身份契约发布后不可回头     | 兼容性压力集中在契约设计               | 契约作为 #291 G1 第一优先级；golden/契约测试                  |
| connection 自动成形复杂度  | L2 核心算法不可达完备                  | 默认策略 + 手工覆写混合；覆写持久化于文档                     |
| 几何文档人机工程学不足     | authoring SSOT 手写成本回潮            | L1 代码 DSL 先行验证内核；编辑器（远期）解决交互              |
| migration 转移至文档层     | upgrade pass 成为新成本点              | 与编译器同 PR 发布；语义化映射较产物迁移便宜                  |
| 二进制镜像选型悬置         | 加载性能收益延迟                       | 推迟至 #72 证据；近期 gzip 零架构成本过渡                     |

## 15. 非目标重申与边界

本范式只重构数据供应链；Core runtime 的数值纪律、确定性、API 边界一寸不动。编译器的第一批用户是 LaneFlow 自己的验证场景与示例（#282–#285 恢复），不是城市模拟器；城市模拟器愿景由 #72 承载，本架构作为其数据供应链侧的兼容方向。
