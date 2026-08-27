# 运行时快照

**文档状态**: Accepted（#302 G1）<br>
**最后更新**: 2026-08-27<br>
**适用范围**: 版本化 Runtime Snapshot 容器、绑定集、保存/恢复合同、检查点与
回放、确定性状态摘要、跨修订迁移入口<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)（§12；其中 static image digest / validation receipt 条款已被 ADR 0025 §8 取代，origin 以 LFCA 为准，以本文与 ADR 0025 §8 为权威）、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0028-integer-millimeter-traffic-geometry.md`](../adr/0028-integer-millimeter-traffic-geometry.md)、
[`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)、
[`retire-precompiled-static-route.md`](retire-precompiled-static-route.md)、
[`shared-static-network.md`](shared-static-network.md)

本文是 ADR 0020 §12（按 ADR 0025 §8 修订后语义）的实现级合同。路线与车辆的快照
表示沿用 ADR 0029 §6 / `retire-precompiled-static-route.md` §5 已冻结形状，本文
只做全世界扩展与容器合同。G2 决定 Rust 拼写。

## 1. 制品定位与版本轴

- Runtime Snapshot 是**独立版本化制品**，不进入 LFCP 发布链，也不绑定 LFCA 的
  跨对象信任；其真实性由宿主存档清单（ADR 0021，城市游戏层拥有）在对象外绑定。
- 版本轴分离（术语表：版本轴）：
  - 容器 `formatVersion`（本文合同，v1）；
  - 被绑定事实的版本：runtime 版本、static-contract versions、
    `networkRevisionDerivationVersion`、identity registry revision。
  - 不混用单一版本号表达多条兼容轴；未知版本值失败关闭。

## 2. 绑定集

必绑（缺失即失败关闭）：

- LFCA origin：LFCA digest / byte length / `NetworkRevisionId`；
- 来源指名：`CommittedNetworkSource`（editable 世界 = committed
  `RoadEditingState` 指名；runtime-only 世界 = `PublishedLfcaReference`）；
- runtime 版本、static-contract versions、`networkRevisionDerivationVersion`、
  identity registry revision（ADR 0020 §12 要求 identity / constraint /
  execution-constraint versions 精确相等；各轴到容器字段的显式映射由 G2 登记本文）；
- 世界身份（快照局部）、`tick` / `time_ms`、输入命令游标；
- 全部每世界可变状态（§4）；
- **`WorldConfig` 快照**：整份记录作恢复核对。`fixed_delta_time_ms` 恢复时必须
  精确相等（信号派生与 tick 语义依赖）；其余容量类字段恢复时允许不同（由恢复方
  重新声明），但**检查点/回放的 exact oracle 要求整份 `WorldConfig` 一致**——
  容量差异会改变重放中生命周期命令的成败，破坏逐点相等。

禁绑（出现即失败关闭）：runtime handle / slot / generation、密集序号、共享静态
数组、`EditableDiffBase`、partition / worker assignment、数组地址 / layout /
capacity、调用方自有 seed/随机流（由宿主存档清单绑定；Runtime 当前没有自有随机
流，若未来 G1 引入，仅该显式授予的流进入快照）。

## 3. 容器编码

- 编码采用仓库封闭契约先例：size-prefixed FlatBuffers 文档 + file identifier
  `LFRS` + `formatVersion` 字段；schema 位于 `schemas/runtime-snapshot/v1`，
  生成物经独立私有 wire package 隔离（沿 `laneflow-road-editing-wire` 先例）。
- 读取 verifier-first：语义 lowering 前完成长度、基数与版本预检；未知
  `formatVersion`、未知表、越界或损坏输入失败关闭，不进入部分恢复。
- 发布链的自定义规范制品仍只有 LFCA / LFSM / LFSD / LFCP；快照不是发布对象，
  不成为第二类规范制品格式，也不要求跨实现字节规范序，只要求逻辑状态确定性与
  可验证性（§6）。

