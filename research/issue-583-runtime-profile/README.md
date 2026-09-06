# 当前 TrafficWorld 性能画像

本报告交付 [#583](https://github.com/illusion-tech/laneflow/issues/583) 的有限研究切片。
生产基线为 `298da8a80a5c2edf6a1bbe521f59db77d1d8b76e`，已包含正式 Conflict/Policy、
#528 准入索引、#531 切换优化和 #580/#581 串行阶段/状态分区。
测试补丁以本目录同一提交为准，实际测量源文件的 Git blob 记录在
[`source-blobs.json`](evidence/source-blobs.json)。生产 API、行为、依赖和 wire 均未改变。

## 1. 测量边界与复现

2026-09-06，Windows 11 x64 build 29648，AMD Ryzen 9 9955HX（16 核 / 32 逻辑处理器），
Rust 1.98.0，release 默认配置，增量编译关闭，Windows 平衡电源计划。
完整机器信息见 [`environment.json`](evidence/environment.json)。本次未绑定 CPU 亲和性，
未锁定频率；进程顺序执行，正式测量前检查没有 Cargo/rustc/link 编译进程。
这是一台开发机上的研究窗口，不是产品硬件认证。

四个入口分别回答不同问题：

| 入口                         | 实际测量                                              | 边界                                                              |
| ---------------------------- | ----------------------------------------------------- | ----------------------------------------------------------------- |
| `runtime_profile_wall_clock` | 未插桩生产库的 `TrafficWorld::step` 墙钟              | 不含安装、编译路网、暖机、校验、快照/摘要、输出日志               |
| `runtime_profile_stages`     | 单元测试构建中的批次阶段墙钟、窗口结束后的 owner 账本 | 整个模块和调用点均为 `cfg(test)`；每批一次计时，不逐车计时        |
| `runtime_profile_allocation` | 独立二进制内 `StatsAlloc<System>` 的窗口分配流量      | 分配器不进入上述墙钟二进制；不把此处耗时当作延迟证据              |
| `cutover_scale_evidence`     | #531 现成 Prepare / pump / commit / 独立 digest 入口  | 独立摘要只计 digest，不含 capture；不是 commit 内部摘要的独占耗时 |

阶段计时是性能归因计时，不是 sampled CPU 或 exclusive self CPU。测试构建还包含
原有工作量计数及本次计时器成本；其中 `whole_step` 仅用于同一插桩轮次的分母，不能
替换未插桩延迟。短资源场景受计时器开销影响明显，仅用于确认资源路径被执行。

从仓库根运行，先完成编译，再串行执行测量；不要与其他任务的大编译或压力测试并行。
`--offline` 可在已有完整依赖缓存时追加，本次使用了该选项。

```powershell
cargo +1.98.0 test --release --locked -p laneflow-runtime --lib --test runtime_profile_evidence --test runtime_profile_allocation --test cutover_scale_evidence --no-run
cargo +1.98.0 test --release --locked -p laneflow-runtime --test runtime_profile_evidence runtime_profile_wall_clock -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --lib runtime_profile_stages -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test runtime_profile_allocation -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_scale_evidence -- --ignored --nocapture --test-threads=1
python research/issue-583-runtime-profile/analyze.py
```

本次实际直接运行 `--no-run` 生成的四个 exe，过滤参数与上述命令相同。
[`evidence/`](evidence/) 保存完整测试输出；[`analyze.py`](analyze.py) 只读取这些固定记录，
校验数量、各轮次/入口的最终摘要和日志字节、owner 总账、阶段调用次数及耗时包含关系，
输出 [`summary.json`](summary.json)。脚本不运行基准、不引入依赖或 CI 耗时阈值。

共享夹具和四个集成测试位于
[`src/kernel/tests/performance_profile`](../../crates/laneflow-runtime/src/kernel/tests/performance_profile/)，
Cargo 显式登记测试 target，保留 #531 的原命令。阶段模块从同级向下引用夹具，满足
现有源码审计对 `#[path]` 的限制，不扩展检查器规则。

## 2. 六个固定窗口

道路组复用 #531 编译器原生输入：256 条互不连接的 8 km 边、4.5 m 车辆、10 m
初始车头间距、零初速、15 m/s 限速；路线各含一条边。`edges` 指实际占用边数，
`routes=edges`，不是静态路网边总数。每组先暖机 32 拍，再暖机 8 拍，随后测量
64 拍，每拍 100 ms。每轮重新建立相同世界，不在此窗口触及路线终点。

日志组在两段暖机之间通过真实 `prepare_cross_revision_cutover` 武装日志，候选路网
仅把限速改为 16 m/s；测量期间不 pump/commit。测试显式使用 64 MiB 日志容量，
生产默认值不变。统计的写入字节包含武装后的 8 拍暖机和 64 拍测量；窗口前后检查
日志仍存在、未超预算，结束后 abandon 候选事务。

资源组复用编译器已受检的 `lfca-world-policies/full-spatial.lfca` 和现有预算测试窗口，
显式固定 Policy；共享根构建省略 Spatial。Waiting 为 1 辆活动车辆，暖机 1 拍后
保有 Waiting membership；Conflict 为两条 stream 上各 1 辆活动车辆，暖机 9 拍后
有 1 个 reservation，另一辆仍得到 NoGrant。两组测量 16 拍，每拍 4 ms；窗口前后
都验证上述资源状态。它们是资源语义探针，不是资源密集型城市规模负载。

所有窗口均为当前道路机动车 exact 路径：`N_individual=N_traffic_active=N_intent`，
即每辆活动车每拍都计算 controller intent；没有 reduced-rate、停车个体或退出窗口。
`N_presented=0`、`N_aggregate=0`，未运行 Spatial、Adapter、renderer 或宿主城市模拟。
三个入口在同一场景的所有最终状态摘要相同，日志组与对应未武装组摘要也相同。
这只验证给定命令流，不替代现有资源事件/失败原子性回归。

## 3. 当前结果

### 未插桩整步墙钟

单位 ms。每个场景独立重建 3 轮；p50/p95/p99 是各轮 nearest-rank 分位数的中位数，
max 是全部轮次中观察到的最大值。道路组每轮仅 64 拍，资源组仅 16 拍，p99 均落在
该轮最大样本上；资源组 p95 也如此。这不是长时间尾延迟估计，也不设置跨机器门禁。

| 场景                        |    p50 |    p95 |    p99 |    max |
| --------------------------- | -----: | -----: | -----: | -----: |
| 道路 1k / 256 边            | 0.3723 | 0.4789 | 0.5420 | 0.5907 |
| 道路 10k / 256 边           | 4.1084 | 4.7070 | 5.0986 | 7.0234 |
| 道路 10k / 16 边            | 4.3936 | 4.8044 | 5.5614 | 5.5627 |
| 道路 10k / 256 边，日志武装 | 5.5286 | 6.4871 | 6.7223 | 9.4890 |
| Waiting / 1 车              | 0.0021 | 0.0028 | 0.0028 | 0.0071 |
| Conflict / 2 车             | 0.0031 | 0.0033 | 0.0033 | 0.0034 |

三轮和短窗口仍有调度/频率噪声；集中与分散的细小差异不支持算法复杂度或稳定收益结论。
日志武装的额外成本需连同下面的提交归因和容量一起判断，不能只从两次独立计时之差推断。

### 批次阶段归因

下表是每个具名阶段相对同轮插桩 `whole_step` 的百分比，再取三轮中位数；各列
单独取中位数，合计不必恰好 100%。它们按现有函数边界命名，并不一一对应逻辑 P0～P8。

| 阶段                | 1k / 256 | 10k / 256 | 10k / 16 | 10k / 256，日志 |
| ------------------- | -------: | --------: | -------: | --------------: |
| `preflight`         |    3.12% |     2.53% |    2.46% |           2.49% |
| `occupancy`         |    7.48% |     6.79% |    6.05% |           6.17% |
| `waiting_prepare`   |    2.12% |     2.11% |    1.95% |           1.95% |
| `conflict_prepare`  |    3.81% |     3.62% |    3.49% |           3.32% |
| `motion_loop`       |   52.27% |    54.38% |   56.62% |          50.00% |
| `waiting_finalize`  |    0.76% |     1.19% |    1.10% |           0.96% |
| `signals`           |    0.02% |     0.00% |    0.00% |           0.00% |
| `conflict_finalize` |    7.37% |     7.35% |    6.89% |           6.51% |
| `waiting_outputs`   |   15.56% |    14.51% |   14.18% |          13.12% |
| `commit`            |    7.03% |     7.33% |    7.21% |          15.49% |

道路组未分配到子阶段的余量约 0.04%～0.26%，
Waiting/Conflict 小样本约为 26.0% / 21.2%；
余量包含计时器记账及阶段外调度。短资源阶段不能据此做细粒度热点排序。
`preflight` 含 Conflict invariant 校验；`commit` 含日志记录与成功状态发布。
`waiting_outputs` 包含 Gate 决策和转移事件整理，不等于纯 Waiting 求解成本。

### 分配流量与保留内存

六组测量窗口的 allocations、reallocations、allocated/deallocated/reallocated bytes
均为 **0**。这不含安装、暖机、prepare、结束后的 snapshot/digest 或 abandon，
不能推断其他命令、停车到达或所有未来 tick 都零分配。

| 场景                        | 源世界自有逻辑 backing B | 共享根 B |  工作区 B | 管理分区 B |
| --------------------------- | -----------------------: | -------: | --------: | ---------: |
| 道路 1k / 256 边            |                1,220,671 |   29,564 |   730,048 |          0 |
| 道路 10k / 256 边           |               11,315,551 |   29,564 | 7,282,048 |          0 |
| 道路 10k / 16 边            |               11,227,231 |   29,564 | 7,282,048 |          0 |
| 道路 10k / 256 边，日志武装 |               78,424,415 |   29,564 | 7,282,048 | 67,108,864 |
| Waiting / 1 车              |                   13,067 |    7,827 |     7,470 |          0 |
| Conflict / 2 车             |                   13,483 |    7,827 |     7,158 |          0 |

日志组写入 **56,162,664 B**，保留容量为显式测试预算 **67,108,864 B（64 MiB）**，
未溢出或静默失效。保留容量与写入长度分开报告，不能用写入长度替代内存账本。
其余五组未武装日志。所有轮次的五分区账本和共享根数值一致。

账本沿用 `WorldMemoryLedger`：只统计 owner 自有容器的逻辑 backing，HashMap 计
payload capacity，不计桶/分配器开销和固定结构体字节。共享根单列，不能每世界重复
相加。表中不含 Harness 持有的候选切换事务、其他共享根、编译器或进程保留内存；
它不是进程峰值或切换共存峰值。五分区完整明细见原始记录与 summary。

### 当前切换成本

复用 #531 入口，每组 3 次，下表为各字段中位数，单位 ms。online 在两个活动 tick
后各 pump 一次，pump 列为两次之和；drain 把日志排空留到 commit；paused 在
Prepare→commit 之间不步进。独立 digest 使用切换前已捕获的快照，不与其他列相加
作为阶段分解。全部 27 次成功切换到目标根并解除日志。

| 活动车辆 / 边 / 路线 | 模式   | Prepare |  pump |  commit | 独立 digest | 暂停窗口 |
| -------------------- | ------ | ------: | ----: | ------: | ----------: | -------: |
| 1000 / 16 / 16       | online |   1.332 | 0.163 |   3.550 |       1.002 |        — |
| 1000 / 256 / 256     | online |   1.001 | 0.168 |   2.695 |       0.900 |        — |
| 4000 / 16 / 16       | online |   9.762 | 0.647 |  18.045 |       4.263 |        — |
| 4000 / 256 / 256     | online |   2.512 | 0.634 |  11.377 |       4.241 |        — |
| 10000 / 16 / 16      | online |  52.228 | 1.601 | 143.502 |      42.962 |        — |
| 10000 / 256 / 256    | online |   7.338 | 1.618 |  99.266 |      45.581 |        — |
| 10000 / 256 / 10000  | online |  17.776 | 1.769 | 183.083 |      82.370 |        — |
| 10000 / 256 / 256    | drain  |   7.508 | 0.000 |  99.741 |      45.526 |        — |
| 10000 / 256 / 256    | paused |   7.136 | 0.000 | 102.510 |      42.578 |  109.597 |

集中分布与路线数量会改变切换成本；这里的单边 synthetic 路线没有覆盖复杂资源迁移。
本轮只建立当前绝对成本基线，不与 #531 的历史轮次组成受控 before/after，也不据此
设定切换 SLA。

## 4. 最多三个后续热点

1. **运动循环及其 leader 查询。** 它是当前道路组最大的具名阶段，但包含 profile/
   route/leader 查询、IIDM、stop/speed 约束、advance 和 next-state store。
   #216 负责 occupancy rebuild 与 leader/route-distance 查询；#217 负责扣除该部分后
   的 controller、约束、advance/store。下一轮先做一组可归因对照，再决定是否优化，
   不预先承诺 SIMD、SoA、halo 或至少两种候选。
2. **无 Waiting 成员时的输出整理。** `finalize_waiting_outputs` 仍遍历 updates 和
   live order、检查 non-entry Gate，并调用 `stage_transition_events`。具名阶段包含
   转移事件整理，所以其比例不能全归因于 Waiting 判定。值得单独判断稀疏资源路径能否
   省去部分工作；任何优化须保留非 Waiting Gate 和转移事件。只登记此候选，不在本次
   实现快捷路径或为所有资源组合建立矩阵。
3. **管理路径中的日志与最终提交。** 武装日志增加提交成本并保留显式容量；切换的
   commit 和独立摘要仍有明显绝对成本。后续应按实际切换停顿预算选择是否继续拆解，
   不能从独立摘要时间直接推出可省去的 commit 时间，也不能为了画像重新设计事务。

#212 的旧 CoreWorld occupancy/leader 35.4%、longitudinal 56%、proposal/store 91%
属于不同实现、100k 输入及不同阶段/采样定义。当前 `motion_loop` 内含 leader 查询，
`occupancy` 仅计索引重建；直接比较百分比或声称热点已消失都会失真。
正式 Conflict 已在基线中，不保留“main / + #284”双轨研究。

## 5. 交付与后继边界

- #216 / #217 以此处生产基线与固定窗口为新起点。旧实验和开工记录保留为历史，
  不自动授权在当前架构上恢复实现；候选切片开工前重新限定范围和 G1/G2。
- #220 消费已合入的串行 P0～P8 合同和这份成本画像。它仍需要 #216/#217 的实际
  优化/局部性结论及产品 workload/budget 输入；本报告未提供 worker-count、halo、
  barrier 或多核加速证据，不能据此宣布并行方案就绪。
- 本次不完成 #304、#542、#544，不宣称单城市 10k/100k Product Pass，也没有测量
  100k、复杂多边路线、资源密集路网、停车周转或宿主帧预算。
- #583 的终点是这六组有限窗口、现成切换入口、原始记录和三个候选的排序。新输入、
  深层 CPU 采样、生产优化和更多错误组合交由后续明确立项的切片。
