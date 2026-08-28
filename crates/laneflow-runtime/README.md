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

本 crate 不依赖 Spatial、compiler、Serde、文件系统或 `laneflow-core`。契约见
`docs/design/traffic-runtime-shared-consumption.md`。
