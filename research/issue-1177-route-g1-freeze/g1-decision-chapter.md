# #237 最小子合同 G1 冻结决策章（F-2，#1177）

**文档状态**: 定案材料（G1 决策包一章；供统筹汇总 `g1-decision-package.md`）
**最后更新**: 2026-10-30
**适用范围**: #237 最小子合同七项未决的定案提案（决策项 / 可自决项分列）、
执行能力检查定位裁决记录、生产 G1 缺口登记
**关联文档**: [slice-s1-s3.md](slice-s1-s3.md)、[plan.md](plan.md)、
`research/issue-1142-route-design/design.md`（边界设计）、
`research/issue-1142-route-design/slice-plan.md`（S0–S6 与验收矩阵）

陈述纪律同 route-design §0：事实 / 推导 / 建议 / 未验证分标；本章不授权生产
实现，不放宽生产路线行为；T01 承载形式、占用映射、执行能力检查在 G1 裁定前
保持待冻结。

## 1. 决策项清单（需 G1 裁定）

### D1. T01 承载形式

- **建议值**：带中心线真实 `LaneEdge` + 普通 successor（`B0_up→T01→B1_down`）；
  必要分段同切两 lane（`B-up`/`TZ`/`B-down`，lane 纵向共延伸不变量）；T01 为
  经授权规则派生的编制材料时保存来源映射与构造依据（见 §2 S4）。
- **备选**：
  - (a) Junction/ManeuverPath-managed 过渡区（= 备选固定连接区）：全流门/
    停止线/冲突/清空覆盖，派生负载（排队/容量/行程时间）需独立 G1 与完整
    冲突覆盖证明；不与默认方案混用材料；
  - (b) 无真实边横向 overlay：需跨 lane 位置映射/共享 stationing（#273 依赖），
    首版排除。
- **影响面**：最终拓扑身份（拆分/合成边按逐修订身份 + 文档化映射登记）；
  路线边链物化（如 `[A0, I1, B0_up, T01, B1_down, I2, C0]`）；`register_route`
  静态序列合同（普通 successor 即可，无新增注册校验类别）；ManeuverPath 覆盖
  兼容声明（T01 若纳入 Junction path 须完整 entry/internal/exit，不能只画斜线）；
  merge/diverge 适用边界（`road-junction-model.md` §17.4，见 §4 G-C）。
- **依据**：`research/issue-1142-route-design/design.md` §2.4/§6.1；
  `road-junction-model.md` §3.2/§3.3；#806 方向裁决（选项 3）。
- **决策请求**：确认建议值为 v1 唯一承载形式；确认备选 (a) 维持独立 G1、
  两案不混用；备选 (b) 维持排除（#273 触发条件见 §2 S1）。

### D2. 占用投影公式与 overlap 求解顺序

- **建议值**（v1 整边保守投影，统筹 2026-10-30 确认为基线）：
  1. 车辆占用 `T01` 期间，在同区共横向边（样本 `S00`/`S11`）各记一条**全长
     保守投影**占用记录（`lo_mm=0`、`hi_mm=length`，带投影标记）；本车记录按
     `VehicleHandle` 自排除；
  2. 求解顺序：P1 重建占用索引时实际记录与投影记录同表 → 跟车/准入查询两者
     均视为占用（后杠间隙照常）→ §6.4 物理重叠校验**只对实际车体记录**执行
     （两辆不同车的实际重叠仍非法，不因投影合法化）→ 投影↔他车实际记录的
     重叠由准入/仲裁保证不可达，提交后出现按状态完整性违例处理；
  3. 不把双投影计为两辆车或两份物理容量。
- **备选**：部分区间投影（按车身扫掠精确展开）——需跨 lane 位置映射（共享
  stationing），推迟到 #273/横向几何 G1；v2 演进（若有）G1 后另议。
- **影响面**：`OccupancyIndex` 记录形状（新增投影标记）；跟车/准入查询面；
  `vehicle-following.md` §6.4 判据适用面；容量计数口径；S4 验收断言（投影
  不可达性）。
- **依据**：`vehicle-following.md` §6.4/§7；route-design §2.3 子合同 4、
  §6.2；统筹裁决 1（2026-10-30）。
- **决策请求**：确认 v1 公式与求解顺序为冻结基线；确认 v2 演进另议。

