# 从源码接入无窗口交通运行时

**文档状态**: Active
**最后更新**: 2026-11-11

本文是操作性接入教程。它引用仓库里的真实示例，不替代
[架构说明](../architecture.md)、[当前能力与限制](../reference/capabilities.md)
或任何 design / ADR。

## 适用范围

这份教程面向已经检出本仓库源码、使用 Rust 1.99.1、只跑道路机动车、并且不需要窗口的宿主。
当前工作区 crate 都是 `0.0.0` 且 `publish = false`，只支持从源码集成。这里不承诺可以从
registry 安装，也不把仓库相对路径写成安装后仍然有效。外部 registry 消费示例不在本教程范围内。

产品定位和六个 Bevy 示例见[仓库首页](../../README.md)。使用条件、拒绝情形和证据范围见
[当前能力与限制](../reference/capabilities.md)。首次 Alpha 的范围、稳定性、平台和
公开支持矩阵不在这里写。

可移植对象的现行版本是 LFCA 5 / LFSM 4 / LFSD 4 / LFCP 2。下面用到的文件名
`v0.2-signalized-corridor` 是检入场景制品名，不是格式版本。

## 先运行

唯一完整程序是
[`crates/laneflow-runtime/examples/headless.rs`](../../crates/laneflow-runtime/examples/headless.rs)。
在仓库根目录执行：

```powershell
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
```

程序通过 `include_bytes!` / `include_str!` 读取这两份检入资产，不接受命令行路径：

- `examples/data/v0.2-signalized-corridor.lfca`
- `examples/data/v0.2-signalized-corridor.catalog.toml`

行为测试和这份程序是同一个文件。Cargo 把它登记成集成测试 `headless_smoke`，nextest 会执行其中的断言，不另写一套装配：

```powershell
cargo +1.99.1 test --locked -p laneflow-runtime --test headless_smoke
```

下面是上述 `run` 在本仓库当前资产上的标准输出。数字来自这次实跑，不是估算。

```text
asset=checked-in:v0.2-signalized-corridor
catalog_version=0.4
spatial=omitted
policy=pinned
network_revision=2de4820d35bee0ef71374cae49036fbef75155a2a587cbd13b351d5f7655ad27
source_revision_matches=true
portal=portal-main-west
lane_index=0
slot=slot-main-west-lane-0-000
route=route-main-west-near-left
route_edges=3
fixed_delta_time_ms=16
worker_count=1
rejected_step=DeltaMismatch
time_after_rejected_step_ms=0
rejected_route=EmptySequence
live_vehicles=1
command_cursor_unchanged=true
steps=64
time_ms=1024
status=active
edge_before=0
progress_before_mm=6500
edge_after=0
progress_after_mm=23567
speed_mm_s=16667
advanced=true
rejected_observation=FirstExportMustBeFull
session_after_rejected_delta=empty
observation=full
observation_base=none
observation_revision=2de4820d35bee0ef71374cae49036fbef75155a2a587cbd13b351d5f7655ad27
observation_tick=64
observation_world_matches=true
observation_entries=66
occupied_length_mm=4500
front_vehicle_count=1
```

`headless.rs` 这个文件名只表示不打开窗口。它不是历史配置档 `traffic-headless-v1`。
无空间保留数据的构建调用是 `SpatialBuildOption::Omit`。

这个 example 用到 `laneflow-runtime` 的 dev-dependencies，其中包括 `laneflow-scenario` 和
`toml`。同一组 dev-dependencies 已经为其他测试打开 `laneflow-static-network` 的
`attestation-fixtures`。本示例不调用该测试模块，也不能据此说整条 dev 依赖图没有测试 feature。

## 管线

宿主自己完成资产准入。编译器在更上游把编制来源做成可移植对象；交通运行时不解析 JSON，
也不把 LFCA 字节当作安装格式。本示例从已经检入的 LFCA 开始，不在程序里调用编译器。

顺序是：

1. `check_canonical_network_input` 得到受检 LFCA（`CheckedCanonicalNetworkInput`）。
2. `build_shared_network_revision` 得到进程内的 `SharedNetworkRevision`。
3. `TrafficWorld::install` 安装这根修订，并带上宿主点名的已提交来源和策略选择。
4. 宿主 `register_route`，再 `spawn_vehicle`。
5. 以世界配置里的固定步长调用 `step`。
6. 打开观测 session，第一次导出必须是 Full。

共享修订不是文件、ABI 或存档。路线只存在于这个世界里，不写回静态制品。

## 示例怎么接

程序正文在
[`headless.rs`](../../crates/laneflow-runtime/examples/headless.rs)。
下面只解释各段在做什么。

### 格式与构建预算，以及 Spatial Omit

