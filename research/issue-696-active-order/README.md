# 停车命令活动顺序增量维护

关联 [#696](https://github.com/illusion-tech/laneflow/issues/696)。
生产基线为 `4194789d7077bd8fadf55f97abe14e37c46ce987`；测量入口先独立提交为
`511fc23d1c42b71bc72978949c0f6271ba1c527a`，此时尚无生产优化。
候选实现为 `6a98ec5d765c310de7b0e6c9c92ed86f18512598`。两侧使用完全相同的
`active_order_evidence` 集成测试，链接不带 `cfg(test)` 工作量计数的生产库。

## 结论与实际 A/B

九次运行共6048个测量批次、231552次API计时；全部42场景在九次运行中的完整
语义摘要一致。保留该缓存：大量Parked、连续成功时明确减少live扫描，128 Active /
4096 Parked / 64次全成功的批次p50下降53.1%，1024 Active时下降20.7%。
这不是所有负载都加速；一次冷成功有额外建表成本，小幅变化还受到A/A波动影响。

下表均为批次API耗时和，单位µs；变化率为 `(B/A - 1) × 100%`，负数更快。
完整42场景、所有冷/热/拒绝分类及逐轮结果见 [results.csv](results.csv)，其时长单位
为ns，包含p50/p95/p99/max、样本数和摘要。

| 形状       | Active / Parked | API数 / 成功率 | A p50   | B p50   | A/B变化 | A/A变化 |
| ---------- | --------------- | -------------- | ------- | ------- | ------- | ------- |
| normal     | 128 / 64        | 64 / 100%      | 451.05  | 439.75  | -2.5%   | -2.3%   |
| normal     | 128 / 4096      | 1 / 100%       | 19.95   | 22.35   | +12.0%  | -3.8%   |
| normal     | 128 / 4096      | 16 / 50%       | 129.25  | 76.15   | -41.1%  | -0.1%   |
| normal     | 128 / 4096      | 64 / 50%       | 531.40  | 265.05  | -50.1%  | -8.3%   |
| normal     | 128 / 4096      | 64 / 100%      | 937.40  | 439.30  | -53.1%  | -6.2%   |
| normal     | 1024 / 4096     | 64 / 100%      | 3414.25 | 2707.90 | -20.7%  | -2.8%   |
| normal     | 128 / 64        | 16 / 0%        | 23.75   | 26.20   | +10.3%  | -2.5%   |
| normal     | 1024 / 4096     | 64 / 0%        | 907.05  | 912.20  | +0.6%   | -1.6%   |
| capacity   | 128 / 64        | 64 / 100%      | 470.15  | 429.35  | -8.7%   | -4.1%   |
| capacity   | 1024 / 64       | 64 / 100%      | 3067.45 | 2658.85 | -13.3%  | -7.5%   |
| high_water | 128 / 64        | 64 / 100%      | 482.95  | 446.90  | -7.5%   | -2.3%   |
| high_water | 1024 / 64       | 64 / 100%      | 3054.90 | 2674.15 | -12.5%  | -5.7%   |

主场景128 Active/4096 Parked/64次全成功的三轮变化分别为-54.0%、-51.7%、-55.1%，
同版本对照为-7.0%、-2.3%、-11.5%；1024 Active对应-16.3%、-32.5%、-16.4%，
同版本为+2.3%、-17.0%、+8.5%。主要连续成功收益跨轮次存在，不能把三轮中较大的
单轮改善当成稳定上限。

冷成功与同批后续热成功分列如下，均为4096 Parked、64次全成功，单位µs。
每个冷成功组48次，热成功组3024次。首次成功还包含原有安全检查和可能的Occupancy
构建，并非位置表单阶段计时。

| Active / 成功类别 | A p50 → B p50 | A/A p50变化 | A p99 → B p99 | A max → B max   |
| ----------------- | ------------- | ----------- | ------------- | --------------- |
| 128 / 首次冷成功  | 21.00 → 16.40 | -23.3%      | 31.20 → 40.50 | 31.20 → 40.50   |
| 128 / 后续热成功  | 14.30 → 6.80  | -3.5%       | 23.40 → 13.30 | 1784.60 → 45.70 |
| 1024 / 首次冷成功 | 58.40 → 53.60 | 0.0%        | 96.50 → 95.20 | 96.50 → 95.20   |
| 1024 / 后续热成功 | 50.90 → 41.30 | 0.0%        | 87.30 → 68.10 | 111.00 → 439.00 |

首次成功不主张稳定改善：128 Active的上表冷p50改善并未超出A/A变化；单次命令
场景反而由19.95增至22.35µs，三轮均增加11.2%～24.4%，接受并明确记录这项建表
取舍。该场景同版本单轮最大变化29.3%，因此也不能把12.0%归因为纯建表开销。
128 Active/64 Parked的64次成功仅改善2.5%，接近A/A的2.3%，不作确定加速结论。
全拒绝场景没有减少工作量；128 Active/16次拒绝观测到批次p50增加10.3%，但单次
拒绝汇总p50同为1.2µs，且第三轮同版本批次变化-50.9%，保留该波动而不归因于优化。

尾延迟未删样本：主128 Active批次p99/max由2717.7变为695.1µs；1024 Active批次
由5152.4变为4243.4µs，但后者热成功的单次max从111.0增到439.0µs，同版本对照
max为443.7µs。没有事件级调度证据，不把尾值判成算法改善或系统噪声。本数据仅
覆盖这些合成负载，不代表城市规模或真实停车业务的时延保证。

## 实现与边界

Active Vec 继续作为 live 的保序投影。成功 park 保序删除一个完整句柄；成功 leave
使用可重建的 slot → live rank 位置表二分定位，再执行 Vec 插入。完整句柄核对避免
复用槽位关联旧世代。仅追加 live 时补录后缀；删除、替换和全量重建使前缀失效。
普通 step 与 Completed 只改变 Active 成员；同修订原地切换不改变 live 顺序，因此
保留位置表。恢复及跨修订候选从空缓存开始，晋升时与候选的 live/Active 一起交换。

位置表只在所有领域检查通过后、提交前可失败预留。分配失败退回既有全量投影，
不新增公共错误；Active Vec 已按世界容量预留，提交段不新增可失败分配。
缓存不进入快照、日志、摘要或公共 API。后车扫描、正式 Occupancy 及停车安全规则
沿用现行实现；本切片只减少成功命令重复扫描 live 的成本。

## 测量口径

- 2026-09-17，日本时间；AMD Ryzen 9 9955HX，Windows 11 Dev 29661.1000，Rust 1.98.0 MSVC。
  opt-level=3、debuginfo=1、CARGO_INCREMENTAL=0，普通优先级，固定逻辑 CPU16。
  没有独占物理核心或关闭其他应用；时延不是跨机器保证。
- #678 的34场景：Active 128/1024、Parked 64/4096、1/16/64次真实 API 调用、
  0/50/100%成功率；单次调用不设50%，另外包含成功在前与拒绝在前。
- 额外8场景：Active 128/1024、Parked 64、64次命令、0/100%成功。
  `capacity` 声明65536车辆容量；`high_water` 先建立4096 Parked，再删除4032辆，
  留下相同 live 数但较高实际槽位水位。删除及建世界不在计时内。
- 每场景每进程2批预热、16批测量，每批从全新相同世界开始。批次耗时为逐次
  `leave_parking` API 计时之和；快照、断言及夹具均在计时外。首次成功列为冷成功，
  此批后续成功列为热成功。全拒绝不建位置表，不把进程预热称为位置表已热。
  冷/热仅指位置表；Occupancy仍按原状态序号失效，成功前的拒绝命令也照实计时。
- 三轮顺序分别为 A/B/Acontrol、B/Acontrol/A、Acontrol/A/B，Acontrol与A为同一
  冻结EXE。每轮、每场景的摘要与命令结果全部核对。p50为中位数，p95/p99采用
  nearest-rank，保留max，不删除离群样本。
- `analyze.py` 校验九次运行中各42场景的精确集合、16批及每批命令结果、时长和，
  拒绝重复、缺失、等量替换或摘要不一致。`summary.csv`/`summary.json`保留全部
  场景、冷/热/拒绝分类、三轮A/B及A/A变化率和汇总分位数。

## 工作量与保留内存

工作量计数来自独立 `cfg(test)` 运行，不混入时延。正常成功路径首次成功访问
一次live建立位置，其余成功不再扫描live；每次成功执行一次Active插入。Vec插入
仍需移动后缀，不宣称变成常数成本。拒绝命令既不建位置表也不改Active。

下面均为4096 Parked、64次API调用、交替排列；Active写入包含插入项自身与被移动
后缀，位置访问单列，不把它隐藏在零次全量重建中。

| Active | 成功率 | API次数 | 全量Active重建/live访问 | 定位构建/访问 | Active插入/写入项数 | Occupancy构建/输入项数 | 后车访问 |
| ------ | ------ | ------- | ----------------------- | ------------- | ------------------- | ---------------------- | -------- |
| 128    | 0%     | 64      | 0/0                     | 0/0           | 0/0                 | 1/128                  | 6176     |
| 128    | 50%    | 64      | 0/0                     | 1/4224        | 32/4128             | 33/4752                | 8224     |
| 128    | 100%   | 64      | 0/0                     | 1/4224        | 64/8256             | 64/10208               | 10208    |
| 1024   | 100%   | 64      | 0/0                     | 1/5120        | 64/65600            | 64/67552               | 67552    |

同一128 Active/4096 Parked/64次全成功基线重建访问 `64 × 4224 = 270336` 个live项；
候选只定位访问4224项。64次Occupancy构建及10208次后车访问依然存在。

位置表的堆内存按实际槽位水位保留，新增字节为 `4 × positions.capacity()`；在
本机另外增加32字节内联结构。声明容量65536但实际192槽位时，表容量192，增加
768字节堆内存；同样192 live但实际槽位水位4224时增加16896字节。内存测试在
无业务提交的单独建表前后比较Derived分区差值，证明该向量恰好计入一次。

## 正确性验证

定向测试逐步与全量live投影比较：冷/部分/热缓存、追加、删除复用槽位、完整句柄
世代、Completed、replace、正常步进、park/leave、恢复、同修订切换及跨修订晋升。
冷表及追加扩容注入分配失败后，回退成功结果与正常世界的快照、公开批次一致；
不安全后车与命令游标耗尽保留原首错且不预建位置。step在提交前失败保留快照和
公开批次，之后同拍重试成功。正常工作量矩阵另逐命令检查Active全量参考。

本地 `cargo +1.98.0 test --locked --offline -p laneflow-runtime` 共508项非忽略测试
通过（377项单元测试及全部非忽略集成/文档测试）；38项手工证据测试默认忽略。
本任务所需的工作量矩阵、位置内存证据和release对照另行执行。
`cargo +1.98.0 clippy --locked --offline -p laneflow-runtime --all-targets -- -D warnings`、
`xtask check-runtime-architecture`、`xtask check-wire-audit`及格式检查通过。
分析器的8组完整/损坏输入校验通过。全工作区测试、全工作区Clippy及CodeQL由
PR exact-head CI验证，不把本地Runtime范围的检查冒充全工作区测试。

## 复现与制品

分别在上述基线测量提交、候选实现提交的独立checkout中执行相同命令；两次显式
设置环境变量，核对Cargo JSON中 `profile` 完全一致后冻结EXE：

```powershell
$env:CARGO_INCREMENTAL='0'
$env:CARGO_PROFILE_RELEASE_DEBUG='1'
cargo +1.98.0 test --locked --offline --release -p laneflow-runtime --test active_order_evidence --no-run --message-format=json > build.jsonl
```

从 `compiler-artifact` 取得 `target.name=active_order_evidence` 的executable路径。
停止编译和其他测量后运行：

```powershell
pwsh -NoLogo -NoProfile -File research/issue-696-active-order/run.ps1 -BaselineExecutable '<baseline-exe>' -CandidateExecutable '<candidate-exe>' -OutputDirectory target/active-order-new
python research/issue-696-active-order/analyze.py target/active-order-new
python research/issue-696-active-order/test_analyze.py
cargo +1.98.0 test --locked --offline -p laneflow-runtime --lib parking_command_work_matrix -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --locked --offline -p laneflow-runtime --lib position_memory_follows_actual_slots_and_is_counted_once -- --nocapture
```

本次最终原始日志位于 `target/active-696/ab-final`；源码/构建清单、工作量和内存日志
位于 `target/active-696`。早先 `ab` 因候选debuginfo=0与基线不一致而弃用，不能并入
最终结果。原始日志、EXE与构建缓存不入库；异机复算本次数字需取得原始制品，
新checkout可按上述入口重新测量。

| 本次采用制品          | SHA-256                                                            |
| --------------------- | ------------------------------------------------------------------ |
| baseline.exe          | `da14525a2628b2aa72ed6469b67d45dc3ce46958361e44716ddc97a85295a11a` |
| candidate-debug1.exe  | `fee7ffc4fd8cc833330c0ca42d4ff85f616b87d8c1e6dfa5283bae1f79fb786d` |
| ab-final/summary.json | `29d27de0ee7b639db963fac9d934f5e405a7bbf6ce20aeebf4040a59e9ab24c4` |
