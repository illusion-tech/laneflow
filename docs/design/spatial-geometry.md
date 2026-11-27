+++
schema_version = 1
id = "design-spatial-geometry"
kind = "design"
status = "accepted"
revised_date = "2026-11-27"
summary = "定义有界空间几何、长度绑定与位姿采样。"
scope = "引擎无关的规范坐标框架、框架编制与修订绑定、折线中心线、长度绑定、采样、局部位姿与制品配对"
applicability = "#applicability"
issues = ["https://github.com/illusion-tech/laneflow/issues/300", "https://github.com/illusion-tech/laneflow/issues/301", "https://github.com/illusion-tech/laneflow/issues/1569"]
related = ["adr-0012", "adr-0013", "adr-0014", "adr-0015", "adr-0020", "adr-0022", "adr-0025", "design-adapter-api", "design-numeric-representation", "design-parking-system", "design-road-editing-source-and-geometry-frontend", "design-signalized-corridor-protected-turning", "design-traffic-runtime-shared-consumption", "reference-glossary"]
+++

# 空间几何设计

<a id="applicability"></a>
## 适用边界与依据

共享空间路网采用现行共享修订合同。一维交通进度以整数毫米 `progress_mm` 表示。空间位姿来自有界局部 `f32` 中心线，其可分辨间距随局部坐标量级变化。转换到宿主坐标后还取决于宿主原点和数值类型；提高求值中间精度也不能恢复来源已量化的坐标。

关联文档：

- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md`
- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0025-checked-canonical-network-and-shared-static-network.md`
- `../reference/glossary.md`
- `numeric-representation.md`
- `adapter-api.md`
- `parking-system.md`
- `signalized-corridor-protected-turning.md`

## 1. 目标、边界与术语

本文定义如何把 Traffic Runtime 的“边句柄 + 边内进度”映射为引擎无关的位姿，同时保持 Traffic Runtime、Spatial 与 Adapter 的最终裁决职责分离。

本文使用以下术语：

- **最终裁决职责（authority）**：某项状态由哪一层定义并最终裁决。
- **标准坐标（canonical）**：LaneFlow 在一个有界局部 frame 内统一使用、尚未转换为宿主坐标的空间。
- **宿主坐标（host coordinates）**：Adapter 把 canonical frame 放置到具体引擎场景后使用的坐标。
- **位姿（pose）**：位置、切向量和上方向向量的组合。
- **制品（artifact）**：编译器发射的受检 LFCA。共享路网修订是由该制品建成的进程内不可变根对象，不是另一份可发布制品。
- **层级名称**：Traffic Runtime、Spatial、Adapter 分别表示交通运行时层、空间层和适配层。
- **应用程序接口（API）**：供其他包或宿主调用的稳定程序边界。

| 层级            | 最终裁决职责                                     | 不负责                             |
| --------------- | ------------------------------------------------ | ---------------------------------- |
| Traffic Runtime | 拓扑、边长、路线区段、边上进度、交通状态         | 标准或世界坐标、中心线、宿主变换   |
| Spatial         | 标准坐标框架、中心线、弧长、绑定、采样和位姿     | 交通规则、网格与材质、引擎生命周期 |
| Adapter         | 快照调度、frame 放置、宿主朝向基与变换、表现提交 | 重算交通进度、覆盖几何或长度正本   |

`TrafficWorld` 可以在没有 Spatial 时以无图形宿主方式运行；需要车辆空间位姿的调用方必须绑定共享路网修订并取得 `SpatialSession`。

### 1.1 #291/#300 共享空间路网（已交付）

ADR 0020 保留本章的 Spatial authority、bounded canonical f32、length/pose、
batch ordering、placement token 与失败原子性，但改变静态数据形成位置：

- compiler 在同一 MIR/LIR 中联合生成 Traffic length、canonical geometry 和绑定；
- `SharedNetworkRevision` 的 Traffic、`SharedIdentityIndex` 与
  `PartitionPlanningHints` component 必选，`SharedSpatialNetwork` 可选；headless
  构建不保留 geometry；
