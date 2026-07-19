# 空间几何设计

**文档状态**: 已接受（Accepted）

**最后更新**: 2026-07-18（#141 ADR 0014 数值边界同步）

**适用范围**: v0.6 引擎无关的标准坐标框架、折线中心线、长度绑定、采样、局部位姿与制品配对（#123）

**关联文档**:

- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `numeric-representation.md`
- `data-format.md`
- `adapter-api.md`
- `parking-system.md`
- `../reference/v0.6-spatial-validation.md`

## 1. 目标、边界与术语

本文定义如何把 Core 的“边句柄 + 边内进度”映射为引擎无关的位姿，同时保持 Core、Spatial 与 Adapter 的权威职责分离。

本文使用以下术语：

- **权威职责（authority）**：某项状态由哪一层定义并最终裁决。
- **标准坐标（canonical）**：LaneFlow 统一使用、尚未转换为宿主坐标的空间。
- **局部坐标（local）**：从标准坐标减去选定局部原点后得到的表现坐标。
- **位姿（pose）**：位置、切向量和上方向向量的组合。
- **制品（artifact）**：可独立版本化、摘要校验和加载的数据文件。
- **场景清单（manifest）**：引用并精确配对交通包和空间包的入口制品。
- **层级名称**：Core、Data、Spatial、Adapter 分别表示核心层、数据层、空间层和适配层。
- **应用程序接口（API）**：供其他包或宿主调用的稳定程序边界。

| 层级    | 权威职责                                               | 不负责                                               |
| ------- | ------------------------------------------------------ | ---------------------------------------------------- |
| Core    | 拓扑、`EdgeLength`、路线区段、`EdgeProgress`、交通状态 | 标准或世界坐标、中心线、宿主变换                     |
| Spatial | 标准坐标框架、中心线、弧长、绑定、采样和位姿           | 交通规则、网格与材质、引擎生命周期                   |
| Data    | 交通包、空间包和清单的线格式校验与规范化               | 运行时推进、文件或网络输入/输出（I/O）、宿主资源句柄 |
| Adapter | 快照调度、局部原点、宿主朝向基与变换、表现提交         | 重算交通进度、覆盖几何或长度权威                     |

Core 可以在没有 Spatial 数据包时以无图形宿主方式运行；需要车辆空间位姿的调用方必须提供已经通过配对和绑定的 Spatial 注册表。

## 2. 坐标框架

### 2.1 标准坐标契约

| 属性         | v0.6 契约                                   |
| ------------ | ------------------------------------------- |
| 标量类型     | LaneFlow 自有 `f64`                         |
| 线性单位     | 米                                          |
| 角度单位     | 弧度                                        |
| 坐标系手性   | 右手                                        |
| 上方向轴     | `+Y`                                        |
| 水平面       | `X/Z`                                       |
| 全局前方向   | 无；边的前方向由有向中心线切向量定义        |
| 坐标框架标识 | 必填且稳定的 `frameId`                      |
| 地理配准/CRS | 首版不表达；后续通过独立 ADR 和格式版本加入 |

`frameId` 只表示坐标空间身份，不表示 EPSG、经纬度、地图服务或宿主场景名。首版禁止把未定义语义塞入不透明扩展字段，否则不同工具会在没有校验的情况下形成隐式格式。

### 2.2 局部表现坐标

局部原点是同一标准坐标框架中的 `f64` 点，由适配器、相机或分块管理器，或者调用方选择。转换顺序固定为：

```text
Core 的有效 `f64` 进度
  -> Spatial 的标准 f64 位姿
  -> 标准位置 - 局部原点（f64）
  -> 校验坐标框架、有限性、局部范围和朝向基
  -> 经过检查的局部 f32 位姿
  -> 宿主专用朝向基和变换（Transform）
```

局部原点不写入 Core 车辆状态，不改变标准几何，也不允许先把标准位置和原点分别转换为 `f32` 再相减。

## 3. 制品模型

### 3.1 三类制品

下列名称表示后续数据规范可能采用的概念名称，不是当前 v0.5 已接受字段：

```text
ScenarioManifest（场景清单）
  traffic: 制品引用 + 原始字节 SHA-256 摘要
  spatial: 制品引用 + 原始字节 SHA-256 摘要

TrafficPackage v0.5（当前有效，保持不变）
  laneGraph.edges[].id / length / connections
  routes / profiles / signals / parking

SpatialPackage（空间包；后续数据规范 Issue 定义首版模式）
  空间格式版本
  坐标框架
  edges[].trafficEdgeId
  edges[].centerline.points[]
```

