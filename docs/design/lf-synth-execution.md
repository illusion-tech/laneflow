# LF-SYNTH 执行核验

**文档状态**: Active<br>
**最后更新**: 2026-10-30

## 1. 语义消费边界

`laneflow-synth-harness` 接受 [规范输入包](lf-synth-input-bundle.md) 的 W1–W4。
先执行生成器 `verify_directory` 的文件/摘要/规范编码及完整重建校验，再通过 LFCA
受检读取、保留 Spatial 的共享路网、显式政策安装、路线注册及车辆生成消费输入。
稳定标识解析到本修订 ordinal，logical rank 不冒充 ordinal 或 Runtime handle。
Traffic Runtime 生产代码不感知 workload ID。

交通行为与窗口公式以 [性能基线](core-runtime-performance-baseline.md) 为唯一权威。
工具读取受检计划并独立核对周期、预热及观察长度，不允许指定短窗口的正式入口。
W1/W2/W3 在 16 ms 使用 `14508 + 29016` 拍，33 ms 使用 `7044 + 14088` 拍；
W4 使用相同预热长度及恰好两条观察拍。初始边界、全部预热拍和观察拍均核验。
W1/W2 初始化后不得发出调用方命令；W3/W4 只执行受检计划和下述冻结规则。

W1/W2 每拍检查：

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
`run` / `matrix` / `burst-matrix` 固定单 worker exact 路径；显式 worker 入口见
第 7 节。worker 扫描不等于穷尽 schedule permutation。

## 2. W1/W2 证据与摘要编码

`report.json` 的域为 `lf-synth-semantic-run-1`；矩阵为
`lf-synth-semantic-matrix-1`。结果保留完整输入 manifest（含其来源提交）、调用者
声明的实际实现提交、窗口、计数和检查点。无 wall-clock percentile 或产品 Pass。
回执的 `workers` 来自已安装世界的执行配置，不进入交通输入身份或状态摘要。
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

## 3. W3/W4 命令与观测

工具从受检 `scheduled` 数组读取操作、逻辑序号、路线、出现项、进度和速度；
生成器完整重建校验同时约束数组顺序及事件驱动规则。正式入口不是任意脚本解释器。
W3 的事件屏障和生命周期替换输入遵循性能基线 §4.2，W4 遵循 §4.3；不重写合同。

- W3 在首拍前按 cell 预约；逐条核对 `Committed` 收据及完整 reservation，拒绝
  `NoChange`。到达观测按 Runtime 返回顺序消费，只允许对应 cell 的 route 0 slot 0，
  且必须位于观察窗口、精确锚点、零速和零微米余数；重复或错配使运行失败。
  全部到达后紧接下一 boundary 按 cell 入位，核验一条成功 step 的 24/76 人口，
  下一 boundary 按 cell 离位。屏障后至少剩两条观察拍，不发第二次预约。
- W3 在首个观察 boundary 执行计划中的 despawn/spawn；下一次成功 step 只允许
  指定探针产生宿主 `Active → Completed` 观测，按逻辑序号形成冻结替换输入。
  `Blocked` 只保留原输入、身份和顺序供后续 boundary 重试，分别记录尝试及实际
  command cursor；禁止重算入口或跳换探针。只有这些已声明探针可以处于待替换
  Completed，结束前必须全部恢复，持续 pending 或额外人口耗尽均使运行失败。
- W4 严格执行 B-1 的完成探针、B0 的 replace/leave 与 A-green 提交、B1 的新鲜
  转换探针。信号使用独立周期公式及 `cycle-delta` offset 核验；B0 不得过门，
  B1 恰好每 cell 一个指定探针过门，随后结束。任意命令失败均使该行失败。
- 逻辑序号跨替换不变；每次新句柄必须不同、旧句柄失效且不与其他 live 身份重合。
  工具维护预期 live 顺序和逻辑代次：despawn/spawn 追加到末尾，atomic replace
  保持 live 序号。逻辑代次只统计宿主重建次数，不冒充 Runtime 私有 generation。
- 每拍核验身份、人口、全部泊位的排他状态及 binding、冻结停车状态、整数运动、
  信号和有序事件。命令后的状态作为下一 step 的运动起点，不能把显式重建位置当作
  一拍位移。车道顺序在命令边界按合法摆放重新建立，之后禁止重叠或超车。
  自然 entry→exit 仍须精确匹配当前 live 顺序的那对转换事件及全部公开载荷。