## 4. 每世界可变状态清单

| 状态         | 快照表示                                                                                                                                                                                                                                                             |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 路线表       | ADR 0029 §6 形状：`snapshot_route_id` + 有序边 `StableId128` 序列（允许重复边）                                                                                                                                                                                      |
| 车辆         | ADR 0029 §6 形状 + 每车唯一快照局部车辆标识 `snapshot_vehicle_id`：`snapshot_route_id` + `route_edge_index` + `progress_mm` / `carry_um` / `speed_mm_s` / `status`；profile / class / parking 等静态绑定用 `StableId128`（车辆自身是运行时实体，没有 `StableId128`） |
| 停车占用     | 车辆 `snapshot_vehicle_id` ↔ 停车位 `StableId128` 占用绑定与 parked 状态；预约 / 到场 / 离场状态机未冻结（现行消费契约只冻占用互斥），不在本容器字段内                                                                                                               |
| live 顺序    | 车辆 `snapshot_vehicle_id` 的规范排序序列                                                                                                                                                                                                                            |
| tick / 时钟  | `tick` / `time_ms` / 输入命令游标                                                                                                                                                                                                                                    |
| 信号灯色     | **不入快照**：由 `time_ms` + 共享根 program + offset 派生（现行消费契约）                                                                                                                                                                                            |
| 车道占用索引 | **不入快照**：由车辆状态确定性重建（占用索引每 tick 从已提交状态重建的现行契约）                                                                                                                                                                                     |

- 快照局部标识（术语表）只在单个快照内稳定；恢复时静态引用经
  `SharedIdentityIndex` 解析 `StableId128`、动态实体经快照局部标识表，**新分配**
  句柄。原进程 `RouteHandle` / `VehicleHandle` 不得成为恢复后身份。
- 一维数值全部为整数毫米 / 微米 / `u32` mm/s（ADR 0028），无浮点字段。

## 5. 保存与恢复合同

- **保存**只接受已进入活动聚合的 `CommittedNetworkSource`；working / candidate
  与 `EditableDiffBase` 不进入存档。保存发生在候选准备期间时仍只捕获旧聚合与
  对应快照（切换文档 §8）。
- **editable 来源恢复**：先从 committed `RoadEditingState` 重新编译，建立新
  session 的根与 exact LFCA base，再恢复快照；重编译失败、修订/契约版本不匹配
  或缺 `EditableDiffBase` 对应关系均失败关闭。同修订不同 LFCA 字节允许（§7）：
  重编译产物的 origin digest / length 差异只承担来源审计，不构成拒绝条件。
- **published 来源恢复**：经不透明 asset key + LFCA digest / length / revision
  的 `PublishedLfcaReference` 重新认证并读取 LFCA，构建共享修订后恢复快照；
  资产缺失、摘要错配失败关闭。
- 发布资产要启用道路编辑，必须另带 committed 道路状态，并以重编译 exact LFCA
  执行同修订 root / source / diff-base rebase；成功前不得启用编辑。
- 恢复成功后世界处于快照 `tick` 边界的一致状态；`install` 核对与
  `register_route` 重建沿现行消费契约执行。

## 6. 回放、检查点与确定性状态摘要

- **输入命令序列由宿主记录并按序重放**（术语表：输入命令序列）；Runtime 不新增
  隐藏输入权威。回放 = 恢复快照（或检查点）后按序重提交命令。
- **回放身份协议**：输入命令序列的目标标识必须使用耐久身份——快照局部标识
  （如 `snapshot_vehicle_id`）或宿主自有 ID，**不得直接记录进程句柄**（恢复后
  句柄重新分配，重放 stale 句柄是调用方错误）。恢复时宿主经「快照局部标识 →
  新分配句柄」映射重提交命令；检查点之后由命令新建的实体，其耐久 ID 由命令
  载荷自带。Runtime 不维护跨恢复的句柄映射。
