# laneflow-static-network

`laneflow-static-network` 把 `laneflow-format` 提供的受检 LFCA 能力构建为进程内、
不可变、可由多个 world 共享的 `SharedNetworkRevision`。

本 crate 不定义静态镜像文件、ABI、缓存或存档格式，不依赖 compiler、当前 Core/Data、
Adapter 或文件系统。共享根拥有 Traffic、Identity、Planning Hints 与可选 Spatial
component；调用方只独立保留 `Arc<SharedNetworkRevision>`。

当前实现状态以 #300 Gate Ledger 为准。