- Spatial component 可以只有 facility/profile/frame；只有 LaneEdge geometry 非空时才
  与 `SharedTrafficNetwork` 共享 logical edge ordinal/cross-table index，并保持完整 edge
  coverage。缺少 lane geometry 时 lane-pose capability 明确不可用；
- Spatial 直接读取修订上的不可变 geometry/sampling tables，按边序号采样；
- 多个 world/Adapter session 可共享同一根 `Arc<SharedNetworkRevision>`，只保留各自 pose
  scratch/output 和 placement lifecycle；Spatial 不独立安装 component。

生产入口是 `SpatialSession::bind(Arc<SharedNetworkRevision>)`，车道进度为
`progress_mm`。无 Spatial 的 headless Traffic Runtime 仍合法。边长由共享静态路网承载，进度与
交通状态由 `TrafficWorld` 承载。

## 2. 坐标框架

### 2.1 标准坐标契约

| 属性         | v0.6 契约                                           |
| ------------ | --------------------------------------------------- |
| 标量类型     | LaneFlow 自有 `f32`                                 |
| 点范围       | 每轴闭区间 `[-16_384 m, 16_384 m]`                  |
| 线性单位     | 米                                                  |
| 角度单位     | 弧度                                                |
| 坐标系手性   | 右手                                                |
| 上方向轴     | `+Y`                                                |
| 水平面       | `X/Z`                                               |
| 全局前方向   | 无；边的前方向由有向中心线切向量定义                |
| 坐标框架身份 | 编译产生的 `CanonicalFrameId`；修订内按框架序号访问 |
| 地理配准/CRS | 首版不表达；frame 放置由 Adapter/场景边界单独管理   |

规范坐标框架表示有界局部空间。宿主负责将其放置到场景中。

### 2.2 Frame 与宿主放置

每个 canonical frame 已经是有界局部空间，不再要求 Spatial 同时维护绝对 `f64` 坐标与 local `f32` 副本。转换顺序固定为：

```text
Traffic Runtime 的有效进度
  -> Spatial 的受检 canonical f32 位姿
  -> 校验 frame、有限性、点范围和朝向基
  -> Adapter 解析 frame 到宿主场景的放置
  -> 宿主专用朝向基和变换（Transform）
```

frame 放置、tile 或相机相对原点不写入 Traffic Runtime 车辆状态，也不改变 Spatial canonical 几何。#136 使用 batch-level `FramePlacementToken` 标记某个 frame 的宿主放置版本；token 只由 Adapter 颁发和复核，不包含世界坐标或 origin value。同一 frame 的放置发生变化时必须换 token，旧批次不得提交。

<a id="canonical-frame-identity"></a>
### 2.3 框架的编制键、稳定身份与修订内序号

| 使用位置                                | 表示                                         | 解释范围                                                                                                                                                                 |
| --------------------------------------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 走廊与路口生成器的 TOML 输入 `frame_id` | 框架局部键，区分大小写的文本                 | 在声明所在的编制模块中解析。同一命名空间内框架键唯一，两个不同命名空间可以使用同一个键。                                                                                 |
| 编译器与共享身份索引                    | `laneflow_static_contract::CanonicalFrameId` | Identity v1 以 `EntityKind::CanonicalFrame` 和依次编码的 `AuthoringNamespaceId`、`CanonicalFrameKey` 派生有类型的 128 位稳定身份。命名空间和键都相同的框架可跨修订对应。 |
| Spatial 位姿批次                        | `Option<CanonicalFrameOrdinal>`              | 在批头的 `NetworkRevisionId` 所标识的绑定修订中解释。非空成功批次为 `Some(ordinal)`，空成功批次为 `None`，两者均回显修订身份与放置令牌。                                 |

宿主收到非空批次，先核对批头修订与持有的共享修订，再用
`revision.identity().stable_id(ordinal)` 取得框架稳定身份。重绑后如需寻找同一框架，
用新修订的 `identity().ordinal(id)` 查找并处理缺项。新旧序号即使数值相同也要分别解析。
同一批次遇到第二种框架时整批拒绝，保留原输出的记录、修订、框架和放置令牌。