- 场景清单使用制品原始字节的 SHA-256 摘要精确配对内容。路径或文件名不构成身份，也不得先重新序列化 JSON 再计算摘要。
- 空间边使用交通数据中的外部边 ID 绑定，加载后转换为紧凑的不透明绑定。
- 如果提供空间包，它必须完整覆盖交通车道图。缺失、重复或未知交通边全部返回阻断错误；只使用 Core 的调用方可以完全不提供空间包。
- 当前 Traffic Data v0.5 的模式、加载范围和诊断不因 #123 改变。空间模式和场景清单使用独立版本系列；精确字段与发布契约由后续数据规范 Issue 按 ADR 0008/0011 交付。
- 加载器接收调用方已经读取的字节或字符串，不读取引擎路径，不解析远端 `$id`，也不创建引擎资源。

### 3.2 配对与加载顺序

```text
校验清单结构、版本和摘要
  -> 校验当前有效的交通包
  -> 校验空间包版本、坐标框架、有限性和结构
  -> 校验外部边身份与完整覆盖
  -> 校验折线结构
  -> 预计算几何弧长
  -> 校验 Core 长度与几何长度的一致性
  -> 校验相连端点的连续性
  -> 生成不可变的已绑定 Spatial 注册表
```

任一步失败都不得返回部分注册表，也不得修改已存在的 Core 世界。

## 4. LaneFlow 自有空间类型

首版公共契约概念上包含以下类型；名称暂作技术标识符：

```text
CanonicalFrameId
CanonicalPoint3F64
CanonicalVector3F64
CanonicalPoseF64 { position, tangent, up }
LocalOriginF64 { frame_id, position }
LocalPoseF32 { frame_id, origin_id, position, tangent, up }
SpatialError
```

具体 Rust 命名由实施 Issue 固化，但必须满足：

- 点与向量是不同类型，不能用一个裸三元组同时表达二者。
- 标准与局部、`f64` 与 `f32`、不同坐标框架或原点，不能在没有检查的情况下混用。
- 公共字段不暴露 `euclid`、`glam`、`nalgebra`、Bevy 或其他第三方类型。
- 所有构造器在进入权威边界前拒绝非有限值，并把带符号零规范化为正零。

## 5. 折线与预计算

每条交通边对应一条有向三维折线：

```text
P0 -> P1 -> ... -> Pn
```

要求：

- 至少包含两个有限的标准坐标点。
- 每个相邻线段长度严格大于 `SPATIAL_MIN_SEGMENT_LENGTH_METERS`。
- 顶点顺序与交通边的行驶方向一致。
- 每个线段中，标准 `+Y` 投影得到的上方向基必须可以归一化；首版不接受近垂直道路。
- 加载器计算线段长度、归一化切向量与上方向向量，以及累计弧长；这些派生值不从线格式接收。
- 折线拐角允许切向量不连续，不做隐式平滑或样条曲线拟合。

首轮 G1 常量提议：

| 名称                                     |        值 | 单位或语义             | 依据                                              |
| ---------------------------------------- | --------: | ---------------------- | ------------------------------------------------- |
| `SPATIAL_MIN_SEGMENT_LENGTH_METERS`      |  `1.0e-4` | `0.1 mm` 输入有效尺寸  | 拒绝重复点或数值噪声线段；远低于道路尺度          |
| `SPATIAL_LENGTH_ABS_TOLERANCE_METERS`    |  `1.0e-6` | 长度绑定绝对容差下限   | 高于 `f64` ULP 和十进制舍入误差，低于表现误差预算 |
| `SPATIAL_LENGTH_REL_TOLERANCE`           |  `1.0e-9` | 长度绑定相对容差项     | 随边长扩展；10,000 m 时约为 `0.01 mm`             |
| `SPATIAL_JOIN_POSITION_TOLERANCE_METERS` |  `1.0e-3` | 相连端点间隙           | `1 mm`，低于 #122 的 `1 cm` 位置误差上限          |
| `SPATIAL_BASIS_MIN_PROJECTED_UP_LENGTH`  |  `1.0e-6` | 上方向投影的无量纲下限 | 防止近垂直道路的归一化放大误差                    |
| `SPATIAL_BASIS_ORTHONORMAL_TOLERANCE`    | `1.0e-12` | 派生朝向基复核容差     | 只校验 `f64` 归一化和正交结果，不作为几何尺寸阈值 |

