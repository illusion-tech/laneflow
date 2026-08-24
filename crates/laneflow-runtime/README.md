# laneflow-runtime

目标交通运行时。`TrafficWorld` 安装完整 `Arc<SharedNetworkRevision>`，只分配每世界
可变状态与 1-worker 执行计划；热路径借用共享根静态 accessor，并读本世界已提交表。

生命周期命令只在两次 `step` 之间调用。`replace_completed_vehicle` 把 Completed
车辆一次提交为新的 Active 句柄；到终点保留 Completed，不进 pose、不占车道，占容量。
不提供独立 `despawn`，也不把人口政策写入 `step`。

本 crate 不依赖 Spatial、compiler、Serde、文件系统或 `laneflow-core`。契约见
`docs/design/traffic-runtime-shared-consumption.md`。
