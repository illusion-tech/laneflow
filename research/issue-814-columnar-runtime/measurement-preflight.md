# 第 0 步：阶段闭合与 WPR 能力试跑

本页保留能力试跑交付边界；32 条状态记录的后续解读与逐拍工具见
[Windows 窗口验收](windows-window-results.md)。旧试跑的 PMC 已被拒绝进入性能比较。

本轮只补测量前置条件，不改变生产 Runtime。**第 0 步尚未通过**：本机 WPR
已采出双 PMC 数据，但逐拍窗口、线程区间完整性、计数器状态、物理核亲和性和频率控制
尚未完成验收。workers=1/2/4/8/16 的正式同窗矩阵尚未采集。

## 本机采集结论

Debian WSL2 当前没有 `cpu` 硬件 PMU、`perf` 或逐 CPU 频率控制接口。Windows
已安装 WPR 10.0.29671；`pmcsources` 列出 `TotalCycles`（19）与
`InstructionsRetired`（26），没有在用 PMC 会话。[微软 PMU 文档]说明如何在
CSwitch 事件上记录硬件计数；采样次数不能替代计数差值。

纯声明式配置见 [pmu-cswitch.wprp](pmu-cswitch.wprp)。本机 WPR 的 `profiles` 与
`profiledetails` 校验成功；计数器使用 `Strict=true`，不静默忽略配置失败。
首次普通权限启动返回 `0x80070005`，没有开始跟踪，拒绝原件保留。
维护者随后授权一次临时 UAC 提权，完成一组 AoS 和一组 AVX2 列式能力试跑。

- Ryzen 9 9955HX，16 物理核、32 逻辑 CPU；workers=4，256 拍，无预热。
- 冻结 AoS 源码 `cbfbb14a714d819cd5e608a1b759575d378f3d53`；列式源码
  `abd0a253685c420af2a8bc09f1ee2c6dd3f46030`，复用已冻结执行文件。
- 100000 总车辆，初始 75000 Active + 25000 Parked，dt=33 ms、seed=544。
  两臂全部载荷摘要、checkpoint、状态/决定/事件/交通计数均匹配独立冻结 AoS 参照。
- WPR 启停均成功；自己的命名会话已关闭，没有更改电源设置或安装工具。
- ETL 219152384 B，SHA-256
  `6fd17e3e9832341cce6d094acbfc6398120ecf4435a594d757c9c1339726a953`。
- 原始事件 3004959 条；事件和缓冲丢失数均为 0。原生只读检查读出
  2706541 条 16 B 双 PMC 扩展数据，全部非零，2706509 条两槽在同 CPU 上均有变化。
  ETL 自身配置顺序为 `InstructionsRetired`、`TotalCycles`，不按配置文件书写顺序猜测槽位。
- 从线程启动载荷识别到每臂七个线程；AoS / 列式分别有 10507 / 9731 条关联 PMC
  CSwitch 记录。这是能力检查，不是完整的线程区间归约或每拍计数。

系统自带 `tracerpt` 可读取丢失数与计数器配置，但不能完整解码本机 CSwitch v5。
因此原件中额外保留一次性 Rust 原生 ETL 检查器及所用缓存依赖。它只读已有 ETL，
不控制会话，不进入维护工具或生产源码。32 条 `PmcCtrConrruption` 状态记录尚未解释；
不得仅凭非零计数宣布有效性验收通过。本轮没有绑核、关闭 boost、标记公共 step 边界
或测量 WPR 开销，也不报告性能差值。

## 历史诊断的互斥阶段表

下面重新分析上一轮冻结诊断的最终版本第 3、4、8 个独立进程，各 256 拍。
三个进程分别为 **22.395376、22.738227、22.779493 ms/拍**；诊断耗时不替代普通版
约 20.4 ms 的分布，也不属于本轮同窗 AoS 或硬件计数结果。

每拍从父段扣除嵌套子段，再单列未归属余量；整数 ns 总和逐拍等于公共 step。
下表是三个进程的阶段均值范围，范围的两端不能直接相加成某个进程总时间。

| 互斥段 | ms/拍 |
| --- | --- |
| Preflight | 2.637379–2.678311 |
| Occupancy | 2.764082–2.987884 |
| WaitingPrepare 自身，不含 Preview | 0.406639–0.411378 |
| ConflictPrepare 自身，不含 Frontier/P4 | 3.098768–3.134135 |
| MotionLoop 自身，不含 Dispatch/Consume | 0.053051–0.053933 |
| WaitingFinalize | 0.119984–0.129460 |
| Signals | 0.119256–0.121919 |
| ConflictFinalize | 1.145270–1.183615 |
| WaitingOutputs | 1.363207–1.371004 |
| Commit | 0.718493–0.762754 |
| Frontier | 2.000071–2.031139 |
| P4 | 0.235927–0.249119 |
| P5Dispatch，含完整 join | 4.148519–4.231805 |
| P5Consume | 0.480481–0.529148 |
| WaitingPreview | 2.882003–2.917718 |
| 未归属框架、P6 与 probe 余量 | 0.163288–0.170753 |