这些数值来自 #123 的 `f64 Core EdgeLength` 绑定基线。ADR 0014 接受 `f32 EdgeLength` 作为下一目标后，几何 `f64` 容差继续有效，但不能单独覆盖 Core 长度量化；#125 已拆分 current-f64 Core 边界与间隙 owner，#127 必须离线标定并验证 target-f32 量化余量。

## 6. 长度权威与绑定

### 6.1 权威职责

- `Core EdgeLength`：交通进度、路线距离，以及跟驰、信号和停车约束的权威。
- `Spatial geometry arc length`：标准中心线参数化与位姿的空间权威。
- 创作或导出工具：应从同一中心线来源生成两者，但运行时仍必须验证，不能无条件信任生产者。

### 6.2 一致性检查

```text
difference = abs(core_length - geometry_arc_length)
geometry_tolerance = max(
  SPATIAL_LENGTH_ABS_TOLERANCE_METERS,
  SPATIAL_LENGTH_REL_TOLERANCE * max(abs(core_length), abs(geometry_arc_length))
)

tolerance = geometry_tolerance + core_edge_length_quantization_allowance

difference <= tolerance -> 绑定成功
difference > tolerance  -> 阻断错误
```

当前生产 `f64 EdgeLength` 的量化余量为零。ADR 0014 的下一 `f32 EdgeLength` 契约必须覆盖合法范围内 `f64 -> f32` 舍入到最近可表示值的最坏误差；#127 冻结精确的含等号公式和边界判定基准。#144 曾在生产候选中启用，但形成性能不迁移（no-go）结论后已回退；未来迁移可用一个完整的局部 ULP 作为保守候选。10 km 处一个 ULP 约为 `0.977 mm`。

禁止静默替换、只给警告、按引擎样条曲线重算，或在适配器端修复。也不能因为旧几何容差小于 `f32` 量化误差就把 Core 长度恢复为 `f64`；两类误差必须分别记录并组合。

### 6.3 进度映射

本文中的 `normalized_core_length` 是 Core `EdgeLength` 经过领域构造、校验和规范化后的权威边长，并以 `f64` 观察值提供给 Spatial：当前 `f64 EdgeLength` 直接提供该值；目标 `f32 EdgeLength` 先完成舍入和规范化，再精确升宽为 `f64`。它不是原始 Data 输入，也不是几何弧长。

`snapped_effective_core_progress` 是 Core `EdgeProgress` 的 `f64` 有效值经过领域 edge 边界吸附规则处理后的观察值；current-f64 由 #125 拆分 owner，target-f32 由 #127 标定，但 #144 no-go 后未进入当前生产。未来只有落在端点容差内时才吸附到 `0` 或 `normalized_core_length`，一般越界仍返回错误。

```text
ratio = snapped_effective_core_progress / normalized_core_length
geometry_s = ratio * geometry_arc_length
```

- 进度必须来自经过验证的 Core 有效状态或快照；Spatial 不读取高位/残差分量。
- 只在 Core 边界容差内吸附到 `0` 或 `normalized_core_length`；一般越界返回错误。
- 比例映射确保 Core 终点精确命中几何终点。
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
- 坐标框架不匹配、场景清单摘要不匹配；
- 点数量不足或坐标非有限；
- 退化线段或退化朝向基；
- Core 长度与几何长度不一致；
- 相连端点不连续；
- 进度或弧长越界；
- 局部原点、局部范围或 `f32` 转换失败；
- 批量记录序号与稳定车辆标识。

错误不得只返回自由文本原因，也不得携带引擎对象、第三方向量或未受控的线格式载荷。

## 9. 批量位姿提取

面向适配器的输入是已提交 Core 快照中的稳定序列：

```text
PoseInputRecord {
  vehicle_handle
  edge_handle
  edge_progress
}
```

输出保持相同顺序：

```text
CanonicalPoseRecordF64
或
LocalPoseRecordF32 { frame_id, origin_id, vehicle_handle, pose }
```

