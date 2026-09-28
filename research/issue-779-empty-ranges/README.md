# 空 Gate 与机动转移区间短路

Refs #779、#679、#707。

本切片在保持车辆、路线与资源合同的前提下，省去可以证明为空的 Gate 决定和
事件区间定位。无新索引、缓存、公开 API 或数据格式。

- [判据、验证与测量边界](results.md)
- [长路线三组 ABBA](evidence/route.json)
- [城市第一轮](evidence/city-first.json)、[针对波动的补测](evidence/city-repeat.json)
- [独立诊断与内存](evidence/diagnostic.txt)

普通 release 的 Runtime 不插桩；城市 host 只在公共 step 计时结束后输出计时值。
长路线使用既有普通 integration test，诊断使用单独的 unit test，不能互换解释。

## 重跑与复算

基线为 `b67c6ea9d99a8ea806fb31ede0364978e0e09618`，实测候选为
`3cbf67f2765c455ea8b57d51ffab96f073c5f7d5`，采集器为
`1070ca0a53d03410c4309fdd951faf8ec8881539`。候选之后的改动仅复用校验器、整理证据。
两个源码导出目录、构建 target 和二进制分别保存，均使用 Rust 1.98.0、release、
locked、offline、`CARGO_INCREMENTAL=0`。两臂全部构建完毕后串行采样。

Rust 入口为 `laneflow-empty-ranges-research`，复用 #763 城市采集器和 #679 长路线
矩阵校验。以下参数中的输出路径必须尚不存在：

```text
cargo run --locked -p laneflow-post-p3-research --bin laneflow-empty-ranges-research -- prepare base plain b67c6ea9d99a8ea806fb31ede0364978e0e09618 target/779-build
cargo run --locked -p laneflow-post-p3-research --bin laneflow-empty-ranges-research -- prepare candidate plain 3cbf67f2765c455ea8b57d51ffab96f073c5f7d5 target/779-build
```

分别从导出的 `<arm>-plain-source/Cargo.toml` 以独立 `<arm>-target` 构建
`laneflow-urban-harness`，把 EXE 保存成 `<arm>-plain.exe`。同一源码以
`cargo test -p laneflow-runtime --test route_query_evidence --release --locked --offline --no-run`
构建普通 integration 测试，将 Cargo JSON 指定的 executable 保存为
`<arm>-route.exe`。不要用 unit test 可执行文件代替普通构建。

```text
laneflow-empty-ranges-research route-run target/779-build target/779-route-runs
laneflow-empty-ranges-research route-analyze target/779-route-runs target/779-route-results.json
laneflow-empty-ranges-research route-verify target/779-route-runs research/issue-779-empty-ranges/evidence/route.json
laneflow-empty-ranges-research run plain target/779-build <frozen-input-root> target/779-city-runs
laneflow-empty-ranges-research analyze target/779-city-runs target/779-city-results.json
laneflow-empty-ranges-research verify target/779-city-runs research/issue-779-empty-ranges/evidence/city-first.json
```

城市补测把 `779-city-runs` 换为 `779-city-repeat-runs`，复核文件换为
`city-repeat.json`。城市输入为冻结的 #707 `4de40e04` 包，两个规模均使用既有
256 拍 probe，不修改输入或计划。长路线每个进程包含三次顺序轮换的固定矩阵，外层
再做三组 ABBA；每轴共 18 个窗口/臂，每个窗口 128 拍、暖机 16 拍、512 辆 Active。

提交的 JSON 保存源码/输入/二进制/原始文件摘要和统计；长路线还保存逐拍数值。
城市原始包与完整导出/构建保留在本机 `target/779-*`，未随 Git 发布；离开采样机器
复核城市完整原始记录需要原始包。历史 #679 制品及其固定断言保持不变。
