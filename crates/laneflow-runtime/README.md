# laneflow-runtime

目标交通运行时。`TrafficWorld` 安装完整 `Arc<SharedNetworkRevision>`，只分配每世界
可变状态与 1-worker 执行计划；热路径借用共享根静态 accessor，并读本世界已提交表。

安装的第五个必填参数为 `WorldPolicySelection`。宿主以 `Pinned(PolicyPin)` 指定策略
稳定标识；仅当整个共享根没有门、冲突区或参与者流时可用 `NotRequired`。不推断默认
策略、不读取业务日期，也没有安装后 setter。`policy()` 借用所选共享规则；
`policy_gap_profiles()` 保存本世界固定步长对应的 checked 间隙派生值。

LFRS 5 / runtime state 5 保存策略选择与 StableId，digest 7 纳入相同逻辑字段。
切换描述符 2 保持字段形状；跨修订保留策略身份、法域和法规版本，并按目标根重建派生间隙。
已有 `ConflictRuntimeUnavailable` 在冲突仲裁接入前继续保留。

生命周期命令只在两次 `step` 之间调用。`replace_completed_vehicle` 把 Completed
车辆一次提交为新的 Active 句柄；到终点保留 Completed，不进 pose、不占车道，占容量。
`despawn_vehicle` 原子移除任意合法 live status 并释放 route/parking binding；人口政策仍
不进入 `step`。

停车由私有 `ParkingRuntimeState` 持有唯一动态 authority。caller 以 tagged
`ParkingTarget::{ExplicitSpace, VirtualPool}` 精确选择资源，并经
`reserve_parking` / `cancel_parking` / `park_vehicle` / `leave_parking` /
`rebind_parking_route` / `spawn_parked_vehicle` 驱动 lifecycle。virtual capacity 不展开为
slot；virtual Parked 保留 live identity 但不进入 `active_order`、lane occupancy 或 pose。
`ParkingSpaceOrdinal` 等静态序号由本 crate 再导出，Adapter 不必直接依赖静态合同包。

交通观测由宿主显式打开 `ObservationExportSession` 并请求 full/delta；Runtime 只从
当前已提交车辆状态重算所选 LaneEdge 的整数聚合。无导出调用时不维护观测副本、
dirty journal 或后台任务，观测 session/基线也不进入 Runtime Snapshot。

动态成本 payload 与 Routing 算法仍由宿主拥有。Runtime 只从实际观测批次构造
`ObservationSetBinding`，签发 `RoutingAdmissionSession`，并由
`register_candidate_route` 校验世界/世代/修订/模型/有效窗后把 LaneEdge 稳定标识降低到
唯一路线编译入口。回放/恢复使用不含旧成本 provenance 的
`register_admitted_route`；两条路径与 direct `register_route` 共用路线槽和边出现项容量。

Runtime Snapshot 以 `capture_snapshot` 在固定步进边界冻结不可变逻辑状态，再由
`encode_lfrs` 离线编码。`restore_lfrs` 先核对 framing / file identifier / verifier
预算，再执行版本、v2 table 未知字段槽、来源、配置、标识、引用、排列、tagged 停车和值
不变量 lowering；所有路线
经 `register_admitted_route`，所有车辆/停车经共同运行时不变量入口在局部 world 中
重建，完全成功后才返回 `RestoredSnapshot` 及局部 ID 到新句柄映射。
`deterministic_state_digest` 按逻辑路线/车辆内容规范化，不依赖 LFRS 字节、进程句柄、
局部 ID、Published 审计地址或 worker 计划；容量、tick/时间/双游标与 live 顺序仍进入
SHA-256 状态身份。
检查点宿主可从不可变 `CapturedRoute` / `CapturedVehicle` 读取局部 ID 与逻辑字段，
恢复后从 `RestoredSnapshot::route_mappings` / `vehicle_mappings` 取得按局部 ID 排序的
完整新句柄表；检查点后的实体继续由宿主命令自带耐久 ID，Runtime 不接管输入日志。
快照预算证据按仪器拆分为 allocation ledger、DHAT restore heap high-water mark 与
未插桩 release wall-clock 三个独立 integration binaries，避免把累计分配量或插桩
计时误报为恢复峰值/产品门槛。

本 crate 不依赖 Spatial、compiler、Serde、文件系统或 `laneflow-core`。契约见
`docs/design/traffic-runtime-shared-consumption.md` 与
`docs/design/traffic-observation-and-routing-integration.md`、
`docs/design/traffic-runtime-snapshot.md` 与 `docs/design/parking-system.md`。
