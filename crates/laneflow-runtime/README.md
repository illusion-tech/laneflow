# laneflow-runtime

目标交通运行时。`TrafficWorld` 安装完整 `Arc<SharedNetworkRevision>`，只分配每世界
可变状态与 1-worker 执行计划；热路径借用共享根静态 accessor，并读本世界已提交表。

生命周期命令只在两次 `step` 之间调用。`replace_completed_vehicle` 把 Completed
车辆一次提交为新的 Active 句柄；到终点保留 Completed，不进 pose、不占车道，占容量。
不提供独立 `despawn`，也不把人口政策写入 `step`。`occupy_parking` 使用的
`ParkingSpaceOrdinal` 由本 crate 再导出，Adapter 不必直接依赖静态合同包。

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
预算，再执行版本、来源、配置、标识、引用、排列、停车和值不变量 lowering；所有路线
经 `register_admitted_route`，所有车辆/停车经共同运行时不变量入口在局部 world 中
重建，完全成功后才返回 `RestoredSnapshot` 及局部 ID 到新句柄映射。
`deterministic_state_digest` 按逻辑路线/车辆内容规范化，不依赖 LFRS 字节、进程句柄、
局部 ID、Published 审计地址或 worker 计划；容量、tick/时间/双游标与 live 顺序仍进入
SHA-256 状态身份。

本 crate 不依赖 Spatial、compiler、Serde、文件系统或 `laneflow-core`。契约见
`docs/design/traffic-runtime-shared-consumption.md` 与
`docs/design/traffic-observation-and-routing-integration.md`、
`docs/design/traffic-runtime-snapshot.md`。
