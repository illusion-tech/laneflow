# Windows 逐拍窗口与计数器状态验收

本轮继续第 0 步，不改变生产 Runtime。**先前 WPR 试跑的 PMC 不能进入性能比较**。
事件零丢失、计数非零和递增都不能排除计数器失效。五点 40 进程正式矩阵须等新的
计数有效性验收；本轮没有据此开展第 1 步生产优化。

## 32 条状态事件的解读

本机 `root/wmi` 中的 `PmcCounterCorruption_V2` 定义事件类型 49，版本来自
`PerfInfo_V2`。它的 `CounterStatus` 是 `CounterCorruptionStatus` 数组，数组元素
按 `WmiDataId` 排列为 `ProfileSource: UInt32`、`LastKnownGoodTimestamp: UInt64`。
因此每条 32 B 原始载荷是 CPU、计数器数目，以及两组 12 B 的来源和最后有效时间。
此解释以本机冻结的 WMI 定义为依据，不用 `tracerpt` 未知复合类型的猜测代替。

对未修改的原始 ETL 重新做只读原生检查：

- 原始事件 3004959，事件和缓冲丢失均为 0；QPC 频率 10000000 Hz。
- CPU 0–31 各一条状态记录，共 64 项；来源 19 / 26 对应本机 cycles / instructions。
- 最晚的最后有效 QPC 为 `319715435440`；最早的目标进程 CSwitch 为
  `319716875577`，两臂都落在最后有效到检测时间之间的未认证范围。
- AoS 进程区间 `319716875577–319901012463`，列式进程区间
  `319901315352–320105950611`。这些区间包含安装和输出，不是公共 step。
- 计数和时间逆转 0、线程切换连续性失败 0；11592 条切换没有 PMC。缺失记录须按
  配置前后及测量窗口检查，不能单凭全文件累计值断定逐拍完整。

保守拒绝范围是最后有效到检测之间；不声称硬件恰在最后有效时间发生损坏，也不把
检测发生在跟踪结束时视为“只影响收尾”。本机 Hyper-V 与 VBS 在运行，但未证明它们
是根因；本轮未更改这些设置、安装工具或重启。

## 逐拍边界工具

`prepare-wpr` / `prepare-wpr-at` 复用现有受控 Windows MSVC 构建器和 Rust 1.98.0。
在完整冻结源码副本的公共 step 外加时间锚点；`LF814_WPR_WINDOWS=1` 开启，`0`
关闭，用相同实际 EXE 做探针开销对照。生产源码不增加探针。

锚点用 `SystemTime` 的整数 FILETIME 和两侧 `Instant` 括住读取误差；公共 step
仍用单调时钟测量。每拍记录起止上下界、单调耗时和两种时钟的偏差。
时钟倒退、偏差超过 100000 ns 或边界不完整均拒绝。CSwitch 跨边界的计数不得
按墙钟比例分配，也不得用整个进程的计数除以 256。

`laneflow-columnar-measurement` 增加三个入口，仍只新建仓库外产物：

| 入口 | 用途 |
| --- | --- |
| `export-wpr <完整 SHA> <新目录>` | 完整源码冻结与 step 外探针导出；不单独认证构建或 PMU |
| `windows <stderr> <新 JSON>` | 验证同一进程 256 个顺序窗口、误差范围及单调耗时 |
| `wpr-status <原生检查 JSON> <新 JSON>` | 拒绝丢失、逆转、重复状态，保留各 CPU 最后有效时间与目标区间重叠 |

纯 Rust 逻辑继承工作区 `unsafe_code=forbid`，不新增依赖。原生 ETL 调查器只读已有
原件，作为一次性调查源码留在外部证据目录。Windows 标记不等于已证明 ETL 时钟
对齐；没有状态重叠也不单独认证计数器。正式解码、物理核亲和性、boost 控制及 WPR
开销仍须实际验证，之后才启动 40 进程矩阵。

本轮测量工具和既有研究工具的 30 项测试、Clippy（`-D warnings`）通过；原生读取
和 WMI 定义保存在仓库外。工具源码冻结于
`988db1a052c4ae9574cfe68d4a1fa6216c2ad5eb`；生产 Runtime / motion-kernel 仍与
`abd0a253` 一致。两臂和新诊断均按既有受控配方、Rust 1.98.0 构建。

