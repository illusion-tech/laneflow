# #679 长路线查询成本

研究基线：`72102fabb2cda5ebadde52dc1260c8402762efda`。本切片使用 Rust 测试入口，
不增加运行时 API、缓存或策略权威。[测量结果与决定](results.md)已完成。

## 当前问题

`signal_stop_distance` 已使用本拍 `MotionReach`，而非 leader 查询窗。远处首门
超出上界即退出。旧的逐门解释假设只适用于显式 `None` 回放，不能冒充当前整步成本。
Waiting、Conflict、Motion 与事件投影仍存在不同键的出现项二分；需要分别计数。

## 冻结矩阵

- 固定三个 8 km 物理边组成循环、512 辆车、单 worker、100 ms 步长；车辆始终
  位于首边，16 拍暖机、128 拍观察。本切片不涉及城市吞吐或正式性能认证。
- 路线重复 8 / 128 / 2048 圈，分别含 16 / 256 / 4096 受控 Gate 与
  8 / 128 / 2048 个 Waiting 出现项；全绿和第二门红灯两种固定相位。
- 三轮普通 Release 整步墙钟，路线顺序正/反/正；路线登记单独计时。
  静态路网编译、车辆生成、输出、状态核验不进入 step 时间。
- 独立测试构建记录 Gate 解释次数、二分调用次数和世界持有内存；该构建含
  测试分配器和探针，不用于报告普通构建延迟。site 为基线文件名与行号。
- 现有 Waiting membership / Conflict reservation + retry 小夹具补充近门与持有
  资源路径；不能把它们称为大规模城市工作负载或长 Conflict 路线矩阵。
- 显式无上界信号查询只说明历史算法机制；不作为当前运行时基线。

## 原型与正确性边界

研究原型为每个 route hop 保存六个纯拓扑定位下标（Gate `<` / `<=`、Waiting
entry / release、Conflict admission、Maneuver exit）。每行 24 字节，额外包含
路线终点哨兵；构建和重建同价。比较三轮离散查询回放，逐 hop 核验索引等价。
它不缓存 class、policy、signal、grant、reservation 或 Waiting membership，
也不进入实际 step；回放收益不能外推为整步收益。

若进入实现，应由编译路线所有者持有索引。注册、恢复、切换重编译时重建，随
完整路线句柄代次销毁；跨阶段复用还必须区分 old/next cursor、边界回退一 hop、
已持有资源和同拍 grant，不能把不同搜索键合并成一项。限制门结果缓存则必须
额外包含 class、policy 和 signal 状态，其当前价值先由扫描计数判断。

运行完整 Runtime 单元测试，包含换路线、恢复、切换、class 漂移拒绝、失败原子性、
重试、完整句柄代次和资源守恒回归。新增循环路线信号测试比较全绿、红灯、黄灯、
每个 hop 的起点/近门/边界位置与无上界查询在本拍可达范围内的结果。

## 运行

在干净且已推送的 source commit 构建，停止构建后按顺序直接运行两个二进制。
保存 source/tree、Cargo.lock、rustc、二进制和原始 stdout 的 SHA-256；记录前后
工作树状态及竞争进程。原始记录生成于忽略目录，完成后归档到本目录。

```text
cargo +1.98.0 test -p laneflow-runtime --release --test route_query_evidence --locked --no-run
cargo +1.98.0 test -p laneflow-runtime --release --lib --features placement-fixtures --locked --no-run
<route_query_evidence.exe> --exact route_query_wall --ignored --nocapture --test-threads=1
<laneflow_runtime.exe> --exact kernel::route_query_research::route_query_diagnostic --ignored --nocapture --test-threads=1
cargo +1.98.0 test -p laneflow-runtime --test route_query_evidence --locked -- --nocapture
```

结束条件：获得三轮普通墙钟、完整诊断与资源/生命周期回归结果，说明远门扫描是否
仍存在、二分热点与候选额外内存；给出具体采用/放弃/后续实验决策。无需以 100k
长窗作为本研究或后续内部优化合入前提。
