# P5 按 Active 投影直接分发

> **证据归档**：本目录的历史诊断 JSON 已迁入[冻结归档](../archives/2026-10-01-json-migration.md)。
> 本文的 JSON 链接指向原提交；文中相对 JSON 路径及依赖它们的历史命令按归档内 `source/` 目录解释。

Refs #775、#707。

本切片删除 P5 的 `(VehicleHandle, active_index, VehicleState)` 输入表。Pool 路径
直接按同拍 `active_order` 分块，任务在对应紧凑位置读取完整句柄并核对代次；结果
仍写入独占槽位，完整 join 后由协调器按 Active 序执行停车到达预留与 next state
接纳。句柄失效用 `Done(Ok(None))` 保留融合路径的跳过语义，不与调度器的
`Pending` / `Skipped` 混用。

- [采用证据与局限](results.md)
- [无插桩平衡 A/B](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-775-p5-direct-active/evidence/plain.json)
- Rust 采集器：
  `research/issue-762-post-p3-hotspots/src/p5_direct_research.rs`

## 复算入口

基线固定为 `2b5431201bb64d82f38ea8228076c54941fd2af3`，实测候选为
`baa60104d4ae93c4ee796887253c7e95bd5667bd`，采集器提交为
`2345536b353bd71174690def6bcd740d5379892f`。输入使用冻结的 #707
`4de40e04` 包。导出与构建目录彼此独立，两个 release 二进制全部构建结束后才
开始串行采集。

```text
cargo run -p laneflow-post-p3-research --bin laneflow-p5-direct-research -- prepare base plain 2b5431201bb64d82f38ea8228076c54941fd2af3 target/775-build
cargo run -p laneflow-post-p3-research --bin laneflow-p5-direct-research -- prepare candidate plain baa60104d4ae93c4ee796887253c7e95bd5667bd target/775-build
cargo run -p laneflow-post-p3-research --bin laneflow-p5-direct-research -- run plain target/775-build <frozen-input-root> target/775-plain-runs
cargo run -p laneflow-post-p3-research --bin laneflow-p5-direct-research -- analyze target/775-plain-runs target/775-results.json
cargo run -p laneflow-post-p3-research --bin laneflow-p5-direct-research -- verify target/775-plain-runs target/775-results.json
```

两棵导出源码分别使用 Rust 1.98.0、`--release --locked --offline`、
`CARGO_INCREMENTAL=0` 和独立 target 目录构建 `laneflow-urban-harness`。
原始进程包与导出源码保留在本机 `target/775-plain-runs`、`target/775-build`，
未随 Git 发布；提交的 JSON 保存源文件、输入、二进制和全部原始文件摘要并可严格
复算。本项是 1.0 前内部 Runtime 优化验收，不完成 #707 最终性能认证。