## 普通验证与新阶段表

五个普通权限进程完成：两臂分别开关 Windows 标记，另有一个独立诊断。每进程
256 拍，绑到四个已核实物理核，全部载荷、checkpoint、状态/决定/事件/交通匹配
冻结 AoS 参照。该组没有关闭 boost，也没有完整平衡的探针开销矩阵；不报告探针
净开销或性能收益。两个有标记进程的最大边界宽度分别为 2300、2200 ns，时钟偏差为 0。

独立诊断每拍整数 ns 闭合，以下是**一个进程**的阶段均值，整拍 **21.032949 ms**：

| 互斥段 | ms/拍 |
| --- | --- |
| Preflight | 2.438606 |
| Occupancy | 2.767763 |
| WaitingPrepare 自身 | 0.373809 |
| ConflictPrepare 自身 | 2.814575 |
| MotionLoop 自身 | 0.047318 |
| WaitingFinalize | 0.129649 |
| Signals | 0.112823 |
| ConflictFinalize | 1.097196 |
| WaitingOutputs | 1.271423 |
| Commit | 0.649841 |
| Frontier | 1.804052 |
| P4 | 0.217951 |
| P5Dispatch，含完整 join | 4.001705 |
| P5Consume | 0.460559 |
| WaitingPreview | 2.686904 |
| P6 Validation | 0.152457 |
| 未归属框架与 probe 余量 | 0.006317 |

表中显示值已四舍五入；逐拍闭合使用原始整数。P6 不再混入未知余量，最终余量约占
整拍 0.030%。这不是同比性能结论，也不从单进程推导串行时间或扩展曲线。

## 绑核、临时关闭 boost 的 WPR 复测

维护者另行授权一次新的短试跑及临时 boost 调整。两个独立进程各 256 拍，
workers=4、进程亲和性 `85`（CPU 0/2/4/6，四个不同物理核）。入口读回一致，
亲和性设置早于全部 step。AC/DC boost 指数由 2 改为 0，结束后均恢复为 2；
电源方案和物理拓扑保持，自己的 WPR 会话已关闭，临时提权进程已退出。

两臂的窗口和完整交通结果通过。ETL 247463936 B，SHA-256
`fbfe37092c3f96c306773e95a7a4dc6e8fae20d22c927a6c1e2044ff38f8add5`，
3363197 条事件、事件和缓冲丢失均为 0。新调查器从 `TRACE_LOGFILE_HEADER` 读取
丢失数；不使用文档标为未使用的 `EVENT_TRACE_LOGFILE.EventsLost` 字段。

同一 ETL 分别读取原始 QPC 和系统转换的 FILETIME。32 个 CPU 首尾事件的转换
偏移完全一致，展开到同一时间域后，两臂所有 step 都在所属进程区间内；边界宽度
最大分别为 4100、4200 ns，时钟偏差为 0。Rust 1.98.0 的 `SystemTime` 使用精确
FILETIME，[ProcessTrace 默认转换]也使用这个纪元；没有猜测 `Instant` 内部布局。

**新采集仍有 32 条状态记录、64 项计数器失效范围。四个允许 CPU 上，两臂全部
256 拍都处于最后有效到检测之间的未认证范围，PMC 再次被拒绝。** 没有按比例
插值、择优重跑或把整段进程计数除以 256。计数器根因、完整线程区间归约及 WPR
开销校准仍未完成，正式矩阵为 0/40；`S_effective` 未拟合，第 0 步仍未通过。

原件见 [冻结索引](windows-window-archive.toml)。新外层 4384 件、冻结源码 1408 个
Git blob 均恢复验封；恢复实际 EXE 后重算窗口、交通、阶段、状态拒绝和原生 ETL
结果一致。完整旧归档内嵌并验封，复用其既有恢复证明。首次找不到 PowerShell 路径
及 `tracerpt` 拒绝 NUL 输出的失败原件保留；这两次都不构成测量结果。暂无公开下载，
Git 只保存 Rust、短结论和索引，旧提交历史保持。

[ProcessTrace 默认转换]: https://learn.microsoft.com/en-us/windows/win32/api/evntrace/ns-evntrace-event_trace_logfilew