- **确定性状态摘要（术语表）**：对逻辑状态按规范排序计算的版本化摘要。本 G1
  冻结摘要算法为 SHA-256（沿 `laneflow-format` 先例）并前置域分隔版本前缀；
  摘要输入的精确规范化序列化由 G2 落定并登记本文。本项履行 ADR 0020 §12 /
  ADR 0021 §6 指派给后续运行时 G1 的摘要算法冻结义务；检查点采样频率由宿主
  选择，发布构建裁剪不适用（快照不是发布对象）。摘要对逻辑状态负责，不比对
  容器字节；同一逻辑状态在任何合法编码下摘要相等。
- **检查点（checkpoint）**：回放序列中周期保存的快照锚点，与输入命令游标共同
  界定重放区间；缩短恢复与失同步定位区间。
- 失同步诊断边界：摘要不等即报告失同步与首个可定位区间，不做自动纠偏、不重启
  世界；处置由宿主决定。

## 7. 跨修订迁移入口

- 跨修订恢复必须由受信任 `NetworkRevisionCutoverDescriptor` 驱动，经 LFSD 与
  稳定标识执行**显式迁移**（切换文档 §3：封闭种类选择器
  `same_revision_restore` / `cross_revision_direct`，无宿主回调，不可映射实体——
  含重绑后非法——整体失败关闭）。
- 旧密集序号不得直接解释为新修订实体；`SharedIdentityIndex` 完整重建稳定静态
  引用。同修订恢复不要求同 LFCA 字节：identity / constraint /
  execution-constraint versions 精确相等即可（ADR 0020 §12 语义，origin 形态
  按 ADR 0025 §8 修订；同修订换根的切换事务路径见切换文档 §3）。

## 8. G1 预算与度量

ADR 0021 §8 与 ADR 0020 验证门要求量化快照的制品尺寸与停顿；度量协议与切换
文档 §9 相同（同机描述性基线，方向只许收紧）。维度：

| 维度         | 口径                                                             |
| ------------ | ---------------------------------------------------------------- |
| 制品字节     | 快照 exact bytes（含头部与全部表）按生产规模登记                 |
| save 停顿    | 保存墙钟与主线程停顿（保存发生在稳态世界上的干扰）               |
| load 停顿    | 恢复墙钟与主线程停顿（editable 重编译 / published 认证分别度量） |
| 恢复峰值内存 | 恢复期间解析、身份重建与动态状态物化的峰值                       |
| 稳态干扰     | 保存期间对 tick 的干扰（不改变已提交状态与事件语义）             |

## 9. 必测项（G2）

- 同修订 save → load exact oracle：逻辑状态（路线边序列、毫米游标、速度、状态、
  停车占用、live 序、`tick` / `time_ms` / 输入命令游标）全等；句柄不比对
  （新分配）。
- 检查点 + 命令重放与连续步进摘要逐点相等；失同步注入能定位区间。回放身份协议：
  以 `snapshot_vehicle_id` 等耐久身份记录的命令序列，恢复后经映射重放与连续
  步进逐点相等。
- `WorldConfig` 恢复规则：`fixed_delta_time_ms` 不一致失败关闭；检查点/回放
  oracle 在整份 `WorldConfig` 一致时逐点相等、容量差异下显式不等（不误判为
  失同步缺陷）。
- 跨修订直移：直移成功 oracle 与不可映射实体整体失败关闭（含删边上有车、路线
  引用被删边、停车绑定失效三类）。
- 容器拒绝面：未知 `formatVersion`、损坏长度、越界基数、含禁绑字段（句柄/
  generation/密集序号/浮点一维值）全部失败关闭且零部分恢复。
- editable / published 两类来源恢复流程端到端；发布资产缺 committed 道路状态
  时编辑不可启用。
- 保存与候选准备并发时只捕获旧聚合；source rebase / diff-base binding 缺失或
  错配失败关闭。
- §8 五个预算维度按基线登记并进入量化切片证据。
