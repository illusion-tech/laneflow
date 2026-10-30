# LF-SYNTH-v2 标称矩阵的跨 worker 确定性

2026-10-31，**120/120 个完整窗口、112/112 对语义比较全部通过，零运行或比较错误。**
八个标称身份的五种 worker、三轮独立进程均完成；本矩阵没有失败补跑、跳过或替换轮次。
独立复核重算 480 个文件的 SHA-256，112 对完整回执与 336 对文件摘要全部一致。

共完成 3,232,860 次成功步进，遍历 3,232,980 条拍记录，核对 3,316,500 次过门与
198,000 条命令收据，并保留 780 个状态检查点记录。运行前后 527 个来源文件、HEAD/源码树、干净状态与二进制身份
保持一致。该结果仅覆盖下述成功路径 worker 扫描，`Product TBD / Uncertified` 保持不变。

## 来源与运行方法

本记录对应 [#1184](https://github.com/illusion-tech/laneflow/issues/1184) 的 F1 子切片。
实现固定为 `d81204adb8318e1521d064f9364002b6bd64fec2`，远程源码标签为
`evidence/issue-1184-worker-source`；输入的设计来源与生成提交均为
`9029c021d1f10a7506eedbf5f2fae3b5a7d57629`。后续证据文档提交不重标运行来源。

每个子进程重新校验正式输入、三个文件摘要、规范编码与重编译结果，独立安装
`TrafficWorld`，将显式 `ExecutionConfig` 传入原有 W1–W4 全窗口核验路径。
回执的 worker 数取自安装后的世界。不改输入、交通配置、命令计划或窗口，不自动降级。

一万/16 ms、十万/33 ms 的 W1–W4 共八个身份，各用 1/2/4/8/16 worker，
每个配置三个全新进程。每轮四十项配置按规模、场景、worker 排序，并左移
`7 * (round - 1)` 项；轮次之间可有已启动运行交叠。最多四个子进程并发，
运行中配置 worker 之和不超过 32。这只是宿主调度限制，不代表总 OS 线程数或
CPU 使用上限，也不是单世界吞吐或性能结果。

| 规模   | 场景 | 步长 ms | 每次预热拍数 | 每次观察拍数 | 每次核验边界数 |
| ------ | ---- | ------- | ------------ | ------------ | -------------- |
| 10000  | W1   | 16      | 14508        | 29016        | 43525          |
| 10000  | W2   | 16      | 14508        | 29016        | 43525          |
| 10000  | W3   | 16      | 14508        | 29016        | 43525          |
| 10000  | W4   | 16      | 14508        | 2            | 14511          |
| 100000 | W1   | 33      | 7044         | 14088        | 21133          |
| 100000 | W2   | 33      | 7044         | 14088        | 21133          |
| 100000 | W3   | 33      | 7044         | 14088        | 21133          |
| 100000 | W4   | 33      | 7044         | 2            | 7047           |

## 比较与独立复核

每个身份以第一轮单 worker 作为参考，其他十四次运行逐项对照。比较前校验
实际 worker、实现提交、完整输入 manifest、正式窗口和必需检查点；只排除
完整回执的 `workers` 字段。`ticks.bin` 与 `events.bin` 逐字节比较；W3/W4 另比较
`commands.bin` 与 `batches.json`。每个运行持续核验原有逐拍语义不变量。

独立复核使用 PowerShell `Get-FileHash -Algorithm SHA256` 与 .NET `BinaryReader`，
遍历每个轨迹和事件文件的全部连续拍号，核对事件时间、命令游标、记录类型、
事件计数和完整文件长度；遍历全部命令的尝试序号、boundary、游标与成功/阻塞位，
再核对批次文件和回执。重算所有运行文件的 SHA-256，并再次比较完整回执及
同身份文件摘要。摘要为 ADR 0033 的文件互操作摘要，不替换内部 BLAKE3 编码。

## 构建与证据

Windows x86-64，Rust 1.99.1 / LLVM 23.1.1，AMD Ryzen 9 9955HX、16 核 / 32 线程、
系统报告约 61.68 GiB 物理内存。完整 OS/工具链以环境 JSON 为准。普通 release 二进制从干净冻结
提交构建、复制到工作树外，再由同一文件启动所有子进程。其 SHA-256 为
`12764da12cde4631e61fbef6bea7fd36b250d311e5920208b3e6f82b0cd39ba3`。
Cargo 可复用有效构建缓存，不声称执行了无缓存全量编译。

- [矩阵回执](summary.json)：全部运行、完整输入 manifest、worker/轮次、进程启动
  次序、语义回执、文件长度/SHA-256、比较结果和环境。
- [独立复核](external-audit.json)：完整记录遍历和逐文件摘要重算结果。
- [环境](environment.json)与[构建来源](build-provenance.json)：冻结提交、源码树、
  构建命令/退出码、二进制和构建日志身份。
- [来源文件](source-files.json)与[运行后核对](post-run-source-verification.json)：
  527 个来源文件，以及运行前后的 HEAD、源码树、干净状态与二进制核对。
- [实现提交 CI](implementation-checks.json)：冻结实现的九项必需检查及精确提交、
  检查应用身份和结果链接；PR 最终证据提交的检查以 GitHub 当前状态为准。
- [原始材料副本复核](archive-verification.json)：617 个文件共 526697927 字节，
  原始目录与保存副本的每个文件 SHA-256 均一致；含完整和中间材料各自的状态。

全部原始目录、冻结二进制、日志及主机复核脚本保存在
`E:\projects\laneflow-evidence\issue-1184-d81204ad`；仓库保存可审查的 JSON 清单与报告，
不把二进制轨迹作为生产输入入库。
本目录的原始 JSON 禁用 Git 换行规范化，保留采集字节，使来源清单中的文件摘要
可直接针对仓库文件复算。
逐文件复核的副本位于 `E:\projects\laneflow-evidence-backups\issue-1184-d81204ad-complete`。

从冻结标签的干净独立检出构建：

```text
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin laneflow-synth-harness
laneflow-synth-harness worker-matrix <generator-matrix-root> d81204adb8318e1521d064f9364002b6bd64fec2 <environment.json> <new-output-root>
```

环境声明的必填字段和完整运行规则见
[工具 README](../../tools/laneflow-synth-harness/README.md)与
[执行设计 §7](../../docs/design/lf-synth-execution.md#7-标称矩阵的跨-worker-对照)。
首错停止启动新进程，保留已启动材料；不得通过补跑、跳过或拼接来完成同一矩阵。

## 覆盖边界

这是同实现、同正式输入的成功路径 worker 扫描。逐拍轨迹覆盖执行设计规定的
公开状态，完整状态摘要只在保存的检查点比较；不声称每拍全部内部状态都已比较。
三轮重复运行不等于穷尽或强制 schedule/completion permutation。

非标称 16/33 ms 交叉组合、跨 worker failpoint 的首错与 retry/fresh、快照解码恢复、
Routing、Spatial/Adapter 的跨 worker 接线，以及完整五轴汇总仍未由本切片证明。
原有单 worker 错误重试证据保持其自身来源，不能与本次结果拼成跨 worker 失败路径认证。

本次不采集性能、不计算 worker 加速比。此前[表现与资源测量](../issue-1166-lf-synth-measurement/README.md)
的尾延迟、W4 秒级突发和预热后分配负面结果保留；回执中的硬停止线投影急减速计数
也保留。跨 worker 相等不能改写这些结论，不能关闭 #539。
`Product TBD / Uncertified` 保持不变。
