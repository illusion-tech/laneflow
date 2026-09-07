# 城市工作负载拓扑生成器

按[已接受的 LF-CN-URBAN-v1 设计](../../docs/design/chinese-style-city-workload.md)生成
10k / 100k 的正式静态输入。这里的规模是后续需求计划的名义个体数；本工具编译路网、
构建共享根、安装空世界和注册路线，不生成活跃车辆。七项行为验证由 #544 交付，
Adapter、保存/恢复及固定增容变体由 #545 交付，产品认证由 #539 / #305 负责。

## 重建与比较

从仓库根目录执行，父目录须存在，输出目录须尚未存在。失败会保留已写入的来源，
重试使用新的目录；没有覆盖已有结果的命令。

```powershell
New-Item -ItemType Directory -Force target/urban | Out-Null
cargo +1.98.0 build --release --locked -p laneflow-urban-generator
target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 10k --output target/urban/10k-a
target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 10k --output target/urban/10k-b
target/release/laneflow-urban-generator compare target/urban/10k-a target/urban/10k-b
target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 100k --output target/urban/100k-a
target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 100k --output target/urban/100k-b
target/release/laneflow-urban-generator compare target/urban/100k-a target/urban/100k-b
```

小型规范夹具使用 `--scale fixture`，由两个相邻完整 tile、20 个 cells 组成；同样
执行两次并比较。测试执行该完整链路，仓库的 `fixtures/v1` 保存其配置入口、规范
manifest 与摘要基线。正式两档的报告在 `fixtures/v1/10k`、`fixtures/v1/100k`。
生成的大型 LFRE / LFCA / LFSM / LFSD 和路线目录放在忽略的 `target/`，不依赖远端发布。
同一工具链、同一配置、同一版本源码是重现边界；计时不参与规范字节比较。

输出包括：

| 文件                               | 用途                                               |
| ---------------------------------- | -------------------------------------------------- |
| `config.toml`                      | 规范化的完整配置，保留全部不同物理参数档           |
| `common.lfre` / `topology.lfre`    | 通过 RoadEditing 正式前端编制的来源及其唯一导入边  |
| `network.lfca`                     | 后发射检查通过的规范静态路网                       |
| `source-map.lfsm` / `genesis.lfsd` | 对应的来源映射、创世语义差分                       |
| `routes.toml`                      | 调用方的路线、停车目标、信号周期及身份目录         |
| `manifest.toml`                    | 规范摘要、计数、预算结果和静态验收结论             |
| `measurements.toml`                | 本次运行的阶段计时及可选分配器观测，不属于规范字节 |

`compare` 逐字节比较前八个规范文件（两个 LFRE 分别计数），包含 manifest；缺失或
不同即失败。`manifest.files` 给出七个载荷的 SHA-256 和字节数，避免 manifest 自引用。

<a id="templates"></a>

## 正式模板与连接规则

`examples/config/cn-urban.toml` 固定 `connected-cn-urban-v1`、十个模板槽、相邻连接和
边界端口规则。v1 是闭合模板集；更改固定规则须修改版本与本说明，不能靠未生效的
自由配置制造新路网。250 m cells 按每 tile 两列五行排列；tile 列数是满足
`columns² >= 2 × tile_count` 的最小整数，逐行铺设实际 tile，末行空位不物化。
10k 为 100 cells / 10 tiles / 5 列，100k 为 1,000 cells / 100 tiles / 15 列。

同一个规范框架采用 +X 向东、+Y 向上、+Z 向南。每条道路方向拥有独立的道路走廊、
参考线、路段、编制车道和车道边；东西为干路，南北为支路。每个存在的路口臂有一条
进入边和一条离开边，车道中心距道路轴线 6 m，宽 3.5 m，右侧通行。路口内部从
中心半径 30 m 开始，以直线或三次曲线表达全部非掉头转向。配置分别固定干路、支路、
内部边限速。

| tile 内槽位 | 模板                                                 |
| ----------- | ---------------------------------------------------- |
| 0           | 受保护四岔口，每个左转含一条 12 m、容量为 1 的待转边 |
| 1           | 信号无保护左转四岔口，左转与同轴直行共享信号组       |
| 2、3        | 相隔一格的南/北支路 T 口，构成沿东西干路的错位双 T   |
| 4           | 主支路让行 T 口                                      |
| 5–9         | 受保护四岔口                                         |

`reciprocal-cardinal-ports-v1` 只在相邻两格均具有对应臂时，将离开边连接到相邻进入边；
跨 tile 使用完全相同的规则。`unmatched-ports-open-v1` 将地图边界或不成对的臂保留为
开放入口/出口，不增造路段。每格至少有东西臂，公共干支路连接所有实际 cells。
验收在实际共享根上检查单一弱连通分量、全部有向 successor 和连接端点几何连续性；
每条目录路线再通过 Runtime `register_route` 的合法出现项检查。

受保护路口依次启用东西直行、东西左转、南北直行、南北左转组；无保护路口启用东西、
南北直行组。每组都有绿、黄、全红三个相位，其他组保持红灯。所有时长和错峰偏移按
528 ms 量子生成，配置默认直行 60、左转 40、黄灯 6、全红 2 个量子。每格的偏移为
`cell_index × offset_step_quanta` 对周期量子数取模。待转的准入/入口门跟随同轴直行组，
释放门跟随同轴左转组，后续需求计划可以据此选择完整观察周期。

