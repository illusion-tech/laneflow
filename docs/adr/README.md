# 架构决策记录

本目录用于记录 LaneFlow 的架构决策记录。

ADR 关注“为什么这样定”，不替代详细设计文档。涉及高影响、难回退、会影响多个模块或长期兼容性的决策，应优先进入 ADR。

## 适用范围

优先写 ADR 的议题：

- LaneFlow Core 与 Engine Adapter 的边界。
- Core 不依赖具体游戏引擎或外部重型交通仿真器。
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

- `0001`: 项目定位与范围边界
- `0002`: 依赖与许可证约束（不依赖 SUMO / CARLA / libsumo）
- `0003`: Runtime tick 与确定性策略
- `0004`: Core 实现语言（Rust）
- `0005`: Core identity、handle 与 lifecycle 模型
- `0006`: Vehicle Following 控制、安全与扩展边界
- `0007`: Traffic Data crate、loader 与 Core normalization 边界
- `0008`: 1.0 前单一当前数据格式与迁移兼容策略
- `0009`: Signal indication、MovementGate/StopLine、法规策略与 Core safety 分层
- `0010`: Parking binding、vehicle lifecycle/position authority 与 Core/Adapter 分层
- `0011`: JSON Schema canonical URI、公共发布、不可变版本与长期保留契约（已被 0026 取代；只保留历史说明）
- `0012`: Core 数值权威、累计精度与局部表现转换边界（已被 0014 取代；继续描述迁移前当前生产实现）
- `0013`: 引擎无关的空间几何、长度权威、制品配对与适配器位姿边界（精度条款已被 0015 取代）
- `0014`: 补偿残差感知的 f32 Core 数值权威、产品范围与生产迁移门槛（Accepted；Spatial 精度条款已被 Accepted ADR 0015 部分取代；生产一维几何实施已被 Accepted ADR 0028 部分取代）
- `0015`: 有界局部 canonical frame、运行时 f32 空间类型与性能边界（已接受）
- `0016`: 场景人口、确定性出口回流、Core 原子 replace 与 Adapter binding 生命周期权威（已接受）
- `0017`: 静态 Junction/Movement/ManeuverPath owner、ManeuverGate identity、Route occurrence 与复杂设施演进边界（已接受）
- `0018`: 多模式横断面 owner（RoadCorridor/RoadSection/FacilityBand）、FacilityKind/ParticipantClass/AccessRule 分层与准入 overlay（已接受）
- `0019`: WaitingZone/ConflictZone identity、多阶段 Gate occurrence、车辆级 right-of-way authority 与 grant/reservation（已接受）
- `0020`: 权威来源模块图（Authoritative Source Module Graph）、编译器拥有的静态路网、
  编译器中间表示（Compiler IR）、完整标识登记表（Identity Registry）、可移植规范
  制品（Portable Canonical Artifact）、历史目标静态镜像方案，以及目标态
  交通运行时（Traffic Runtime）边界（Accepted；#291 G1；独立 validator、规范发布
  receipt 与 #299 统一收据职责已由 Accepted ADR 0024 部分取代；镜像文件/ABI 已由
  Accepted ADR 0025 继续取代）
- `0021`: 面向中国特色城市模拟游戏的交通基础产品北极星、城市游戏/出行编排/
  路径规划/交通运行时权威分层、不可变路网修订、修订切换事务、运行时快照与确定性
  降级边界（Accepted；#291 G1）
- `0022`: Geometry 编制解析曲线、编译期参考折线、规范 `f32` 运行时折线与 Adapter
  表现几何的误差预算分层；冻结 `Fine2Cm`、`Balanced5Cm`、`Compact10Cm` 三个封闭
  B1 工程目标档与独立方向连续性约束，不声明连续最大误差保证或长期存档兼容；旧 JSON
  决策已纠偏，FlatBuffers B1 契约已重新接受（Accepted；#296 FlatBuffers G1）
- `0023`: 道路编辑状态作为生产编制权威、可视化编辑器与程序化生成器共享有类型道路
  编辑模型，以及道路修改按整体替换 A 阶段演进到候选调整/原子替换 C 阶段；产品负责人
  已选择按模块 size-prefixed FlatBuffers 作为 production source 编码，并冻结 A → C 演进、
  未发布兼容边界和后继差异职责（Accepted；#296 FlatBuffers G1；§2.1 的 22 向量来源
  形状已被 Accepted ADR 0029 部分取代）
- `0024`: compiler 对最终 LFCA/LFSM/LFSD 不可变、可重读对象来源执行共享、无分配的
  后发射检查，以受检能力守卫最小发布闭合；不交付独立 validator/receipt，LFCP v2
  一次性移除 receipt 且不兼容读取 v1（Review；既有后发射决定已实现，LFCA 4
  staged-source 容量修订待接受）
- `0025`: 从受检 LFCA 构建性能优先、不可变、进程内共享的
  `SharedNetworkRevision`；Traffic/Identity/Spatial 物理拆分并绑定同一修订，不交付
  静态镜像文件/ABI、descriptor/完整性清单、mmap 或磁盘缓存，保存只接受已进入
  Runtime 的 committed 道路状态或已认证 LFCA asset reference；可编辑 session 在共享根外
  保留 exact LFCA diff base，但不把它写入存档（Review；LFCA 4 百万级分块输入修订待接受）
- `0026`: 推倒 G3/G4 自然语言门禁，改为原生 PullRequestReview Check、Merge Queue
  盖章与收窄的 commit 校验；退役 Schema Publication / ADR 0011 公共发布义务；
  External Review、六项 required checks 与相关启用顺序已被 0027 取代，其它决策继续
  有效（Accepted；#468）
- `0027`: 退役非 required 的 External Review 自定义 Check，收敛为五项机器门禁、
  Merge Queue 与 GitHub 原生未解决对话阻断；required approvals 不在本次启用，owner
  bypass 转入 #493 独立治理（Accepted；#492）
- `0028`: 交通运行时已提交一维几何改为整数毫米 + 微米余数，固定步进
  `4..=1000` ms，已提交速度 `u32` mm/s；有折线边长由规范折线弧长派生并写回 IR；
  准入后 Typed AST / HIR / MIR / LIR 交通一维与制品同一套整数毫米（#500）；
  已部分取代 ADR 0014 的 current-`f64` 生产实施与残差 `f32` 目标合同
  （Review；整数毫米决定保持，LFCA 4 合同轴修订待接受）
- `0029`: 路网产品删除预编译 `StaticRoute`；`TrafficWorld` 路线入口只留
  `register_route`；Identity 种类 21 / 字段标签 30 / 关系角色 13–16 保留空位；
  对象 `formatVersion = 4`；走廊示例边序列改由 catalog 0.3 拥有
  （Review；路线退役决定保持，统一 LFCA 4 登记修订待接受）

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
