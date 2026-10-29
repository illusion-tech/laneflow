# LF-SYNTH 执行核验

消费 [`laneflow-synth-generator`](../laneflow-synth-generator/README.md) 的正式四文件
输入。交通合同来自 [性能基线 §4](../../docs/design/core-runtime-performance-baseline.md)，
消费及证据编码见 [执行核验设计](../../docs/design/lf-synth-execution.md)。

## 运行

```powershell
cargo +1.99.1 build --release --locked -p laneflow-synth-harness
```

单个输入的完整窗口：

```text
laneflow-synth-harness run <input-directory> <implementation-commit> <new-output-directory>
```

`implementation-commit` 必须是构建本二进制的完整提交 SHA。使用独立干净工作树从该
提交构建；参数仅记录调用者声明，不替代构建来源证明。输入 manifest 的设计来源和
生成提交保留原值，不因消费实现变化而改写。

W1/W2 八行矩阵及每行两个独立进程：

```text
laneflow-synth-harness matrix <generator-matrix-root> <implementation-commit> <new-output-root>
```

W3/W4 使用同样的输入布局及两轮独立进程：

```text
laneflow-synth-harness burst-matrix <generator-matrix-root> <implementation-commit> <new-output-root>
```

输入根目录采用生成器矩阵布局，例如 `10000-W1-16-a`、`100000-W2-33-a`。
每个消费进程都重新验证四文件和重编译结果。按规模、场景、16/33 ms 排队，
最多四个 fresh-process 消费进程并发，每行两轮；原始日志分别保存。这只是语义
对照，不能用并发进程的耗时评价性能。输出目录必须不存在。

## 结果与边界

- `report.json`：完整输入 manifest、实现提交、窗口、实际计数、逐拍轨迹摘要、初始/
  预热结束/H/2H/4H/最终检查点；成功才有 `complete=true`。
- `ticks.bin`：包含初始边界的全部逐拍公开观测摘要，矩阵逐字节比较两份轨迹。
- `summary.json`：矩阵已通过行、完整回执、独立进程一致性；中断/失败保持
  `complete=false`，已写材料保留，不覆盖或跳过失败行。

W1/W2 初始化后只调用 `step` 和只读查询。W3/W4 按受检计划执行停车和生命周期
命令，实际收据写入 `commands.bin`，批次前后状态写入 `batches.json`；矩阵同时
逐字节比较这些文件。W3 保留到达屏障与单拍入位，W4 在 B1 后立即结束。
所有运行固定单 worker，不隐式回收、替换或缩短窗口。
每拍检查人口/身份、停车占用、精确整数运动、重叠、信号与事件因果。
完整状态摘要只在检查点生成；逐拍轨迹覆盖设计列出的公开字段，不宣称逐拍内部状态
全部受检。完整窗口执行会较长，进度每 1024 拍写入 stderr。

这是包含大量宿主核验的语义运行，不是正式性能测量。表现选择、外层帧、计时/分配/
内存、worker 排列、快照恢复、Routing 与五轴认证不由本工具完成。
两轮用于确定性对照，不替代性能合同的三轮。`Product TBD / Uncertified` 保持不变。

## 失败重试

先从同一提交构建 optimized Runtime test binary；普通生产二进制没有 failpoint：

```powershell
cargo +1.99.1 test --release --locked -p laneflow-runtime --lib lf_synth_contract --no-run
```

从 Cargo 输出取得 `laneflow_runtime-<hash>` 测试程序的真实路径，再用**已完成的**
同提交 W3/W4 正常矩阵运行：

```text
laneflow-synth-harness retry-matrix <runtime-test-binary> <generator-matrix-root> <implementation-commit> <normal-matrix-output> <new-output-root>
```

八行各启动一个新测试进程，最多两进程并发；每个测试独立安装两个世界，先走完整
预热，保存命令前后规范快照，在 `AfterTransitions` 注入一次失败，然后只重试相同
step。W3 运行到前两条观察拍，W4 完成 B1；完整 W3 窗口由正常矩阵证明。

输出包含 `report.json`、七份 `.lfrs` 捕获及测试日志。矩阵比较失败前后、独立世界
的命令后状态和 retry/fresh 快照字节，还将输入、批次摘要、游标和最终状态与正常
矩阵对应检查点比较。原始快照用于本次逐值/字节比较，不代表恢复认证；测试构建
耗时不作 latency/tail 结论。失败保留 `complete=false` 及已写材料。

## 回归

```powershell
cargo +1.99.1 test --locked -p laneflow-synth-harness
cargo +1.99.1 clippy --locked -p laneflow-synth-harness --all-targets -- -D warnings
```

默认测试覆盖两步长相位边界、正式一万人口初始化与短序列、错误状态拒绝、重复
到达/预约拒绝、屏障与窗口边界、输入顺序摘要、输出保留、轨迹/检查点/重试证明
篡改比较和并发子进程失败收集。长窗口与正式失败重试由上述三个矩阵入口显式执行。
