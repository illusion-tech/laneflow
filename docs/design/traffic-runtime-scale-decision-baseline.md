# Traffic Runtime 城市规模扩展决策基线

**文档状态**: Active<br>
**最后更新**: 2026-11-09<br>
**适用范围**: 现行 `TrafficWorld` 道路机动车路径的城市规模扩展决策与接口可扩展性复核<br>
**关联议题**: #72、#1336<br>
**关联文档**:

- [`core-runtime-performance-baseline.md`][core-runtime-performance-baseline]
- [`traffic-runtime-parallel-execution.md`][traffic-runtime-parallel-execution]
- [`traffic-runtime-snapshot.md`][traffic-runtime-snapshot]
- [`traffic-runtime-revision-cutover.md`][traffic-runtime-revision-cutover]
- [`../adr/0021-traffic-infrastructure-and-host-boundary.md`][0021-traffic-infrastructure-and-host-boundary]

## 1. 目的与边界

本文取代已退役的 [`core-runtime-scalability-audit.md`][core-runtime-scalability-audit]
作为城市规模扩展的决策入口，记录现行 `TrafficWorld` 接口的可扩展性复核、证据边界和
各方向的启动条件。复核基线为 `main@d819c151`。

本文不选择生产分区、调度、降频或聚合方案，不改变 Runtime API、数据格式或 Adapter
协议。产品目标与测量协议以
[`core-runtime-performance-baseline.md`][core-runtime-performance-baseline] 为准。

## 2. 证据边界

| 记录                        | 可以说明                                                         | 不能说明                                                        |
| --------------------------- | ---------------------------------------------------------------- | --------------------------------------------------------------- |
| #583 性能画像（`298da8a8`） | 一万档各具名阶段的相对占比                                       | 十万档成本；`motion_loop` 内部的控制意图占比                    |
| #707 认证（`10c625ae`）     | 精确单世界并行曾使十万活动车辆在 16 workers 的研究机器上达到预算 | 其他机器、更少 workers、当前主分支；P4 未单独计时；切换场景未测 |
| #305 综合结论（`c0045f8f`） | performance、resource 均为未认证                                 | 通过或失败                                                      |
| #1166 / #1233               | 十万表现、预热后分配、W4 突发的已知超标现象                      | 根因；当前主分支的数值                                          |

#707 记录的协调线程串行时间占步进内时间的 34%–43%，最大热点是 Conflict 准备的串行段。
这说明当时的关键路径包含大量串行段，也不构成采用空间分区的依据。

## 3. 当前接口复核

| 审计面         | 结论                 | 依据                                                                                                                                                                                                    | 承接               |
| -------------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| 步进与整拍提交 | 已满足               | `step` 不暴露 worker、分块或布局；ADR 0021 禁止不同分区读取不同逻辑时点                                                                                                                                 | 分区原型须证明等价 |
| 事件与决策读取 | 需要补证据           | `latest_transition_events`、`latest_waiting_decisions`、`latest_conflict_decisions`、`waiting_zone_members`、`live_vehicles` 返回连续切片（`&[T]`），要求提交后存在单一规范序列；分区时的归并成本须计入 | 分区研究           |
| 句柄与物理位置 | 已满足               | 句柄只含槽位下标与代次，经目录解析物理位置；不编码 worker、分区或物理行；恢复重新签发并提供映射                                                                                                         | —                  |
| 位姿读取       | 已满足（接口）       | `committed_pose_source` 按句柄读取，`extract_selected_committed_pose_batch` 按选择集合提取；成本随世界规模增长的原因未归因                                                                              | #1274              |
| 边观测导出     | 需要补证据           | `ExplicitLaneEdges` 与 `Delta` 的接口合同已存在，但每次导出遍历两遍全部存活车辆，选择集合与增量模式都不减少该扫描                                                                                       | #1337              |
| 生命周期命令   | 需要补证据           | 只有单车命令，没有批量入口；是否需要批量合同取决于 #1275 的分段结果                                                                                                                                     | #1275              |
| 快照与恢复     | **存在实际阻碍**     | 执行配置不进入快照，符合可重建边界；但含活动过渡或预约的世界不能保存（#1217 结构化拒绝）                                                                                                                | #1200              |
| 修订切换       | 需要补证据           | 已准备并晋升目标执行计划；含过渡资源的跨修订迁移结构化拒绝；候选计划与切换共存峰值未测                                                                                                                  | #1200；资源认证    |
| 资源预算       | 需要补证据           | `VehicleStore`、空闲槽位、`live_order`、`active_order` 按配置容量预留，少量 Active 不会使这部分内存同比下降；容量进入快照与逻辑摘要，改变容量即改变世界身份                                             | §5                 |
| 多执行域       | 已满足（未过早冻结） | `VehicleState` 字段私有、经访问器暴露，未成为公共基类；`ParticipantClass` 只作静态准入分类                                                                                                              | #236               |

### 3.1 降频状态的归属规则

精确执行的 worker、任务范围和物理布局是可重建执行状态，不进入快照。降频若使用旧意图
并因此影响未来轨迹，其缓存内容、更新相位或失效状态就属于仿真语义，不能按“派生即不存”
排除，必须逐项证明可重建，否则进入快照与摘要。

## 4. 方向裁定

| 方向                                | 裁定                 | 启动条件                                                                                               |
| ----------------------------------- | -------------------- | ------------------------------------------------------------------------------------------------------ |
| 串行段优化（Conflict 准备、提交等） | 候选，优先于架构变更 | 当前提交上的归因确认其仍位于关键路径                                                                   |
| P4 组件并行（#701）                 | 条件触发             | P4 取得独立计时，且联合裁决占足够关键路径                                                              |
| 空间分区                            | 条件触发             | 查询局部性、负载失衡或全局数据访问被证明为主要成本；评估须计入分区构建、邻区读取、迁移、归约和负载失衡 |
| 控制意图降频                        | 条件触发             | 满足性能基线 §10 第 4 项，并先取得控制意图独立归因；#218 已关闭                                        |
| 聚合／中观                          | 暂不采用             | 满足性能基线 §10 第 6 项；减少表现量不属于降低交通保真度，不作为其依据                                 |
| 第二执行域（#236）                  | 条件触发             | 选定明确使用场景和最小行为范围                                                                         |

路线顺序遵循 ADR 0021。无新增产品需求时，先解决十万强个体路径的实际成本，再决定百万档
需要什么模型。

## 5. 资源预算口径

预算至少分别报告：

- 活动与非活动个体；
- 配置容量与实际使用量；
- 路线边出现记录与冲突出现记录；
- 常驻索引、暂存区与每拍实际触达字节；
- worker 线程资源；
- 保存恢复与修订切换的共存峰值。

[core-runtime-performance-baseline]: core-runtime-performance-baseline.md
[traffic-runtime-parallel-execution]: traffic-runtime-parallel-execution.md
[traffic-runtime-snapshot]: traffic-runtime-snapshot.md
[traffic-runtime-revision-cutover]: traffic-runtime-revision-cutover.md
[0021-traffic-infrastructure-and-host-boundary]: ../adr/0021-traffic-infrastructure-and-host-boundary.md
[core-runtime-scalability-audit]: core-runtime-scalability-audit.md
