# 运行时快照

**文档状态**: Accepted（#302 G1）<br>
**最后更新**: 2026-08-27<br>
**适用范围**: 版本化 Runtime Snapshot 容器、绑定集、保存/恢复合同、checkpoint 与
回放、确定性状态摘要、跨修订迁移入口<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)（§12，经 #300 G1 修订：静态镜像制品已取消，origin 绑定 LFCA）、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0028-integer-millimetre-traffic-geometry.md`](../adr/0028-integer-millimetre-traffic-geometry.md)、
[`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)、
[`retire-precompiled-static-route.md`](retire-precompiled-static-route.md)、
[`shared-static-network.md`](shared-static-network.md)

本文是 ADR 0020 §12（修订后）的实现级合同。路线与车辆的快照表示沿用 ADR 0029
§6 / `retire-precompiled-static-route.md` §5 已冻结形状，本文只做全世界扩展与
容器合同。G2 决定 Rust 拼写。

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
- runtime 版本、static-contract versions、`networkRevisionDerivationVersion`；
- 世界身份（快照局部）、`tick` / `time_ms`、输入命令游标；
- 全部每世界可变状态（§4）。

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
- LFCA 保持唯一自定义规范制品格式（发布链专用）；快照不是发布对象，不要求跨
  实现字节规范序，只要求逻辑状态确定性与可验证性（§6）。

## 4. 每世界可变状态清单

| 状态        | 快照表示                                                                                                                                                          |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 路线表      | ADR 0029 §6 形状：`snapshot_route_id` + 有序边 `StableId128` 序列（允许重复边）                                                                                   |
| 车辆        | ADR 0029 §6 形状：`snapshot_route_id` + `route_edge_index` + `progress_mm` / `carry_um` / `speed_mm_s` / `status`；profile / class / parking 绑定用 `StableId128` |
| 停车预约    | 车辆 `StableId128` ↔ 停车位 `StableId128` 绑定与状态                                                                                                              |
| live 顺序   | 快照局部标识的规范排序序列                                                                                                                                        |
| tick / 时钟 | `tick` / `time_ms` / 输入命令游标                                                                                                                                 |
| 信号灯色    | **不入快照**：由 `time_ms` + 共享根 program + offset 派生（现行消费契约）                                                                                         |

- 快照局部标识（术语表）只在单个快照内稳定；恢复时经 `SharedIdentityIndex`
  解析 `StableId128` 并**新分配**句柄。原进程 `RouteHandle` / `VehicleHandle`
  不得成为恢复后身份。
- 一维数值全部为整数毫米 / 微米 / `u32` mm/s（ADR 0028），无浮点字段。

## 5. 保存与恢复合同

- **保存**只接受已进入活动聚合的 `CommittedNetworkSource`；working / candidate
  与 `EditableDiffBase` 不进入存档。保存发生在候选准备期间时仍只捕获旧聚合与
  对应快照（切换文档 §8）。
- **editable 来源恢复**：先从 committed `RoadEditingState` 重新编译，建立新
  session 的根与 exact LFCA base，再恢复快照；重编译失败、origin 不一致或缺
  `EditableDiffBase` 对应关系均失败关闭。
- **published 来源恢复**：经不透明 asset key + LFCA digest / length / revision
  的 `PublishedLfcaReference` 重新认证并读取 LFCA，构建共享修订后恢复快照；
  资产缺失、摘要错配失败关闭。
- 发布资产要启用道路编辑，必须另带 committed 道路状态，并以重编译 exact LFCA
  执行同修订 root / source / diff-base rebase；成功前不得启用编辑。
- 恢复成功后世界处于快照 `tick` 边界的一致状态；`install` 核对与
  `register_route` 重建沿现行消费契约执行。

## 6. 回放、checkpoint 与确定性状态摘要

- **输入命令序列由宿主记录并按序重放**（术语表：输入命令序列）；Runtime 不新增
  隐藏输入权威。回放 = 恢复快照（或 checkpoint）后按序重提交命令。
- **确定性状态摘要**：对逻辑状态按规范排序计算的版本化摘要（`u256` 级，算法与
  域分隔由 G2 落定并登记本文）。摘要对逻辑状态负责，不比对容器字节；同一逻辑
  状态在任何合法编码下摘要相等。
- **checkpoint**：回放序列中周期保存的快照锚点，与输入命令游标共同界定重放区间；
  缩短恢复与失同步定位区间。
- 失同步诊断边界：摘要不等即报告失同步与首个可定位区间，不做自动纠偏、不重启
  世界；处置由宿主决定。

## 7. 跨修订迁移入口

- 跨修订恢复必须由受信任 `NetworkRevisionCutoverDescriptor` 驱动，经 LFSD 与
  稳定标识执行**显式迁移**（切换文档 §3：封闭策略枚举 v1-a / v1-b，无宿主回调，
  不可映射实体整体失败关闭）。
- 旧密集序号不得直接解释为新修订实体；`SharedIdentityIndex` 完整重建稳定静态
  引用。同修订恢复不要求同 LFCA 字节：identity / constraint /
  execution-constraint versions 精确相等即可（ADR 0020 §12 语义，origin 形态
  按 ADR 0025 修订）。

## 8. 必测项（G2）

- 同修订 save → load exact oracle：逻辑状态（路线边序列、毫米游标、速度、状态、
  预约、live 序、tick）全等；句柄不比对（新分配）。
- checkpoint + 命令重放与连续步进摘要逐点相等；失同步注入能定位区间。
- 跨修订直移：直移成功 oracle 与不可映射实体整体失败关闭（含删边上有车、路线
  引用被删边、停车绑定失效三类）。
- 容器拒绝面：未知 `formatVersion`、损坏长度、越界基数、含禁绑字段（句柄/
  generation/密集序号/浮点一维值）全部失败关闭且零部分恢复。
- editable / published 两类来源恢复流程端到端；发布资产缺 committed 道路状态
  时编辑不可启用。
- 保存与候选准备并发时只捕获旧聚合；source rebase / diff-base binding 缺失或
  错配失败关闭。