### D3. 仲裁顺序落点

- **建议值**（对齐 `traffic-runtime-phase-protocol.md` P0–P8）：
  - **P3 资源请求**：过渡进入请求作为组合资源 bundle（源边 + 目标边 +
    过渡区边 + 下游清空区间），与 Waiting 本地预选、Conflict 请求同阶段；
  - **P4 确定性联合裁决**：all-or-nothing，基线 C(T) + 较早成功 staged
    claims；同拍竞争按稳定顺序（live 序）裁决；no-grant = 在 `sl-t01` 前
    等待（非错误、非非法路线）；
  - **P5**：正式运动从 C(T) 计算（预览无提交权）；**P7**：状态/资源/事件
    单次原子提交，车尾清空复用 Conflict 真实车长清空判据（`rear >=
    clearance`，不随车头离开释放）；
  - 普通直行流（`B0_up→S00→B0_down`、`B1_up→S11→B1_down`）不进入该仲裁。
- **备选**：独立换道求解器（否：另建第二套安全权威）；P2 预览期裁决（否：
  预览无提交权）；普通 successor 充当换道准入（否：现行无门 hop 直接放行）。
- **影响面**：P3 组合请求形状与 P4 读写集；Conflict bundle 复用面；固定步进
  确定性与首错顺序；S4/S6 验收。
- **依据**：`traffic-runtime-phase-protocol.md` §3/§3.1；route-design §2.3
  子合同 6/7。
- **决策请求**：确认 P3/P4/P5/P7 四阶段接入为冻结落点。

### D4. 快照形状（含版本号建议值，定号 = G1 决策时）

- **建议值**：
  - 车辆记录新增 tagged 过渡状态 `PreEntry | InTransition | Clearing` +
    reservation wire row（acquired tick、源/目标/过渡区边 `StableId128` +
    整数毫米区间并集、route occurrence 证明）；完整 reservation 只装运行时
    权威（仿 Conflict 行），车辆不持副本；
  - 机动/门/等待/冲突出现项照旧**不入档**，恢复经目标修订重编译
    （ADR 0029 §6）；
  - 确定性状态摘要前像扩展过渡记录段；恢复对称原则照旧（稳定引用 + 快照
    局部标识 + 整数毫米，无进程句柄/槽位/generation），恢复不重复 grant/
    事件，零部分恢复。
  - **版本号建议值（不提前钉版，G1 定号）**：`runtime_state_version` 5→6、
    摘要版本递增；未知版本失败关闭照旧。
- **备选**：过渡状态并入既有 Conflict 行形状（复用 tagged 模式但不得改变
  既有 kind 语义——ADR 0029 §3 的 `StaticRoute`/kind 复用教训）；独立
  tableKind（需 Identity registry 连续登记与实体/字段标签分配）。
- **影响面**：LFRS schema 与版本轴（`traffic-runtime-snapshot.md` §2/§4）；
  确定性状态摘要前像（§6）；恢复等价/回放身份（§6）；occurrence/容量语义
  （§2 两类容量 + 过渡预约是否计数）；跨修订迁移入口（§7）。
- **依据**：`traffic-runtime-snapshot.md` §2/§3/§6；ADR 0029 §6；
  ADR 0033（摘要角色与算法选型权威）。
- **决策请求**：确认形状方向与备选取舍；**版本号随本批 G1 定号**（与 D5
  若采纳建议值时的 WorldConfig 扩项同批处理，见下）；过渡预约的容量计数
  口径一并裁定。

### D5. 执行能力检查位置（⑦ 的定位部分；2026-10-30 统筹裁决后仍为 G1 决策项）

- **背景（边界冲突核查记录）**：`traffic-runtime-execution-config.md` §1
  冻结 `ExecutionConfig` = **显式非零 worker 数**，不进入快照/摘要/静态路网/
  交通身份；过渡执行能力属「影响准入或行为」，落在 `WorldConfig` 定义域。
- **三选项**：
  - **选项 1（建议值）：能力声明放 `WorldConfig`（交通配置）**——与「影响
    准入或行为」定义严格一致；进快照与摘要；WorldConfig 形状扩项与版本号
    **随 D4 同批 G1 定号**；
  - **选项 3（备选）：不引入配置字段**——能力为构建期/实现特性：
    `register_route` 出现项编译期按「出现项含过渡 ∧ 当前运行时不支持」
    结构化拒绝 + spawn 绑定期复核；配置与快照零改动（最小格式面）；
  - **选项 2（排除）：扩义 `ExecutionConfig`**——违反 #302/#303 冻结边界
    （把行为能力塞进 worker-only 且不进交通身份的类别），排除。