冲突区来自固定模板中不同入口流的实际中心线相交或出口合流：三次曲线分成 32 段定位
相交点，每对冲突流建一块 4 m × 4 m 区域。每条流的 passage 保守覆盖从准入门到
出口边界的内部路段；待转流从释放门开始。共享入口的分流由上游车道占用约束。
此算法仅编制本套有限模板，不承诺作为任意几何的通用碰撞检测器。

每个无保护左转都有实际冲突直行流的让行关系；无信号口采用东西干路优先、直行优先，
再按固定方向顺序确定优先级，对实际冲突且优先级更高的流让行。所有门均有显式策略。
策略身份为 `urban-policy`，工程法域 `engineering-workload`、版本
`cn-urban-template-v1`，依据定位符指向本节。间隙采用现有
`urban-conservative-v1`：前向 5,000 ms、后向 2,000 ms、清空 500 ms；由 Runtime
按实际步长解释。这是通用技术验证输入，不构成现实道路法规认证。

## 物理配置与停车

共同模块只声明一个 `road-vehicle` 类别和三个不同的物理配置：4 m 的 compact、
4.5 m 的 car、6 m 的 van；完整速度、加减速度、间距和时距参数保存在配置中。
两档共用该目录，同一类别按现有路权共享解析规则工作。外观目录由宿主拥有，本工具
不生成模型、颜色或材质项；不能用外观数量替代物理配置数量。

每个 cell 有一个混合设施：虚拟容量默认 100、一个入口、一个出口，以及两个独立显式
泊位。第一个显式泊位与虚拟池共用进入锚点；两个泊位分别位于离开道路的 40 m / 70 m，
在 60 m / 85 m 离场，具有独立身份和真实位姿，默认长度 6.5 m。每 tile 另有一个地下
设施，虚拟容量默认 1,000，两个入口、两个出口，接入最后两格的共同道路。
地下容量不会展开成内部路网、伪泊位或虚拟位姿。

默认每 tile 可分配位置为 `20 + 10 × 100 + 1,000 = 2,020`；两个门仍只计一个虚拟池。
路线绑定检查每个显式泊位及虚拟池的实际编译身份、锚点、路线位置和车型准入，显式
泊位还检查每个配置的车长可容纳。只有通过检查的目标参与逐 tile 容量核算，要求
每 tile 至少 750。设施总容量不再与显式泊位和虚拟容量相加。

`routes.toml` 的 `parking` 项是实际独立分配池：显式泊位容量为 1，虚拟池为设施的
`virtual_capacity`；保留 tile、目标与设施 StableId、进入/离开路线、边上毫米进度和
`route_edge_index`。虚拟目标的 `virtual_anchor_index` 来自编译后的规范锚点序列，
不能用来源输入顺序猜测。`parking_capacity_by_tile` 为 #544 分配初始停驻个体提供上限。

## 调用方目录与度量边界

目录是仓库内部版本化 TOML，不是新增 Runtime wire。调用方应将 `edge_ids`、
`profile_ids`、`frame_id`、`policy_id` 绑定到同一个 `network_revision` 的实际共享根，
使用 `PolicyPin` 安装世界，然后注册 `routes` 的有序边序列。路线包括各路口转向、
每个有向跨格/跨 tile 连接以及每个停车锚点的进入/离开行程。信号目录给出完整周期、
偏移、相位顺序及灯态，需求计划无需从名称猜测时序。OD、出发时刻、case seed、
车辆初态及运行命令由调用方计划拥有，不写入 LFCA。

生成链路使用原有 `LF-COMP-SINGLE-NETWORK-1M-v2`、`FormatLimits::HARD`，文件支持的
发射暂存，以及共享根各 2 GiB 的 retained / scratch 预算，路权解析限制沿用默认值。
分别构建 headless / Spatial 共享根并安装空世界；Spatial 路径注册全部目录路线。
显式泊位通过 `SpatialSession` 提取实际位姿，与车道折线、冲突区域一起核验规范
框架坐标边界。测试覆盖完整两 tile 模板、跨 tile 路线、三个控制模式及重复生成。

manifest 的来源引用数是公开 RoadEditing 声明中带类型的实体引用出现次数，按引用
所属声明类型分类；参考线、冲突区域单列。LIR 只读取公开视图；逐 LFCA / LFSM 表和
嵌套记录向量计数来自受检实际制品，报告分块数及最大块行数/字节数。`lane_successors`
是 LIR 显式道路连接；共享根还由 ManeuverPath 推导路口内部连接，两者不能混为一数。
`road_alignment_length_mm` 是实际编制的有向道路参考线长度，不是去重后的物理街道长度；
`lane_length_mm` 包含路口内部车道。HIR / MIR 内部计数明确未测量，不以推算代替。

CLI 的分配器观测按阶段记录分配调用数、累计请求字节和净存活请求字节变化；这些值
不是堆峰值或操作系统工作集。编译器受控峰值、输出逻辑字节和共享根保留逻辑字节
单独记录在 manifest。库调用未提供仪表分配器时省略该观测，而非报告假零值。
阶段计时用于静态链路诊断，不是道路活动个体性能或产品认证测量。

本工具只依赖工作区 crate 和锁文件已有版本的 serde、sha2、thiserror、toml、tempfile、
stats_alloc；统计分配器只安装在 CLI，不进入 Runtime 固定步进或引擎适配器。
