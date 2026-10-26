# LuST 跨边方向调查与采样候选

研究问题：现行修复程序成功发射一条边，是否足以保证它与真实机动路径的相邻边通过
compiler 的方向检查？在保持源几何的前提下，后续有界采样应如何定义成功与失败？
任务入口为 [#1084](https://github.com/illusion-tech/laneflow/issues/1084)，父问题为
[#790](https://github.com/illusion-tech/laneflow/issues/790)。

## 结论与边界

2026-10-26 在代码 `c62567b7e45d1460936a83dbe21d9b2e22072e14`、Rust 1.99.1、
Windows x86_64 上调查 pinned LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`。
转换入口核验全部 8 个 pinned 文件的 revision、大小及摘要。

- 24,495 条车道图边中，16,265 条真实发射成功，8,230 条发射失败。
- 14,098 条机动路径包含 29,971 个相邻位置；本输入的去重有向边对数也为 29,971。
- 两侧发射均成功的 14,890 对进入真实 compiler 的最小夹具：14,887 接受，3 对方向失败。
  其余 15,081 对未完成目标检查；占位程序不参与验收。
- 3 对失败为 `sumo:--32668#0_0 → sumo::-19726_1_0`、
  `sumo:-32668#0_0 → sumo::-25490_3_0`、`sumo::-8018_0_1 → sumo:-32770#1_1`。

边内发射通过不足以保证跨边方向通过。夹具保留两条目标边的规范点和接缝，但添加人工
lead；其 accepted 只证明该边对在夹具中的受检性质，不证明真实完整路径或全网通过。
现行失败数量也不证明硬预算下无解。

[S1 候选设计](s1-design.md) 要求同一候选在保留真实入口、全部内部边、出口、approach
和控制锚点的完整路径中接受编译。候选搜索预算、偏离/长度合同以及原成功边改动范围
仍须 G1 决定；本研究没有实现 S1、GE1、换道或 Runtime 能力。

## 保存的审阅输入

- [summary.json](summary.json)：来源身份、全部计数、夹具边界。
- [samples.json](samples.json)：17 条真实样本的输入原始位、重放结果及实际拒绝调用摘录。
  省略成功子调用和完整程序，不代表完整执行轨迹；父调用 ID 保留原始编号。
- [determinism.json](determinism.json)：两个独立进程的五个结果文件经直接字节比较相同；
  SHA-256 用于 GitHub 读者以 PowerShell 等外部工具核对生成文件，不作为模型性能结论。

每个成功程序导出完整有序段类型、f64 坐标位和画布选择键，Line 程序另含全部有序
f32 点位。完整基线覆盖 16,265 个程序、670,966 个量化点。普通回归确认改变中间点会
改变记录，即使段数和首末点相同。另有曲线端点误差为零而内部偏离 0.75 m 的反例。

约 153 MB 的逐边/逐对/逐位置记录和原始日志保留在本次本地交付中，不提交源码树，
本目录不承诺提供其下载或长期恢复；可用下述源码入口重新调查。摘录是重新序列化的
审阅材料，不声称与原始 JSONL 字节相同。

## 重建

实现位于 [s0f.rs](../../tools/laneflow-lust-converter/src/output/geom/tests/s0f.rs)，
复用生产转换和真实 compiler。测试观察器仅在 `cfg(test)` 下存在；采样算法和阈值不变。

先设置 `LUST_SOURCE_DIR` 为上述 pinned 来源目录，`S0F_OUTPUT_DIR` 为一个空的输出
目录，然后在 checkout 根执行：

```text
cargo +1.99.1 test --locked -p laneflow-lust-converter --lib output::geom::tests::s0f::pinned_lust_path_adjacency_survey -- --exact --ignored --nocapture
```

更换为空的新输出目录再执行一次；比较 `summary.json`、`pairs.jsonl`、`positions.jsonl`、
`edge-emissions.jsonl`、`sampling-samples.jsonl`。`run-meta.json` 包含时间，不要求相同。
不执行全部 ignored 测试，也不运行重写现有 manifest 的 regenerate 入口。

普通回归与研究检查：

```text
cargo +1.99.1 test --locked -p laneflow-lust-converter --lib
cargo +1.99.1 fmt --all -- --check
cargo +1.99.1 check --locked -p laneflow-lust-converter --tests
cargo +1.99.1 clippy --locked -p laneflow-lust-converter --all-targets -- -D warnings
```

关联约束见 [真实路网工作负载](../../docs/design/real-road-workloads.md)、
[ADR 0022](../../docs/adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md) 和
[ADR 0028](../../docs/adr/0028-integer-millimeter-traffic-geometry.md)。