### 2.4 生成器的框架局部键

两个生成器沿用 TOML 字段名 `frame_id` 及原始字节。配置接受 **1–53 个 ASCII 字节**，
首字节为字母或数字，后续仅允许字母、数字及 `._:/-`，相邻双冒号 `::` 为保留分隔符。
例如 `Frame-1.z:/X`、`a:` 和 `7` 均合法。LFRE 在导入引用中用
`namespace::key` 指明目标模块；这两个输入字段只接受声明本身的局部键。
53 字节取自 Identity v1 的 ASCII 字段硬上限，也是 LFRE 单个引用组件的上限。
语法依据见[道路编辑来源设计][road-editing-source]。

- **走廊**在 `laneflow/signalized-corridor` 的合成模块中声明框架，将车道边引用和对应折线直接挂在声明下。配置摘要包含 `frame_id` 原始字节。
- **路口**在 `laneflow/complex-junction` 声明一次框架，道路参考线、普通冲突区与汇入冲突区以同一局部键引用它。生成网格时这些声明和引用仍在同一个模块中。配置序列化参与来源摘要。

`laneflow_compiler::road_editing` 提供专用公开函数
`validate_canonical_frame_local_key(key: &str) -> Result<(), DiagnosticBundle>`。
函数借用原始文本，复用 LFRE 现有的 token 校验、单组件上限和保留分隔符检查。
走廊与路口的配置 `validate` 都调用它，因此 TOML 解析和直接调用
`generate(&config)` 都在构建几何或写文件前发现错误。

53 字节的提前检查仅用于上述专用入口。`CanonicalFrameInput::try_new` 与
其余 23 类声明构造器、`CanonicalFrameReference::local` 均沿用各自的现行校验时机。
例如 54 个 ASCII 字母可以构造框架声明和局部引用；构造成功只表示通过该层校验。
builder 仍按编译配置检查资源限制，LFRE 预检仍独立检查外部字节。这样可以直接验证
builder 拒绝超限声明后仍可复用。普通合成模块的通用键规则及 Identity v1 编码沿用现行契约。

两个生成器将专用函数的错误映射为各自的 `Error::Config(String)`，消息标明 TOML
字段 `frame_id`，并附上 `DiagnosticBundle` 的显示文本。允许直接显示内部路径
`canonicalFrame.canonicalFrameKey`。调用方可据配置字段、违规原因以及适用的字节下标
或长度与上限定位问题；前缀、标点和内部路径的显示拼写均不作为配置 API 验收条件。

Spatial 接收已绑定修订中的采样地址，两个生成器直接依赖编译器完成配置校验。
宿主从 `laneflow_static_contract::CanonicalFrameId` 引入稳定身份。Spatial 根不重导出该类型。
城市生成器输出目录中的 `frame_id` 已表示稳定身份的十六进制文本，沿用该输出语义。

### 2.5 实施验收

1. 对两个生成器分别经 TOML `parse` 和直接构造配置后 `generate` 验证同一组键。
   仓内样例、混合大小写、尾单冒号以及 53 字节键应生成成功。空串、非法首字节、
   非 ASCII、非法后续字节、54/128/129 字节和 `a::b` 应在配置层报 `frame_id`；
   直接调用 `validate_canonical_frame_local_key` 应接受 53 字节，对 54 字节和 `a::b`
   分别从诊断载荷中取得 `TooLong { limit: 53, observed: 54 }` 和
   `ReservedDelimiter { byte_index: 1 }`（首个冒号，从 0 起）。两个生成器均断言
   `Error::Config`、字段 `frame_id`、对应原因和参数。另验 54 个 ASCII 字母仍能构造
   LFRE 框架声明及局部引用，并保留 `add_checks_string_limits_before_mutation`
   对 builder 拒绝后可复用的覆盖。
   迁移前 54 与 128 字节通过配置、稍后生成失败；`a::b` 在走廊确能出产制品，
   在路口则于框架声明失败。统一局部键规则有意收紧了这一走廊输入。
