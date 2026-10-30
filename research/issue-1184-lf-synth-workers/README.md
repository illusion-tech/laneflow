# LF-SYNTH-v2 标称矩阵的跨 worker 确定性

2026-10-31，完成 [#1184](https://github.com/illusion-tech/laneflow/issues/1184) 的
F1 子切片：**120 个完整窗口、112 对语义比较全部通过，零运行或比较错误。**
八个标称身份各使用 1/2/4/8/16 worker、三个全新进程；本矩阵没有失败补跑、跳过
或替换轮次。便于审阅的八行结果见 [results.json](results.json)。

共完成 3,232,860 次步进，保存 780 个状态检查点记录和 198,000 条命令收据。
独立复核遍历 3,232,980 条拍记录，核对 3,316,500 次过门，重算 480 个文件摘要；
112 对完整回执及 336 对文件摘要全部一致。结果限于下述成功路径 worker 扫描，
`Product TBD / Uncertified` 保持不变。

## 来源与条件

实现冻结为 `d81204adb8318e1521d064f9364002b6bd64fec2`，远程标签为
`evidence/issue-1184-worker-source`。正式输入的设计来源与生成提交均为
`9029c021d1f10a7506eedbf5f2fae3b5a7d57629`。后续报告整理不改变运行来源。

环境为 Windows 11 Insider 29695、Rust 1.99.1 / LLVM 23.1.1、
`x86_64-pc-windows-msvc`、AMD Ryzen 9 9955HX（16 核 / 32 线程）；
系统报告约 61.68 GiB 物理内存。普通 release 二进制从干净冻结提交构建并复制到
仓库外，SHA-256 为 `12764da12cde4631e61fbef6bea7fd36b250d311e5920208b3e6f82b0cd39ba3`。
运行前后 527 个来源文件、HEAD/源码树、干净状态和二进制身份均核对一致。
Cargo 可复用有效构建缓存，不声称执行了无缓存全量编译。

## 运行与比较

一万/16 ms 和十万/33 ms 各覆盖 W1–W4。每轮四十项配置按规模、场景、worker
排序，再左移 `7 * (round - 1)` 项；已启动运行可跨轮交叠。最多四个子进程并发，
运行中的配置 worker 之和不超过 32；这不表示总 OS 线程数或 CPU 使用上限。

每个子进程重新校验正式输入并独立安装 `TrafficWorld`，显式传递 `ExecutionConfig`，
回执记录世界实际使用的 worker 数。16 ms 的预热为 14508 拍，33 ms 为 7044 拍；
W1–W3 的观察窗口为预热长度的两倍，W4 固定观察两拍。窗口不缩短，不自动降级。

每个身份以第一轮单 worker 为参考，其余十四次逐项对照。先校验来源、输入、实际
worker、窗口和必需检查点，再仅排除回执的 `workers` 字段比较全部其他字段。
轨迹与完整有序事件逐字节相等；W3/W4 另比较命令收据和批次。原有逐拍不变量持续生效。

独立复核使用 PowerShell `Get-FileHash -Algorithm SHA256` 与 .NET `BinaryReader`，
遍历全部轨迹、事件与命令记录，核对连续拍号、事件时间、游标、记录类型、计数和
文件长度，并再次比较完整回执及文件摘要。文件互操作摘要遵循 ADR 0033，
不替换内部 BLAKE3 编码。

## 复现与材料边界

从冻结标签的干净独立检出构建并运行：

```text
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin laneflow-synth-harness
laneflow-synth-harness worker-matrix <generator-matrix-root> d81204adb8318e1521d064f9364002b6bd64fec2 <environment.json> <evidence-root>/new-matrix
```

环境声明必填字段及运行规则见[工具 README](../../tools/laneflow-synth-harness/README.md)
和[执行设计 §7](../../docs/design/lf-synth-execution.md#7-标称矩阵的跨-worker-对照)。
输出目录必须全新；首错停止启动新进程并保留已启动材料，不以补跑或拼接完成同一矩阵。

仓库仅保存本报告与小型结果摘要。全量回执、原始轨迹、环境转储、来源清单和复核
记录保留在采集机的仓库外目录，既有副本也保留；本次精简前逐文件复核了两处各
617 个文件，摘要均一致。原件没有重新序列化或以重跑结果替换。
这些本地材料不是团队下载入口，本报告不新增外部发布或长期恢复承诺；需要原件
复核时应由材料保管方提供实际访问方式。此前入库记录仍可在提交
`bbea9c744dcee10d77afb277eaee063c50195f74` 查阅，Git 历史不代表未入库原包的备份。

## 覆盖边界

这是同实现、同正式输入的成功路径 worker 扫描。逐拍轨迹覆盖执行设计规定的公开
状态，完整内部状态摘要只在保存的检查点比较。三轮重复不等于穷尽或强制调度置换。

非标称 16/33 ms 交叉组合、跨 worker failpoint 的首错与 retry/fresh、快照解码恢复、
Routing、Spatial/Adapter 的跨 worker 接线及完整五轴汇总仍未由本切片证明。
原有单 worker 重试证据保持自身来源，不能拼成跨 worker 失败路径认证。

本次不采集性能或 worker 加速比。[既有测量](../issue-1166-lf-synth-measurement/README.md)
的尾延迟、W4 秒级突发、预热后分配及硬停止线投影急减速记录保留。
跨 worker 相等不改变这些结论，不能关闭 #539，不能转为产品认证。