W3 完整窗口结束时，每个初始 Active 逻辑身份恰好过门一次；W4 只允许 B1 的指定
探针过门。全部行各执行每 cell 六次成功命令；Blocked 尝试不推进 command cursor。
检查点包含初始、预热结束、前两条观察拍、H/2H/4H（若在窗口内）、最终边界，以及
W3 入位单拍和离位后的边界。最大减速度与超 profile 次数继续单独报告。

## 4. W3/W4 证据编码

W3/W4 回执域为 `lf-synth-burst-run-1`。逐拍摘要、命令批次摘要及命令收据摘要均为
ADR 0033 内部完整性摘要：LaneFlow 进程产生和复核，持久化在运行材料，使用
BLAKE3-256、固定域和小端整数；不进入 Runtime 热路径或正式计时。

逐拍前像依次为：

1. ASCII `lf-synth-burst-boundary-1` 加 NUL。
2. `u64` tick、time_ms、实际 command cursor，及 `u32` 人口数。
3. 逻辑序号升序的十个 `u32`：逻辑序号、逻辑代次、live 序号、route rank、route
   occurrence、progress_mm、carry_um、speed_mm_s、status（Active=0/Parked=1/
   Completed=2）、本拍过门标志（0/1）。
4. `u32` 转换事件数、`u32` 到达观测数，再按返回顺序写每条到达的 `u32` 逻辑序号。

固定身份、泊位、信号及事件载荷已由逐值断言核验，动态字段决定该 workload 的
公开事件语义；摘要不宣称覆盖每拍内部状态。`ticks.bin` 的头为 ASCII
`lf-synth-burst-ticks-1` 加 NUL，记录仍为初始 tick=0 起的 `u64` tick 和 32 字节摘要。
流摘要为 ASCII `lf-synth-burst-stream-1` 加 NUL 后接相同记录，不含文件头。

命令输入编码为六个 `u32`：操作码、logical rank、route rank、edge occurrence、
progress_mm、speed_mm_per_s。操作码依次为 reserve=0、park=1、leave=2、
despawn=3、spawn=4、atomic replace=5。批次摘要为 ASCII
`lf-synth-command-batch-1` 加 NUL、`u32` 命令数、按执行顺序的输入编码。
`TickInput` 摘要为 ASCII `lf-synth-step-input-1` 加 NUL、`u64` delta_time_ms。

`commands.bin` 的头为 ASCII `lf-synth-command-receipts-1` 加 NUL。每条收据为
四个 `u64`（boundary、从零开始的尝试序号、命令前/后游标）、命令输入编码、三个
`u32`（前/后逻辑代次、结果：Committed=0/Blocked=1），每条 68 字节。
收据摘要覆盖含头部的全部文件。`batches.json` 每个批次另记录输入摘要、首条收据
序号、命令数、前/后 command cursor 及现行完整状态摘要。它随批次落盘；中断和
失败保留 `complete=false`。完整回执同时保留到达/完成观测的实际 tick 与逻辑序号。

## 5. 测试构建的失败重试

Runtime 内 `formal_input_failed_step_retry` 是显式 ignored 测试，只允许 optimized
test binary。普通生产二进制不包含 failpoint，不新增生产 feature 或调试入口。
该测试独立读取和校验两次正式输入，各自受检构建修订、安装世界并推进完整预热；
按相同计划在两个独立世界应用一次命令。W3 注入点是第一批 atomic replacements
后的 step，W4 是 B0 的 step。

保存 pre-command 和 post-command/pre-step 规范快照及状态摘要；两个世界的命令
后状态必须逐值相等。`AfterTransitions` 必须返回
`ParkingObservationAllocFailed`，且失败前后规范快照、状态摘要、已发布事件、
信号和 command cursor 全部相等。失败点消耗后，只重试同一 `TickInput`；与未注入
失败的独立世界比较 outcome、有序事件及完整规范快照。W4 只有重试成功后才进入
B1。W3/W4 均在前两条观察拍结束后停止本 guardrail，完整 W3 窗口由普通矩阵证明。

`pre-command.lfrs`、`post-command.lfrs`、`fresh-post-command.lfrs`、`failed.lfrs`、
`retry.lfrs`、`fresh.lfrs` 和 `final.lfrs` 保存上述捕获的现行规范编码；这不代表
快照恢复或继续运行认证。`retry-matrix` 还将失败边界的批次摘要、step 输入摘要、
命令前后状态和游标、重试及最终状态与相同实现的正常矩阵比较，并逐字节比较应相等
的快照文件。回执域分别为 `lf-synth-retry-run-1`、`lf-synth-retry-matrix-1`。
测试构建的运行耗时不进入 latency/tail 或 Product Pass/Fail。