2. 用仓内走廊和路口各自的原版 TOML，在迁移前后分别生成。
   通过实际修订的身份索引读取框架稳定身份，比较 `NetworkRevisionId`，
   并逐字节比较两次生成的 LFCA 和场景目录文件（`.catalog.toml`）。另在两个不同编制命名空间声明
   同一键，验证各自的局部引用解析到自己的框架，且稳定身份不同。
3. 通过 Spatial 公开入口检查非空批次的序号与稳定身份往返、成功空批的批头，
   以及混框架失败后原批头和记录完整保留。以外部调用方验证从静态契约根导入
   `CanonicalFrameId` 成功，从 Spatial 根导入同名类型报未解析导入。

## 3. 制品模型

几何在可选的 `SharedSpatialNetwork` 上。生产入口是
`SpatialSession::bind(Arc<SharedNetworkRevision>)`。无 Spatial 组件时 `bind` 返回
`Ok(None)`。绑定失败不留下可观察的半个 session。批量采样先写满再提交，失败保持旧输出，
见 [Adapter API §6][adapter-api-batch]。

## 4. LaneFlow 自有空间类型

生产 crate 为 `crates/laneflow-spatial`（package `laneflow-spatial`、lib
`laneflow_spatial`），生产依赖为
`laneflow-spatial -> laneflow-static-network + laneflow-static-contract + laneflow-format`。
中立的 `laneflow-static-contract` 提供 Spatial、编译器和 Runtime 共同解释的数值
边界、稳定身份与有类型的序号。Spatial 与 Runtime 互不依赖（`xtask` 运行时架构检查）。

公共几何类型包括 `CanonicalPoint3F32`、
`CanonicalVector3F32`、`CanonicalUnitVector3F32`、`CanonicalPoseF32`、
`FramePlacementToken`、`PoseSource`、`PoseInput`、`SpatialError` 与 `SpatialAxis`。
位姿输入与批次形状见 [Adapter API §4][adapter-api-pose]。

其中：

- 点与向量是不同类型，不能用一个裸三元组同时表达二者。
- 点与向量共享批头的框架。宿主通过绑定修订解释框架序号，见[框架身份][frame-identity]与 [Adapter API §4][adapter-api-pose]。
- 点、向量和单位方向的字段私有，只能通过受检构造器和返回 `Result` 的命名运算进入有效状态；公共表面不实现可绕过有限性/范围不变量的裸 `Add`、`Sub`、`Mul` 或 `Div`。
- 公共字段不暴露 `euclid`、`glam`、`nalgebra`、Bevy 或其他第三方类型。
- 所有构造器在进入正本状态前拒绝非有限值，并把带符号零规范化为正零；点每轴还必须位于 `±16_384 m`，返回点的运算重新校验范围。
- 向量不套用点范围；两个合法端点的差可以达到 `±32_768 m`。
- `CanonicalUnitVector3F32` 只能从有限非零向量通过缩放归一化产生；近垂直道路的 projected-up 阈值仍由折线/basis 切片拥有，不是通用向量 epsilon。
- `CanonicalPoseF32` 的字段私有，getter 只返回已经通过 canonical 点、单位切向量与单位上方向不变量的值。
- frame 与 placement token 只在批次头保存一次，不逐点或逐车辆复制。失败原子性见 [Adapter API §6][adapter-api-batch]。不同坐标框架或不同宿主放置不能在没有检查的情况下混用。编制 `f64` 不形成第二份 runtime 几何正本。

## 5. 折线与预计算

每条交通边对应一条有向三维折线：

```text
P0 -> P1 -> ... -> Pn
```

要求：

- 至少包含两个有限且每轴位于 canonical frame 范围内的 `f32` 点。
- 每个相邻线段长度严格大于 `SPATIAL_MIN_SEGMENT_LENGTH_METERS`。
- 顶点顺序与交通边的行驶方向一致。
- 每个线段中，标准 `+Y` 投影得到的上方向基必须可以归一化；首版不接受近垂直道路。
- 加载器计算线段长度、归一化切向量与上方向向量，以及累计弧长；这些派生值不从线格式接收。
- 折线拐角允许切向量不连续，不做隐式平滑或样条曲线拟合。