- **排除方案及依据**：选项 2 排除，依据 = `traffic-runtime-execution-config.md`
  §1 配置职责表（Accepted）与 `traffic-runtime-snapshot.md` §2「执行配置设计
  将 worker 移出快照」；静默挪边界被统筹裁决明令禁止。
- **影响面（建议值口径）**：`WorldConfig` 形状与 `WorldConfigBinding`（LFRS
  vtable 槽位/封闭字段数）；快照摘要前像的行为语义配置段；恢复/切换链的
  能力校验位置（`traffic-runtime-execution-config.md` §4.1/§4.2 恢复链）；
  **显式交叉引用 #302/#303 冻结的配置边界，不改其语义**。
- **共同约束（任一选项均适用）**：检查为双层（`register_route` 出现项编译期
  + spawn 绑定期复核）；拒绝语义见 §2 S3；全部入口统一（direct / candidate /
  cutover / restore / replay，参照 `route_edge_occurrence_capacity` 统一预算
  先例）。
- **验收归属（PR #1178 评审修正）**：S3 只做独立静态序列验证 +「执行不支持
  时注册被拒」断言；含过渡路线的注册成功与注册表原子性验收在 **S4**（能力
  落地后）——静态验收不得依赖运行时能力准入，消除 S3→S4→S3 验收依赖环。
- **决策请求**：在选项 1 / 选项 3 中裁定；若裁定选项 1，WorldConfig 扩项的
  版本号与 D4 同批定号。
- **G1 裁定结果（#1199 S4 对齐注）**：**裁定选项 3（实现属性）**——能力为
  构建期/实现特性，不引入配置字段（WorldConfig/ExecutionConfig 零扩义、
  快照/摘要零改动 ✓ 最小格式面）。S4 切片已执行：`transition_execution_
  supported()` 实现属性翻转为 `true`（预约路径全绿后经裁定授权；含过渡
  路线由注册拒绝改为可注册可执行——PR 行为变化清单 BREAKING 条）。共同
  约束双层检查中的注册面结构化拒绝在翻转前由 S3 六例反证承载、翻转后按
  授权语义演化迁移为正断言；spawn 绑定期复核语义并入 R 验收面（R1–R3
  四钉版全绿）。

## 2. 可自决项（已定案 + 依据引用）

### S1. #273 排除条件（⑤）

**已定案**：#237 首版在以下四条件下不需要 #273（共享 stationing/横向几何）：
① 过渡以真实边物化且完全落在显式 section 边界内（整边界锚点）；② 位置表达
只用 edge-local 整数毫米 + route occurrence（禁百分比/最近点投影）；③ 占用
投影采用整边保守公式（D2），不做跨 lane 位置映射；④ 不要求段级边界 identity。
**触发登记 #273 为真实依赖**（任一即登记）：任意纵向位置过渡；部分区间投影
需要共享 stationing；出现长度百分比/跨 lane 进度换算诉求；段级边界锚点成为
必要输入。
依据：`cross-section-access.md` §3.2.1/§14（整边界锚点、段级推迟）；#273
非目标（明文不阻塞整边界/预制过渡首版）；统筹裁决 1。

### S2. 跨层合同落地条件（⑥）

**已定案**：route-design §3.4 六项共同输入转正式契约文本，逐项引用权威来源
（边链身份=逐修订身份 + 文档化映射，不发明跨修订稳定 ID；许可依据引
`cross-section-access.md` §5/§6 AccessRule 权威；控制锚点对齐现行绑定语义；
修订/制品绑定锚定 ADR 0025/0033）；类型化 schema 随 S1/S3 实现切片落地；
凡现行模型不提供的保证逐条记入 §4 缺口登记，不静默假设。
依据：route-design §3.4（统筹复核附条件通过，2026-10-29）；#806 裁决补修
（`d9ae03ae`）。

### S3. 执行能力检查拒绝语义（⑦ 的语义部分）