检查 LFCA 时传入 `FormatLimits::HARD`。这是本仓库示例使用的格式天花板，不是推荐给任意
不受信路网的安全预算。接收方应按自己的部署收紧对象字节数等调用方预算。格式天花板和
接收方预算的差别见
[可移植规范制品 §7](../design/portable-canonical-artifact.md#7-格式安全天花板)。

构建预算是另外两层调用方上限：保留 `64 * 1_024 * 1_024` 字节，暂存
`16 * 1_024 * 1_024` 字节。它们只覆盖这份检入走廊，同样不是任意路网的安全预算。

`SpatialBuildOption::Omit` 在 LFCA 带有空间载荷时仍检查几何连通，但不把 Spatial
component 留在共享修订里。实跑结果是 `spatial=omitted`。本示例源码不导入、不调用
Spatial 或 Bevy。`laneflow-runtime` 的 dev-dependencies 里已经有 `laneflow-spatial`，
因此 `cargo run --example headless` 仍会编译这个 crate；那不是对 Spatial API 的使用。
需要位姿时再单独绑定，见
[能力页 · Spatial](../reference/capabilities.md#spatial)。

### 来源绑定

`asset` 那一行的 `checked-in:v0.2-signalized-corridor` 是宿主资产键，不是安装后的文件路径。
`PublishedLfcaReference` 记下这次受检字节的 digest、精确长度和 `NetworkRevisionId`。

安装发布来源时，严格核对的是 `NetworkRevisionId` 与共享修订 origin 一致。实跑
`source_revision_matches=true` 只表示这两份修订标识相同。digest 或 length 不同只供来源审计，
不是拒绝条件。同修订重发布可以带不同的 digest / length。
不要写成这三个字段必须全部相等。编辑来源不同：它的差异基线必须与 origin 的 LFCA
digest / length 对应。本示例走的是发布来源。

合同见
[运行时快照 §5](../design/traffic-runtime-snapshot.md#5-保存与恢复)。

### 四容量与固定步长

`WorldConfig` 的四个容量和固定步长在安装时写死：

| 字段                                 | 本示例的值 | 含义                               |
| ------------------------------------ | ---------- | ---------------------------------- |
| `vehicle_capacity`                   | 1          | 只生成一辆车                       |
| `route_capacity`                     | 1          | 只注册一条路线                     |
| `route_edge_occurrence_capacity`     | 3          | 等于这条路线的边出现记录数         |
| `route_conflict_occurrence_capacity` | 0          | 等于这条路线注册时的冲突出现记录数 |
| `fixed_delta_time_ms`                | 16         | 与检入走廊的信号程序一致           |

这组数字只够这一条路线和这一辆车。冲突容量为 0 只说明 `route-main-west-near-left`
这次注册没有冲突出现记录，不说明整网没有门、冲突区或参与者流。策略仍然必须 Pinned。

`fixed_delta_time_ms` 的合法范围是 `4..=1000`。不要把走廊的 16 毫秒改成 100 毫秒再指望
信号相位仍然对齐：相位时长短于步长，或不是步长的正整数倍，安装会拒绝。

### worker

`ExecutionConfig::new(NonZeroU32::MIN)` 是 1 个 worker，在调用线程上计算，不创建辅助线程。
输出里的 `worker_count=1` 就是这个值。当前后端接受的 worker 数有上限，超出会在安装时拒绝，
不会静默降下来。worker 数不进入快照或逻辑摘要。并行范围见
[能力页 · 单世界并行](../reference/capabilities.md#单世界并行) 和
[执行配置](../design/traffic-runtime-execution-config.md)。

### Pinned 策略

走廊 catalog 0.4 的 `policy_selection` 是显式 `pinned`。`bind` 把它解析成
`WorldPolicySelection::Pinned`，再原样交给 `TrafficWorld::install`。没有默认策略，也没有
安装后的 setter。

`WorldPolicySelection::NotRequired` 只有在整个共享修订都没有 `ManeuverGate`、
`ConflictZone` 和 `ParticipantStream` 时才合法。这份走廊不满足该条件。程序若没有得到
Pinned，会直接返回错误，而不是装一个不需要策略的世界。

### 路线与车辆

路线和入口按 catalog 对齐，不按边长猜：

- 取 catalog 的第一个 portal，实跑是 `portal-main-west`。
- 取该 portal 的第一条车道，`lane_index=0`。
- 取该车道的第一条路线选项，实跑是 `route-main-west-near-left`，共 3 条边。
- 取该车道的 `entry_spawn_slot_id`。实跑是 `slot-main-west-lane-0-000`，进度 6500 毫米。
- 该槽所在边必须是这条路线的起点出现记录。不是的话程序返回错误。

然后各调用一次 `register_route` 和 `spawn_vehicle`。入口使用 `with_open_entrance()`，
因为槽在路线起点所在的 portal 上。初速是车型期望速度和这条边限速的较小值。
实跑后车辆仍为 `active`，速度 16667 毫米每秒。

不启动 `CorridorPopulationController`，也不调用 `place_existing_active_vehicle` 或
`replace_existing_completed_vehicle`。人口、出发计划和路线选择策略留在宿主。catalog 的
`bind` 只负责把编制字符串换成这条修订上的序号、策略和入口，不在 `step` 里做回流。

### 固定步进

先用 15 毫秒调用一次 `step`。这和世界的 16 毫秒不一致，返回
`StepError::DeltaMismatch`，`time_ms` 与 `tick_index` 都保持 0。

然后用 16 毫秒连续步进 64 次。已提交时间变为 1024 毫秒，等于 `64 × 16`。车辆仍在路线
下标 0，进度从 6500 毫米增加到 23567 毫米。

### 已提交状态与 Full 观测

新的 `ObservationExportSession` 第一次导出必须是 Full，不能先 Delta。程序先请求 Delta，
得到 `ObservationError::FirstExportMustBeFull`，session 的 `previous()` 仍是 `None`
（输出里的 `session_after_rejected_delta=empty`）。
随后的 Full 才成功。

这次 Full 来自当前世界：`observation_revision` 与 `network_revision` 相同，`observation_tick`
是 64，`observation_world_matches=true`，并且 Full 没有 delta 基线（`observation_base=none`）。
`ObservationSelection::AllLaneEdges` 会列出所选的每一条边，包括计数为零的行，所以行数是 66，
不是 1。占用长度合计 4500 毫米，前保险杠计数合计 1，对上这一辆仍在路上的车。

观测 session 不进入运行时快照。没有导出调用时，运行时也不维护观测副本。

## 失败定位

三类失败不要混成一种“调用失败”：

| 种类            | 本示例里的样子                                 | 提交了什么                 | 之后还能用这个世界吗                       |
| --------------- | ---------------------------------------------- | -------------------------- | ------------------------------------------ |
| 可恢复 `Result` | 15 毫秒步进、首次 Delta                        | 时间不前进；观测基线不写   | 能。改成 16 毫秒和 Full 后，同一次运行继续 |
| 业务拒绝        | 空边序列 `register_route` 得到 `EmptySequence` | 不增加路线，命令游标不前进 | 能。已经生成的那一辆车还在                 |
| 执行 panic      | 本示例不触发                                   | 没有可恢复的交通提交       | 不能。世界已经失效                         |

可恢复 `Result` 指步进、观测这类操作返回错误时，已提交边界保持在失败前。业务拒绝指
路线、生成或安装因输入不合法而拒绝：安装失败不留下半个世界，生成失败不留下半辆车。
空路线只是其中一种，重叠、超速、容量不足也属于这一类，具体变体见 `RouteError`、
`SpawnError` 和 `InstallError`。

执行 panic 不是 `StepError`。在支持 unwind 的构建里，运行时先收拢已分发任务和借用，
再把 panic 传出来；这个世界退出可用状态。宿主即使捕获了 panic，也不能继续查询、发命令
或做管理操作，只能销毁并重建。`panic=abort` 则按进程终止处理。见
[并行执行 §4.3](../design/traffic-runtime-parallel-execution.md#43-join取消与失败清理)。

## 当前限制

- 交通运行时不生成出行需求，也不替宿主选择路线。本示例只注册 catalog 里已经写明的一条边序列。
- 固定预构造换道只执行已经编制好的过渡。它不是任意位置变道。见
  [能力页 · 固定预构造换道](../reference/capabilities.md#固定预构造换道)。
- 上面的 64 步只确认时间和位移。它不是性能认证，也不能外推到其他路网或车辆数。

## 延伸阅读

- 快照与恢复：[运行时快照](../design/traffic-runtime-snapshot.md)
- 运行中更换修订：[修订切换](../design/traffic-runtime-revision-cutover.md)
- 可选位姿：[空间几何](../design/spatial-geometry.md)、[`laneflow-spatial`](../../crates/laneflow-spatial/README.md)
- 窗口和 Bevy 宿主：[`laneflow-bevy`](../../crates/laneflow-bevy/README.md)
- 编译器、线格式和共享修订：[`laneflow-compiler`](../../crates/laneflow-compiler/README.md)、[`laneflow-format`](../../crates/laneflow-format/README.md)、[`laneflow-static-network`](../../crates/laneflow-static-network/README.md)、[`laneflow-static-contract`](../../crates/laneflow-static-contract/README.md)
- catalog 与 `bind`：[`laneflow-scenario`](../../crates/laneflow-scenario/README.md)
- 运行时入口：[`laneflow-runtime`](../../crates/laneflow-runtime/README.md)
- 回到[当前能力与限制](../reference/capabilities.md)和[文档首页](../README.md)