#135 冻结并实现以下常量：

| 名称                                     |              值 | 单位或语义              | 依据                                       |
| ---------------------------------------- | --------------: | ----------------------- | ------------------------------------------ |
| `SPATIAL_MIN_SEGMENT_LENGTH_METERS`      |           `0.1` | `10 cm` 输入有效尺寸    | 拒绝量化重合点与无展示价值的短噪声线段     |
| `SPATIAL_LENGTH_ABS_TOLERANCE_METERS`    |          `0.01` | 长度绑定绝对容差下限    | 与最终 `1 cm` 展示误差预算一致             |
| `SPATIAL_LENGTH_REL_TOLERANCE`           |          `1e-6` | 长度绑定相对容差项      | 长边上的生产工具与累计误差余量             |
| `SPATIAL_JOIN_POSITION_TOLERANCE_METERS` |         `0.005` | 相连端点间隙            | `5 mm`，保留在最终 `1 cm` 位置误差预算之内 |
| `SPATIAL_MIN_PROJECTED_UP_LENGTH`        | `0.008_726_535` | `sin(0.5°)` 的 `f32` 值 | 等号有效；更小值拒绝近垂直 basis           |

这些值应用于受检转换后的 runtime `f32` 坐标。若相邻输入点量化后重合，直接返回退化线段错误，不静默合并。basis 阈值按 `projected_up_length < SPATIAL_MIN_PROJECTED_UP_LENGTH` 拒绝，不能复制旧 `f64` epsilon。交通边长是独立的整数毫米正本，绑定容差不另加量化余量。

这里的最终 `1 cm` 是调用方已经提供同一条高保真参考折线后，`f64` 参考到 runtime
`f32` 表示的误差预算。ADR 0022（#296 FlatBuffers G1 已接受）为上游解析曲线细分提出
`Fine2Cm`、`Balanced5Cm`、`Compact10Cm` 三个封闭几何精度配置档，并以
`Smooth1Deg`、`Balanced2Deg`、`Compact5Deg` 三个封闭几何方向配置档正交选择；该决策不修改
本节任何 production Spatial 常量，也不把 Adapter 展示或车辆物理偏差计入静态几何
预算；其 B1 厘米值是内部验证的工程目标，不是连续曲线最大误差证明。候选状态不修改
本节任何 production Spatial 常量。

## 6. 长度正本与绑定

### 6.1 最终裁决职责

- `Traffic EdgeLength`：交通进度、路线距离，以及跟驰、信号和停车约束的正本。
- `Spatial geometry arc length`：标准中心线参数化与位姿的空间正本。
- 创作或导出工具：应从同一中心线来源生成两者，但运行时仍必须验证，不能无条件信任生产者。

### 6.2 一致性检查

```text
difference = abs(traffic_edge_length - geometry_arc_length)
tolerance = max(
  SPATIAL_LENGTH_ABS_TOLERANCE_METERS,
  SPATIAL_LENGTH_REL_TOLERANCE * max(abs(traffic_edge_length), abs(geometry_arc_length))
)

difference <= tolerance -> 绑定成功
difference > tolerance  -> 阻断错误
```

现行交通正本边长是整数毫米；对账时把 `length_mm / 1000` 换成米再与规范 `f32` 弧长比较，不另加量化余量。ADR 0014 的残差 `f32 EdgeLength` 不是现行合同。#144 曾在生产候选中启用，但形成性能不迁移（no-go）结论后已回退。

禁止静默替换、只给警告、按引擎样条曲线重算，或在适配器端修复。也不能因为旧几何容差小于 `f32` 量化误差就把交通边长恢复为 `f64`；两类误差必须分别记录并组合。

### 6.3 进度映射