- 批量 API 不遍历引擎实体组件系统（ECS），也不持有宿主演员或实体。
- Spatial 注册表按已解析的边句柄或索引查询，不在每辆车的高频路径中解析外部字符串 ID。
- 局部 `f32` 路径先把全部记录计算到临时缓冲区；任何失败都不修改调用方已经提交的输出。
- 调用方可以在成功后交换或复用缓冲区；具体内存分配 API 由实施与性能 Issue 决定。
- 表现插值、细节层次和相机相对原点切换不能回写 Core/Spatial 的权威状态。

## 10. 停车位姿

停车系统继续由 Core 拥有边相对权威：入口边与进度、横向偏移、朝向偏移、长度和宽度。Spatial 解析标准位姿：

```text
anchor = sample(entry_edge, entry_progress)
left = normalize(anchor.up × anchor.tangent)
position = anchor.position + left * lateral_offset
heading = anchor.tangent * cos(heading_offset_radians)
        + left * sin(heading_offset_radians)
```

Spatial 只计算位姿，不验证多边形重叠、机动轨迹、地形贴合或停车网格。已停放车辆的生命周期与绑定权威仍在 Core；适配器只消费最终位姿。

## 11. 第三方 Rust 包边界

首轮生产实现选择 LaneFlow 自有类型，不增加第三方依赖：

| 候选                  | 结论              | 主要原因                                                         |
| --------------------- | ----------------- | ---------------------------------------------------------------- |
| LaneFlow 自有类型     | 首选              | 操作面最小，点、向量、坐标框架和错误语义明确，无依赖与 API 泄漏  |
| `euclid`              | 保留为内部备选    | 支持带类型单位和空间、`f64` 与三维运算，依赖小；当前转换收益不足 |
| `glam`                | 适配器末端候选    | `DVec3` 无依赖且轻量，但不区分点、向量与坐标空间                 |
| `nalgebra`            | 不采用            | 通用线性代数能力和依赖面超出折线需求                             |
| `mint`                | 不采用            | 只提供互操作数据类型，不提供数学、校验和空间权威                 |
| `kurbo` / `lyon_geom` | 不采用            | 面向二维曲线与渲染，首版明确只使用折线且不使用样条曲线           |
| `geo`                 | 不采用            | 二维地理算法与依赖面过大，坐标参考系统仍不是首版目标             |
| `rstar`               | 延后到 #72 类需求 | R 树解决空间查询与索引，不解决中心线权威和采样                   |

完整版本、MSRV、许可证、特性开关、依赖与维护证据见 `v0.6-spatial-validation.md`。

## 12. 确定性与验证

必须覆盖：

- 同一运行时重复构建、采样和批量处理时，结果逐位相等，或具有明确的连续值契约；
- 顶点与端点、微小或退化线段、近垂直朝向基；
- 长度容差边界内外；
- 当前 `f64 EdgeLength` 与下一规范化 `f32 EdgeLength` 的量化余量边界内外；
- 相连端点容差边界内外；
- Core 终点到几何终点的精确映射；
- 标准坐标偏移很大时，先减局部原点再转换为 `f32` 的正确顺序；
- 坐标框架或原点不匹配、溢出、非有限值和批量失败原子性；
- 1 万和 10 万记录的批量吞吐量、内存分配与保留内存；
- Bevy 宿主的坐标轴、手性和 `f32 Transform` 集成（v0.7）。

当前研究原型只证明最小算法和类型候选，不等同于生产验证或性能结论。

## 13. G1 后实施拆分

1. LaneFlow 自有 Spatial 类型、注册表与 Rust 包依赖方向。
2. 空间数据包及其模式、场景清单、加载器与交通包配对。
3. 折线构建、长度绑定、拓扑连续性与采样。
4. 标准/局部批量位姿提取与停车位姿解析。
5. 性质测试与边界测试、1 万/10 万性能验证，以及适配器契约冒烟测试。
6. v0.6 Spatial 收口审阅；之后 #121/v0.7 才能进入 Bevy 实施。

#141/ADR 0014 不改变上述 Spatial 分层和实现顺序，只修订 Core 标量、有效进度与长度绑定容差来源。#127 已完成 target-f32 量化余量标定；#144 原子迁移 no-go 后，Spatial 生产绑定仍不能把目标 `f32 EdgeLength` 当作当前稳定输入。未来重启必须重新通过生产闸口。
