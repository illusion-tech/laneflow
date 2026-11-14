# laneflow-runtime

LaneFlow 交通运行时。`TrafficWorld` 是当前唯一的可运行交通世界，现只实现道路机动车。

## 职责与边界

世界安装一整份 `Arc<SharedNetworkRevision>`，只分配这个世界自己的可变状态。热路径借用共享修订里的静态数据，并读取本世界已经提交的表。路线、车辆、信号遵守、等待区与冲突区裁决、停车和固定步进都在这里。出行需求、出发计划和路线选择策略不在 `step` 里产生。

本 crate 的生产依赖不包括 Spatial、编译器、Serde、文件系统或引擎。位姿采样由宿主另接 `laneflow-spatial`。展示和实体生命周期由 Adapter 负责。

## 最小使用入口

从源码检出仓库后，直接运行无窗口示例：

```powershell
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
```

该程序检查检入走廊 LFCA、省略 Spatial 保留数据、在安装前认证共享修订（只有 `Pass` 才安装）、按 catalog 的 portal 入口注册一条路线并生成一辆车，再以 16 毫秒固定步进，最后导出 Full 观测。它不启动持续人口控制器。步骤、实跑输出和失败分类见[从源码接入无窗口交通运行时][source-integration]。

安装入口是 `TrafficWorld::install`。调用方同时给出 `WorldConfig`、`ExecutionConfig`、已提交来源、世界身份和 `WorldPolicySelection`。

## 主要约束与限制

- 发布来源在安装时严格核对 `NetworkRevisionId`。digest / length 不同只用于来源审计，不是拒绝条件。编辑来源的差异基线则必须与 origin 的 LFCA digest / length 对应。
- `WorldPolicySelection::NotRequired` 仅当整个共享修订没有 `ManeuverGate`、`ConflictZone` 和 `ParticipantStream` 时合法。走廊 catalog 必须使用 `Pinned`。没有默认策略，也没有安装后的 setter。
- `fixed_delta_time_ms` 必须落在 `4..=1000`，并且与信号相位对齐。检入走廊使用 16 毫秒。
- 四个容量是车辆、路线、路线边出现记录和路线冲突出现记录。它们进入快照和逻辑摘要。worker 数另由 `ExecutionConfig` 指定，不进入快照。
- 新观测 session 的第一次导出必须是 Full。观测从当前已提交状态计算；失败不推进 session。
- 生命周期命令只在两次 `step` 之间调用。命令失败不留下半条路线或半辆车。
- 执行 panic 不是可恢复的 `StepError`。世界因此失效后，只能销毁并重建，不能继续查询或发命令。
- dev-dependencies 已经启用 `laneflow-static-network` 的 `attestation-fixtures`，供其他测试使用。无窗口示例不调用这些测试入口。

固定预构造换道、停车、快照和修订切换的拒绝条件见能力页与对应合同，不在这里列账。

## 示例与验证

```powershell
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
cargo +1.99.1 test --locked -p laneflow-runtime --test headless_smoke
```

`headless_smoke` 与上面的示例是同一个 `examples/headless.rs`，由集成测试目标执行其中的断言。它检查受检构建、安装前修订认证通过且认证失败时不安装、Pinned 策略、来源修订、一条路线与一辆车、64 步后的精确时间、车辆位移，以及错误步长和首次 Delta 都不提交。

## 深入阅读

- [从源码接入无窗口交通运行时][source-integration]
- [当前能力与限制][capabilities]
- [共享静态路网消费][shared-consumption]
- [执行配置][execution-config]
- [并行执行][parallel-execution]
- [观测与 Routing][routing-integration]
- [运行时快照][runtime-snapshot]
- [修订切换][revision-cutover]

[source-integration]: ../../docs/guides/source-integration.md
[capabilities]: ../../docs/reference/capabilities.md
[shared-consumption]: ../../docs/design/traffic-runtime-shared-consumption.md
[execution-config]: ../../docs/design/traffic-runtime-execution-config.md
[parallel-execution]: ../../docs/design/traffic-runtime-parallel-execution.md
[routing-integration]: ../../docs/design/traffic-observation-and-routing-integration.md
[runtime-snapshot]: ../../docs/design/traffic-runtime-snapshot.md
[revision-cutover]: ../../docs/design/traffic-runtime-revision-cutover.md
