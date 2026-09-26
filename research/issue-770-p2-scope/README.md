# 同拍可达性收窄 P2 预览

Refs #770。复用 #763 已计算的同拍可达性，在原检查和距离过滤之后，仅对
`Some(false)` 且没有 Waiting membership 的车辆省略 P2 运动预览。保留 horizon，
未知、缺失和持有资源的路径继续完整计算，P5 完整求值最终运动。

- [采用证据与局限](results.md)
- [无插桩 24 轮](evidence/plain.json)
- [独立诊断 12 轮](evidence/detail.json)
- Rust 工具：`research/issue-762-post-p3-hotspots/src/p2_scope_research.rs`；共享
  `cache_research.rs`，保留 #763 的固定基线、协议与历史运行顺序。

## 复现

从干净且包含基线、候选和工具提交的 checkout 导出。`base` 固定为
`7bdf1f0ee4ae436ffc688903899ce9d16f89e41b`，本次候选为
`2e98ad2afcf549d18325d62af7b6840c925ea938`。输入是冻结的 #707 `4de40e04`
外部包；四个构建目录独立，全部构建结束后再串行运行。

```text
cargo run -p laneflow-post-p3-research --bin laneflow-p2-scope-research -- prepare base plain 7bdf1f0ee4ae436ffc688903899ce9d16f89e41b target/770-build
cargo run -p laneflow-post-p3-research --bin laneflow-p2-scope-research -- prepare candidate plain 2e98ad2afcf549d18325d62af7b6840c925ea938 target/770-build
```

对 `detail` 重复导出，在各导出树以 Rust 1.98.0、release/locked/offline、
`CARGO_INCREMENTAL=0` 构建 `laneflow-urban-harness`，为四树分别指定
`CARGO_TARGET_DIR`，复制对应 exe 至 `target/770-build/<arm>-<mode>.exe`。

```text
laneflow-p2-scope-research run plain target/770-build <frozen-input-root> <new-plain-raw>
laneflow-p2-scope-research analyze <new-plain-raw> <new-plain-results.json>
laneflow-p2-scope-research verify <new-plain-raw> <new-plain-results.json>
```

对 `detail` 重复采集和校验。输出必须为新路径，工具拒绝覆盖，保留失败与完整轮次。
原始包和导出源码未随 Git 发布；提交的 JSON 是有完整文件摘要的复算结果。
本项是内部优化验收，不完成 #707 最终性能或长期交通质量认证。