本文中的 `normalized_traffic_length` 是交通 `EdgeLength` 经过领域构造、校验和规范化后的正本边长。Spatial 只消费 Traffic Runtime 提供的有效观察值；它不是原始 Data 输入，也不是几何弧长。Traffic Runtime 的补偿进度可以继续以 `f64` 有效值观察，但 Spatial 顶点和输出位姿保持唯一的 `f32` runtime 正本。

Traffic Runtime 已提交进度为整数毫米。Spatial 只消费换算后的有效米制观察值，不复制运行时边界规则。#135 精确要求 `0 <= progress <= normalized_traffic_length`：零值直接返回首点，等于正本长度时直接返回末点，严格越界返回结构化错误。Runtime 若先产生已经吸附的有效进度，Spatial 仍只观察结果，不拥有或复制该容差。

```text
ratio = effective_traffic_progress / normalized_traffic_length
geometry_s = ratio * geometry_arc_length
```

- 进度必须来自经过验证的 Traffic Runtime 有效状态或快照；Spatial 不读取高位/残差分量。
- Spatial 不自行吸附或截断；输入严格越界即返回错误。
- 比例映射确保交通终点精确命中几何终点。
- 容差内的比例差仍必须小于验证确定的位置误差预算。

## 7. 采样语义

`sample(edge, progress)` 返回“位置 + 切向量 + 上方向向量”：

- 位置：在线段内按累计弧长线性插值。
- 切向量：归一化的线段方向。
- 内部顶点：精确命中时使用出段，形成确定的右连续规则。
- 最终端点：使用最后一个入段。
- 上方向向量：标准 `+Y` 在切向量正交平面上的归一化投影。
- 左方向：解析停车横向偏移时使用 `up × tangent`；正横向偏移继续表示沿行驶方向左侧。
- 正朝向偏移：保持停车契约，从上方向下看时表示逻辑道路坐标中的逆时针旋转。
- 首版不输出四元数或矩阵，不表达道路倾斜；适配器使用切向量和上方向向量构造宿主朝向。

相连交通边 `A -> B` 的 `A.end` 与 `B.start` 必须在连接容差内。位置连续是阻断性不变量；切向量可以因路口或折点而不连续。

## 8. 错误模型

实现必须用 LaneFlow 自有的结构化错误区分：

- 未知、重复或缺失的交通边绑定；
- 坐标框架不匹配；
- 点数量不足或坐标非有限；
- 退化线段或退化朝向基；
- 交通边长与几何长度不一致；
- 相连端点不连续；
- 进度或弧长越界；
- canonical frame、点范围、宿主放置或 `f32` 转换失败；
- 批量记录序号与稳定车辆标识。

错误不得只返回自由文本原因，也不得携带引擎对象、第三方向量或未受控的线格式载荷。

## 9. 批量位姿提取

输入是 `PoseInput`，输出是 `CanonicalPoseBatch`。形状、首错与「先写入 scratch、全部成功后才提交 output」见 [Adapter API §4][adapter-api-pose] 与 [§6][adapter-api-batch]。

- 批量 API 不遍历引擎实体组件系统（ECS），也不持有宿主演员或实体。
- 采样按已解析的边序号或停车位序号查询，不在每辆车的高频路径中解析外部字符串 ID。
- 调用方可跨 tick 复用输出；#137 已验证稳定容量下的零分配和一万/十万性能。逐轮固定机结果见 git 历史，那些毫秒数不是当前产品认证。
- Adapter 在提交宿主 Transform 前比较 batch token 与当前 placement token；同一 frame 的 placement 切换必须颁发新 token。
- 表现插值、细节层次和相机相对原点切换不能回写 Runtime/Spatial 的正本状态。

## 10. 停车位姿

停车位的入口边、入口进度、横向偏移、朝向、长度和宽度存放在共享路网修订的交通关系里。`PoseSource::Parking` 只携带停车位序号。Spatial 用该序号读取这些静态值，再采样标准位姿：

```text
anchor = sample(entry_edge, entry_progress)
left = normalize(anchor.up × anchor.tangent)
position = anchor.position + left * lateral_offset
heading = anchor.tangent * cos(heading_offset_radians)
        + left * sin(heading_offset_radians)
```