历史保留槽没有独立测量 P6，不能把零值解释成零成本。新的诊断导出把槽 15 改为
`Validation`，从 WaitingOutputs 计时结束到 P6 返回单独计时；尚未重新采集这一版。
阶段工具拒绝缺拍、重复调用、子段超父段、阶段和超过整拍以及混合 P6 schema。

## 已交付的 Rust 测量入口

从 Git 工作树运行 `laneflow-columnar-measurement`。所有产物必须在仓库外，路径检查
使用 Git 顶层目录，不能通过从子目录启动把原件写回仓库；已有输出不覆盖。

| 命令与参数 | 用途与边界 |
| --- | --- |
| `stages <trace.stderr> <新 JSON>` | 完整 256 拍互斥阶段及显式余量 |
| `traffic <参照目录> <运行目录> <新 JSON>` | 验封全部载荷并比较状态/决定/事件/交通，不认证性能 |
| `plan <完整候选 SHA> <新 JSON>` | Linux perf 分组硬件计数方案 |
| `plan-wpr <完整候选 SHA> <新 JSON>` | Windows WPR 方案；窗口采集与正式解码明确标为未实现 |
| `perf <perf JSON 对象流> 256 <新 JSON>` | 校验原始 cycles/instructions，拒绝缺失、零、溢出、重复或 multiplex；本身不证明窗口来源 |
| `fit <五组 worker 墙钟均值 JSON> <新 JSON>` | 拟合 `T(p)=S_effective+P/p`，保留残差与拟合适用性 |
| `preflight <新 JSON>` | Linux perf 只读清单；发现目录不代表 PMU 可用 |
| `export-perf <完整源码 SHA> <新目录>` | 导出完整冻结源码与 step 外 FIFO 握手；未验收 Linux 受控构建和采集 |

两种矩阵方案都固定同窗 AoS + AVX2 列式、workers=1/2/4/8/16，每点 ABBA 与 BAAB，
共 40 个独立进程，不择优重跑。先构建并核对等价编译配方、完整源码/工具链/实际执行
文件/输入与方案摘要，再串行采集。Linux 原有受控构建器目前专用于 Windows MSVC，
不能把源码导出成功当成 Linux 构建链已经移植。

正式 WPR 采集还须验证目标线程的调入/调出区间和公共 step 的起止时间；跨窗口边界的
CSwitch 区间要披露计数不确定性，不能按墙钟比例分摊后声称精确逐拍计数。进程生命周期
包含安装、载荷处理和输出，不能整体除以 256 当作公共 step 的 cycles/拍。
另需物理核拓扑与亲和性、固定频率或 boost 关闭的前后原件，以及独立探针开销检查。

拟合输入用每拍墙钟；所有线程 cycles 的总和不是 `T(p)`。`S_effective` 也包含调度、
缓存和内存扩展效应，要结合重复样本、残差与独立协调器时钟解释，不能直接认证为纯串行时间。
本轮先完成计数有效性和窗口工具，再开展 40 进程矩阵；没有据此启动第 1 步生产优化。

研究工具的 22 项测试、导出锚点、Clippy（`-D warnings`）、fmt 与 diff 检查通过。
三个实际负证明分别拒绝交通 checkpoint 不一致、从子目录向仓库写入原件和覆盖已有输出。
完整工具源码冻结在 `c9620e10cb29735daa8d96fa38c18a157404db69`。

冻结原件和验证摘要见 [归档索引](measurement-archive.toml)。外层 126 件及恢复后的
1403 个 Git 源码 blob 全部验封；恢复的执行文件重新分析阶段、交通与 PMC，结果一致。
内嵌上一轮完整源码/原件归档按摘要验封，复用它已有的三层恢复证明，不声称重新恢复三层。
可再生成的 2.8 GB tracerpt XML 与初始开发态 perf 导出留在原始外部目录，不重复打包；
原始 ETL、配置、输入、失败、工具源码/实际 EXE、PMU 解读和验收边界均在本层。
所有原始数据保存在仓库外，无公开下载。本轮不重写旧 Git 历史。

[微软 PMU 文档]: https://learn.microsoft.com/en-us/windows-hardware/test/wpt/recording-pmu-events
