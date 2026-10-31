# LF-SYNTH-v2 标称矩阵的跨 worker 失败与重试

2026-10-31，完成 [#1191](https://github.com/illusion-tech/laneflow/issues/1191) 的
F2 核验：**四个完整正常参考、60 次失败重试运行、56 对跨运行比较全部通过，零错误。**
四个标称 W3/W4 身份各使用 1/2/4/8/16 worker、三个全新测试进程；每次独立安装
失败与 fresh 两个世界。没有失败补跑、跳过或替换轮次。四行结果见
[results.json](results.json)。

包括正常参考、完整预热和失败/fresh 两世界，共完成 1,379,572 次成功世界步进，
并注入 60 次预期失败。独立复核重算 560 个运行文件的长度/SHA-256，核对 56 对
完整语义回执、448 对跨运行文件摘要和 180 对同次快照摘要；420 份规范快照及
420 个边界观测均保留。结果限于本报告的失败点与窗口，`Product TBD / Uncertified`
保持不变。

## 来源与条件

实现冻结为 `346e5c5f477d6e4248574bba4f90da5b76982650`，远程标签为
`evidence/issue-1191-worker-retry-source`。正式输入的设计来源与生成提交均为
`9029c021d1f10a7506eedbf5f2fae3b5a7d57629`。普通 release 工具和 optimized Runtime
测试程序从同一干净检出构建，复制到仓库外后执行；前后 HEAD、源码树、干净状态及
两个二进制身份均核对一致。后续报告提交不改变运行来源。

环境为 Windows 11 Insider 29695、Rust 1.99.1 / LLVM 23.1.1、
`x86_64-pc-windows-msvc`、AMD Ryzen 9 9955HX（16 核 / 32 线程），系统报告约
61.68 GiB 物理内存。两个二进制的 SHA-256 记录在小型摘要中。Cargo 可复用有效
构建缓存，不声称执行无缓存全量编译；本次也不采集性能或 worker 加速比。

## 运行与比较

先以当前普通工具生成一万/16 ms、十万/33 ms 的 W3/W4 四个完整单 worker 正常
窗口。四个参考通过后，再执行失败矩阵。16 ms 预热 14508 拍，33 ms 预热 7044 拍；
失败重试保留原有前两拍 guardrail。W3 在预热后第一拍完成并提交替换命令后注入，
W4 在 B0 命令提交后注入；W3 完整观察窗口由正常参考证明。

每个世界显式接收并核对真实 `ExecutionConfig`。每轮二十项配置按规模、场景、
worker 排序，再左移 `7 * (round - 1)` 项；已启动运行可跨轮交叠。最多四个子进程
并发，所有已启动世界（含 fresh 世界）的配置 worker 总和不超过 32；两世界交替
同步步进，此上限不表示总 OS 线程数或 CPU 使用上限。

测试专用 `AfterTransitions` 注入点返回一次 `ParkingObservationAllocFailed`，
这不是实际内存耗尽试验。核对失败后已提交状态、规范快照、事件、信号和命令游标
不变，注入点已消耗；不重放生命周期命令，只重试相同 `TickInput`。重试与独立
fresh 世界比较步进结果、事件、快照及最终状态，并对照同源正常参考的命令批次、
输入、游标和状态检查点。

每个身份以第一轮单 worker 为参考，其余十四次先验证来源、输入、实际 worker 和
窗口，再只排除回执的 `workers` 字段，精确比较其余字段。七份 LFRS 原始编码及
`observations.json` 逐字节相等，合计 56 对运行。独立 PowerShell 复核重新读取原始
JSON 并重算 SHA-256，检查时间、游标、信号覆盖、事件/到达编码和上述比较；它是
另一条文件复核路径，不是 LFRS 解码恢复测试或独立产品审阅。摘要角色遵循 ADR 0033，
原有 BLAKE3 内部编码不变。

## 复现与材料边界

从冻结标签的干净独立检出构建：

```text
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin laneflow-synth-harness
cargo +1.99.1 test --release --locked -p laneflow-runtime --lib --no-run --message-format=json
laneflow-synth-harness worker-retry-matrix <runtime-test-binary> <generator-matrix-root> 346e5c5f477d6e4248574bba4f90da5b76982650 <environment.json> <evidence-root>/new-matrix
```

从 Cargo 输出取得实际测试程序路径。环境声明、观测编码和运行规则见
[工具 README](../../tools/laneflow-synth-harness/README.md) 与
[执行设计 §8](../../docs/design/lf-synth-execution.md)。输出目录必须全新；首错停止
新启动并保留已启动材料，不以补跑或拼接完成同一矩阵。

仓库仅保存本报告与小型结果摘要。完整回执、快照、观测、日志、环境与二进制保留
在采集机仓库外；原件与副本各 638 个文件、2,280,716,040 字节，逐文件长度和
SHA-256 核对一致。原始字节没有重新序列化或用重跑结果替换。这些本地材料不是
团队下载入口，也不新增外部发布或长期恢复承诺；原件复核需由材料保管方提供
实际访问方式。

## 覆盖边界

这是同实现、同正式输入的标称 `AfterTransitions` 失败重试核验。所选失败/重试
边界的事件数组为空；非空有序事件出现在 W4 最终边界，一万规模每次 200 条、
十万规模每次 2000 条，三轮全部配置合计 33,000 条记录。所有捕获边界的停车到达
数组为空，因此不能据此声称非空停车到达顺序或非空失败前事件保持经过验证。
预热中的两世界逐拍比较步进结果与事件，完整内部状态只在保存的边界核对。

本次不证明所有失败点、多错误逻辑首错裁决、强制调度/完成置换、非标称交叉组合、
快照解码恢复、Routing、Spatial/Adapter 跨 worker 接线、性能或完整五轴认证。
三轮重复不等于穷尽调度。[F1 成功路径](../issue-1184-lf-synth-workers/README.md) 与
本次 F2 分别保留自己的实现来源，不能合称同一提交的完整五轴验收。

[既有测量](../issue-1166-lf-synth-measurement/README.md) 的尾延迟、W4 秒级突发、
预热后分配和硬停止线投影急减速记录保留。跨 worker 相等不改变这些结论，也不能
关闭父项 #539。