**已定案**：结构化错误类别（如 `TransitionUnsupported`）、失败关闭、零部分
状态（注册表不变 / 恢复零部分丢弃 staging）、不 warn-and-continue；检查
双层（register_route 出现项编译期 + spawn 绑定期复核）；全部入口统一。
依据：`route-system.md` D4/D5（注册校验与失败原子）；`traffic-runtime-
snapshot.md` §5 恢复链「执行能力校验……任一失败只丢弃 staging」先例；
统筹裁决 3 的拒绝语义部分。

### S4. T01 来源保真规则（① 的可自决部分）

**已定案**：T01 可为经授权规则派生的编制材料，但必须保存来源映射与构造依据，
**不得冒充原 SUMO connection**；合成来源为原生声明；身份按逐修订身份 +
文档化映射登记。依据：#806 裁决口径（2026-10-29）；ADR 0029 §3 连续登记
（`sourceLanguage`/来源关系现行登记值，不复用旧语义）。

## 3. 证据引用

- **研究目录**：
  - `research/issue-1142-route-design/`：边界设计（design.md §2/§3）、切片
    计划与验收矩阵（slice-plan.md）、最小闭环样本（sample/，v2 钉版 13 边 /
    11 场景 / 三拓扑 / actor 构型）；
  - `research/issue-1053-lust-population-v2/README.md`：人口 v2 基线
    9,344/10,000 不可展开与旧选取口径 9,350 对照（数值口径权威）；
  - `research/issue-1085-route-diagnostics/`（A1 诊断）与 A0/A1 报告
    （`target/issue-790-806-handoff-20261026/kimi-806-a0/`、`kimi-806-a1/`，
    交接材料）：真实反例、连续对照、六类诊断、A1 几何修复失败原文。
- **主线合同文档**：`route-system.md` D1–D8；`cross-section-access.md`
  §3.2/§3.2.1/§5/§6/§14；`road-junction-model.md` §3.2/§3.3/§7/§17.4；
  `vehicle-following.md` §6.4/§7；`real-road-workloads.md` §3.1；
  `traffic-runtime-phase-protocol.md` §3/§3.1；`traffic-runtime-snapshot.md`
  §2/§3/§5/§6；`traffic-runtime-execution-config.md` §1/§4/§6。
- **ADR**：0025（受检规范路网与共享静态路网）、0029（预编译静态路线退役，
  §2/§3/§6）、0033（摘要角色与摘要算法选型权威）。
- **Issue/PR**：#806（方向裁决评论，2026-10-29）、#237（本章为其 G1 输入）、
  #273、#1142（F 路设计）、#1144 / #1151（route-design 合入）、#1177（本章
  任务书）。
- **外置制品**：人口基线等大体量证据按协调章程 v2 挂 GitHub Release
  （`research-evidence-*` + MANIFEST 哈希钉版），仓库只留计划、入口与摘要。

## 4. 缺口登记（生产 G1 缺口清单并入项）

| 编号 | 缺口（现行模型不提供的保证）                                                                         | 处置                                                                                     |
| ---- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| G-A  | 跨修订稳定 ID 语义不提供                                                                             | 合同按逐修订身份 + 文档化映射写（S2 已定案）；不发明语义                                 |
| G-B  | 部分区间占用投影需共享 stationing，不提供                                                            | v1 整边保守投影规避（D2）；#273 触发条件登记（S1）                                       |
| G-C  | merge/diverge 的 gap/priority/lane availability profile 缺（`road-junction-model.md` §17.4 独立 G1） | 备选固定连接区（D1 备选 (a)）启用前必须补齐并独立 G1                                     |
| G-D  | 若 D5 裁定选项 1：`WorldConfig` 形状扩项与版本轴联动                                                 | 与 D4 版本号**同批 G1 定号**；LFRS `WorldConfigBinding` 封闭字段数变化随 schema 钉版流程 |
| G-E  | 过渡状态的跨修订迁移入口细则未定                                                                     | D4 形状定案后按 `traffic-runtime-snapshot.md` §7 补登记                                  |
| G-F  | 诊断分类（`lane-continuity-gap` 等）升格稳定机读输出未定                                             | route-design §6.1 第 3 项，另行评审；不属本章裁决                                        |

以上各项均不得在设计或实现中静默假设；新缺口随实现切片继续登记，由统筹在
收口批汇总（协调章程 §7）。