横向偏移在采样时由毫米换到米，朝向为弧度；结果必须进入 canonical `f32` 点与单位方向。越界时作为对应 batch record 的结构化错误返回。长度和宽度仍留在该静态关系中，不由 Runtime 再提供一份几何。Spatial 只计算位姿，不验证多边形重叠、机动轨迹、地形贴合或停车网格。车辆生命周期和泊位绑定仍由 Traffic Runtime 拥有；适配器只消费最终位姿。

## 11. 第三方 Rust 包边界

首轮生产实现选择 LaneFlow 自有类型，不增加第三方依赖：

| 候选                  | 结论              | 主要原因                                                        |
| --------------------- | ----------------- | --------------------------------------------------------------- |
| LaneFlow 自有类型     | 首选              | 操作面最小，点、向量、坐标框架和错误语义明确，无依赖与 API 泄漏 |
| `euclid`              | 保留为内部备选    | 支持带类型单位和空间，但当前包装/转换收益不足                   |
| `glam`                | 适配器末端候选    | 轻量且贴近游戏引擎，但不区分点、向量与坐标空间                  |
| `nalgebra`            | 不采用            | 通用线性代数能力和依赖面超出折线需求                            |
| `mint`                | 不采用            | 只提供互操作数据类型，不提供数学、校验和空间正本                |
| `kurbo` / `lyon_geom` | 不采用            | 面向二维曲线与渲染，首版明确只使用折线且不使用样条曲线          |
| `geo`                 | 不采用            | 二维地理算法与依赖面过大，坐标参考系统仍不是首版目标            |
| `rstar`               | 延后到 #72 类需求 | R 树解决空间查询与索引，不解决中心线正本和采样                  |

完整版本、MSRV、许可证、特性开关、依赖与维护证据见 git 历史中的 v0.6 Spatial 验证记录。

## 12. 确定性与验证

生产正确性测试覆盖：

- 同一运行时重复构建、采样和批量处理时，结果逐位相等，或具有明确的连续值契约；
- 顶点与端点、微小或退化线段、近垂直朝向基；
- 长度容差边界内外；
- 当前 `f64 EdgeLength` 的零量化余量与绝对/相对容差边界；下一规范化 `f32 EdgeLength` 仍属于未来迁移验证；
- 相连端点容差边界内外；
- Traffic Runtime 终点到几何终点的精确映射；
- canonical 点范围闭区间、量化后重合点拒绝，以及受检 `f64 -> f32` 输入转换；
- 坐标框架或宿主放置不匹配、溢出、非有限值和批量失败原子性；
- 一万和十万记录的批量吞吐量、内存分配与保留内存；
- 相对同一条 `f64` 参考折线的位置误差 `<= 1 cm`、切线角误差 `<= 0.5°`；
- #137 固定性能机上的一万 p95 `<= 2 ms`、十万 p95 `<= 20 ms`、稳态零分配和 `<= 12x` 扩展；这些是当时的 Spatial 提取门槛，不是当前产品认证，协议见 [性能基线][performance-baseline]；
- 相同布局下相对 `f64` 候选 retained memory 至少降低 `25%`，吞吐回退不超过 `5%`；
- Bevy 宿主的坐标轴、手性和 `f32 Transform` 集成（v0.7）。

#135 已以 production 单元测试与公共 API 测试验证上述单条折线契约。#136 进一步验证 Lane/Parking 稳定顺序、frame mismatch 优先级、placement token 切换、record 上下文、Parking offset/heading/range、伪宿主 stale-token 拒绝，以及中间失败时旧 batch 不变。#137 已交付误差 oracle、稳态零分配、retained memory 和一万/十万性能 Gate；历史研究原型不替代 production 验证或性能结论。

## 13. G1 后实施拆分

1. LaneFlow 自有 Spatial 类型与 Rust 包依赖方向（#133 已建立生产边界）。
2. 折线构建、长度绑定、拓扑连续性与采样（#135 已交付）。
3. canonical `f32` 批量位姿、frame placement token 与停车位姿解析（#136 已交付）。
4. 性质测试与边界测试、一万/十万性能验证，以及适配器契约冒烟测试（#137 已交付）。
5. v0.6 Spatial 最终审阅；之后 #121/v0.7 才能进入 Bevy 实施。

