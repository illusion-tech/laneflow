# LF-SYNTH 执行核验

**文档状态**: Active<br>
**最后更新**: 2026-10-29

## 1. 语义消费边界

`laneflow-synth-harness` 只接受 [规范输入包](lf-synth-input-bundle.md) 的 W1/W2。
先执行生成器 `verify_directory` 的文件/摘要/规范编码及完整重建校验，再通过 LFCA
受检读取、保留 Spatial 的共享路网、显式政策安装、路线注册及车辆生成消费输入。
稳定标识解析到本修订 ordinal，logical rank 不冒充 ordinal 或 Runtime handle。
Traffic Runtime 生产代码不感知 workload ID。

交通行为与窗口公式以 [性能基线](core-runtime-performance-baseline.md) 为唯一权威。
工具读取受检计划并独立核对周期、预热及观察长度，不允许指定短窗口的正式入口。
16 ms 使用 `14508 + 29016` 拍，33 ms 使用 `7044 + 14088` 拍。
初始边界、全部预热拍和观察拍均核验；初始化后不得发出调用方命令。

每拍检查：

- live 顺序、同一代际句柄、profile/class/route/车长与初始身份相等；W1 保持
  75/25 Active/Parked，W2 全 Active，Completed 为零；command cursor 不变。
- Parked 的整个 `VehicleState`、Occupied 绑定和泊位占用者不变；停车 arrival 为空。
- route occurrence 只允许 entry→exit 一次；进度与微米余数合法、绝对进度不倒退、
  速度不超过 13900 mm/s、位移不超过该速度的一拍距离、正加速度不超过冻结 profile。
  正加速度检查容许毫米量化的 1 mm/s，位移检查容许 1 µm；这不放宽闯灯或重叠。
  速度/位置为有界整数，固定步长非零，派生加速度有限。
  [整数几何 §1/§5](traffic-runtime-integer-geometry.md) 允许硬停止投影直接清速度，
  因此不能把 `emergency_decel` 强加为已提交速度差的上限。回执单独记录观察到的
  最大减速度（mm/s²，向上取整）和超出 profile 紧急减速度的拍/车计数，不隐藏硬停。
- 同一 route 的 Active 按初始 slot 顺序不超车，车身区间不重叠；Parked 不占车道。
- 每个信号组与冻结六阶段及 `(cell*997)%cycle` 独立计算的灯态完全一致。
  entry→exit 必须在调用 step 前的已提交绿灯下发生；成功 step 后验证新时刻灯态。
- 每次 entry→exit 对应 live-order 中恰好一对 `GateCrossed`、
  `ManeuverTraversalCompleted`。事件 tick、车辆、更新序号、kind、Gate、route、
  occurrence、hop、位置全部精确核对，不接受缺失、多余或乱序事件。
- 本输入不含 Waiting/Conflict；其 membership/traversal 均须为空。完整窗口结束时
  每个 Active 恰好过门一次，Parked 从未过门。

这是冻结 workload 的行为核验，不是对任意 Traffic Runtime 场景的通用验证器。
只运行当前单 worker exact 路径；worker/schedule permutation 需后续单独证据。

## 2. 证据与摘要编码

`report.json` 的域为 `lf-synth-semantic-run-1`；矩阵为
`lf-synth-semantic-matrix-1`。结果保留完整输入 manifest（含其来源提交）、调用者
声明的实际实现提交、窗口、计数和检查点。无 wall-clock percentile 或产品 Pass。
创建新目录先写 `complete=false`，只有全窗口及末态检查通过后才标记完成；失败或
中断保留已写轨迹与日志。矩阵只有八行全部两轮一致后才标记完成。

逐拍公开边界摘要是 ADR 0033 的**内部完整性摘要**，产生与重算方均为 LaneFlow
harness；持久化在本次证据中。采用默认 BLAKE3-256，不依赖 `Debug`、Rust `Hash`
或本机布局。摘要前像依次为：

1. ASCII `lf-synth-public-boundary-1` 和一个 NUL。
2. 小端 `u64` tick、time_ms、初始化后的 command cursor，再接 `u32` 人口数。
3. 每个 logical rank 升序的八个小端 `u32`：rank、route rank、route occurrence、
   progress_mm、carry_um、speed_mm_s、status（Active=0/Parked=1）、本拍是否过门
   （0/1）。
4. 小端 `u32` 公开事件数量。

其余固定字段、身份/绑定、信号与事件全部由逐拍断言精确核对。过门行唯一确定那对
已逐字段核验的事件及其顺序，因此无需重复保存事件载荷；未来允许其他事件时必须
修改编码版本和验证合同。公开轨迹不覆盖未列出的内部状态。

`ticks.bin` 为 ASCII `lf-synth-public-ticks-1` 加 NUL，随后从初始 tick=0 开始，
每拍一条 `u64` 小端 tick 加 32 字节边界摘要。总轨迹摘要的前像为 ASCII
`lf-synth-public-stream-1` 加 NUL，随后相同的 tick/摘要记录（不含文件头）。
矩阵同时逐字节比较轨迹文件与 JSON 完整回执，不仅比较最终摘要。

初始、预热结束、观察 H/2H/4H 和最终边界另调用 `capture_snapshot` 与现行
`deterministic_state_digest`，记录其完整状态摘要。它们验证检查点内部状态的重放
一致性，不等于快照编码、恢复及继续运行认证。所有摘要与只读遍历都在宿主端，
不进入 Runtime 热路径，不用于正式 latency/tail 或 allocation 指标。

## 3. 独立进程与后续范围

正式矩阵为 `10000/100000 × W1/W2 × 16/33 ms`。每行两次全新进程使用同一输入、
同一实现二进制和单 worker，独立初始化并跑完整窗口。矩阵最多同时启动四个消费
进程；这是进程间并发，不改变世界内的 worker 数。各进程不共享世界或可变状态，
不采集性能，完成先后也不影响汇总按身份排序。任何失败或轨迹不一致均使
矩阵不完整，不能以另一次成功覆盖原始失败记录。两轮语义对照不替代性能协议三轮。

W3/W4 的命令执行、表现选择/60 Hz 外层帧/追赶、性能与资源、快照恢复、Routing、
五轴认证均由后续切片承载；本结果不能解除这些缺口或转化为 Product Pass。