## 6. 独立进程与后续范围

普通矩阵分为 `10000/100000 × W1/W2 × 16/33 ms` 和
`10000/100000 × W3/W4 × 16/33 ms`。每行两次全新进程使用同一输入、
同一实现二进制和单 worker，独立初始化并跑完整窗口。矩阵最多同时启动四个消费
进程；这是进程间并发，不改变世界内的 worker 数。各进程不共享世界或可变状态，
不采集性能，完成先后也不影响汇总按身份排序。W3/W4 同时精确比较命令收据文件、
批次文件和完整 JSON 回执。重试矩阵最多同时运行两个测试进程，每个进程拥有独立
的失败/正常两世界；正式身份仍是单世界规模，不聚合两个世界的吞吐。任何失败或轨迹不一致均使
矩阵不完整，不能以另一次成功覆盖原始失败记录。两轮语义对照不替代性能协议三轮。

表现选择/60 Hz 外层帧/追赶、性能与资源由独立的
[测量入口](lf-synth-measurement.md)承载；本语义入口不认证快照恢复、Routing 或
完整五轴，不将语义对照转化为 Product Pass。

## 7. 标称矩阵的跨 worker 对照

`worker-run` 显式接收非零 worker 数，将 `ExecutionConfig` 传给同一安装与 W1–W4
核验路径。支持范围由 [执行配置合同](traffic-runtime-execution-config.md)规定；
不按 CPU 数推断、不静默降级，不改变正式输入、交通配置、命令计划或窗口。
成功回执记录世界实际使用的 worker 数；失败回执保留请求值与错误。

`worker-matrix` 只覆盖一万/16 ms、十万/33 ms 的 W1–W4 八个标称身份。
每个身份使用 1/2/4/8/16 worker，各运行三个全新进程，共 120 次。配置按规模、
场景、worker 排序，每轮将四十项配置顺序左移 `7 * (round - 1)` 项。最多四个
子进程并发，运行中的配置 worker 数之和最多 32；这只是宿主进程调度限制，
不表示总 OS 线程数、CPU 使用率或多世界吞吐预算。

每个身份以第一轮单 worker 运行作为参考；其余十四次运行均须匹配，共 112 对。
比较前核对实际 worker、实现提交、完整输入 manifest、正式窗口长度和必需状态
检查点。完整 JSON 回执只排除 `workers`，其余字段全部精确比较；`ticks.bin` 与
完整有序 `events.bin` 逐字节相等，W3/W4 另比较 `commands.bin` 和 `batches.json`。
这是同实现的成功路径确定性对照；公开轨迹与检查点的覆盖边界仍按前文解释。

矩阵域为 `lf-synth-worker-matrix-1`，保留完整回执、输入身份、配置/轮次、进程启动
次序、比较结果、实际可执行文件的字节数/SHA-256，以及各运行文件的字节数/SHA-256。
SHA-256 供外部通用工具重算，属于 ADR 0033 的互操作摘要；原有 BLAKE3 编码不变。
执行前后重新核对同一个 binary 文件身份。环境 JSON 是调用者声明，必须含相同
`implementation_commit`、`source_clean=true` 及非空 `rustc_vv`、`cargo_version`、
`target`、`build_command`；其余硬件/OS 字段原样保存。工具不据这些声明认证源码与
binary 的关联，执行者仍须提供干净检出、构建日志和冻结二进制。

输出目录必须全新，开始即写 `complete=false`。首个运行或比较失败后不再启动新
进程，收集已启动运行并保留所有材料；父进程因 I/O 错误或展开退出时回收其仍持有
的子进程。不自动补跑、跳过或拼接失败矩阵。只有 120 次运行和 112 对比较全部成功
才标记完成；成功后也不产生性能或产品通过声明。

非标称交叉组合、强制调度/完成置换、跨 worker failpoint 的首错与 retry/fresh、
快照解码恢复、Routing、Spatial/Adapter 的跨 worker 接线和完整五轴均不由本入口
证明。两轮单 worker 旧证据与本次三轮矩阵分别保留各自源码身份，不能拼成同一验收。