#141/ADR 0014 不改变上述 Spatial 分层和实现顺序，只修订 Core 标量、有效进度与长度绑定容差来源。#127 已完成 target-f32 量化余量标定；#144 原子迁移形成不迁移（no-go）结论后，Spatial 生产绑定仍不能把目标 `f32 EdgeLength` 当作当前稳定输入。ADR 0015 进一步把 Spatial runtime geometry 修订为有界 `f32` canonical frame；#134/#135/#136 已分别交付转换、量化后几何和批量 frame/placement 生命周期，#137 已固化 production correctness、allocation、retained memory 与性能证据。

## 14. v0.8 走廊几何口径

#184 的“道路总长”按三条物理轴线计：主干道 800 m、两条次干道各 300 m，默认合计 1.4 km，配置上限 2 km；directed lane edge、各 lane 和 connector 不重复计入产品道路总长。Traffic progression 仍以各 edge 的交通 `EdgeLength` 为正本，Spatial pose 仍以绑定后的 centerline 弧长为正本，两者由 generator 的同一中心线输入生成并通过既有 binding validation。

每个路口的每条直行 lane 使用独立 connector edge；#229 已把每条 traversal 显式归属到 ManeuverPath，并以 entry-transition ManeuverGate 绑定 StopLine。默认坐标、车道宽度、路口位置和 14 条 routes 见 `example-scenarios.md`；#188 负责 generator 与制品验证，#229 的 Traffic clean-break 不改变 SpatialPackage v0.1 shape。

## 15. v0.9 受保护转向 target

#196 保持 SpatialPackage 0.1 shape、canonical frame、量化、长度绑定与 pose
authority 不变，但将 connector profile clean-break 为 32 条 lane-level
ManeuverPath：

- same-lane straight 使用两点折线；
- main `lane 1 -> lane 0` straight 使用纵向线性、横向 smoothstep 的 64 段折线；
- left/right 使用 64 段 XZ quarter-ellipse；
- 全部采样点量化到 1 mm，Traffic length 从同一量化后 point sequence 派生；
- 继续执行 1 cm length binding、5 mm join、0.1 m 最小线段与既有 pose validation。

完整 boundary、公式、reference lengths、turn radius 和安全净距见
[`signalized-corridor-protected-turning.md`][protected-turning]。
这些是 #229 的 Accepted target；在生成制品和 loader 往返通过前，不代表 current
Spatial artifacts 已改变。

## 16. WaitingZone / ConflictZone Accepted 设计边界

#235 的 Accepted 设计
[`waiting-zone-conflict-right-of-way.md`][waiting-zone]
允许未来 SpatialPackage 绑定 WaitingZone/ConflictZone 的 canonical 3D region，
用于 authoring validation、debug draw 与 presentation；Traffic/Runtime 的
Gate-bounded path interval、PathAnchor、ParticipantStream/ConflictZone 显式关系
仍是行为 authority。二维中心线相交、mesh bounds、physics collider 或 Adapter
visual overlap 不能自动创建/删除 conflict 语义，高架/下穿不得因俯视相交被误判。
Spatial geometry tolerance 也不能充当 Traffic Runtime PathAnchor crossing tolerance；#235
已为 conflict boundary 冻结独立 Traffic Runtime numeric owner。
具体 Spatial shape/version 由后续独立 implementation G1 决定，#235 不改变 current
SpatialPackage 0.1。

[protected-turning]: signalized-corridor-protected-turning.md
[waiting-zone]: waiting-zone-conflict-right-of-way.md
[adapter-api-pose]: adapter-api.md#4-位姿输入与输出
[adapter-api-batch]: adapter-api.md#6-批量处理与错误语义
[road-editing-source]: road-editing-source-and-geometry-frontend.md
[frame-identity]: #canonical-frame-identity
[performance-baseline]: core-runtime-performance-baseline.md
