# 保持正式行为的路线阶段诊断

问题：现行路线错误不能区分道路 hop 缺失和可达车道衔接缺口；直接把全部归一化失败
归为来源引用错误，又会误报合法引用下不支持的拓扑。任务入口为
[#1085](https://github.com/illusion-tech/laneflow/issues/1085)，父问题为
[#806](https://github.com/illusion-tech/laneflow/issues/806)。

诊断入口复用正式展开和物化算法的观察钩子，按道路 occurrence 推进全部可达车道
集合，定位第一次整体耗尽的位置。保留人口和来源位置、候选机动路径与原始错误。
正式入口使用空观察器，不调用额外诊断入口；本研究没有增加隐式换道。

## 结果与计量

2026-10-26 在代码 `4ace80110c985e175840c3314f2e265b7730d4c3`、Rust 1.99.1、
Windows x86_64 上导出 15 个案例，11 组 A0 正式行为对照全部相同。覆盖道路 hop
缺失、车道连续性缺口、先死后成功分支、多起点车道、重复道路 occurrence、来源错误、
真实不支持的 stub 续链、物化故障注入，以及连续小夹具的组合静态转换。

来源校验失败未进入展开时，`expansionCalls=0`。stub 携带真实续链的拒绝归为
`Normalization / Unclassified`，无虚构人口上下文。错误文本和正式返回行为保持原样。
计数只包含诊断观察器的执行；整次诊断计时还包含建索引、正式基线调用、绑定和排序。
详细定义见 [诊断口径](diagnostic-contract.md)。单次耗时不作为性能结论。

- [results.json](results.json)：15 个实测案例、11 组正式行为比较和未执行阶段。
- [baseline-results.json](baseline-results.json)：已集成 A0 提交
  `85a2d383f1cd66d8d8d04a14ce7d7a049172ed42` 的固定对照输入。

这两个文件保留语义数据，构建目录和本机命令改为占位并重新序列化，不声明与本地
原件字节相同。当前入口继续对固定 A0 输入执行比较，不能用当前代码重新生成的基线
代替历史基线来证明正式行为未变。

## 重建

设置 `LANEFLOW_806_A0_BASELINE` 为本目录 `baseline-results.json`，
`LANEFLOW_806_A1_RESULTS` 为 `<evidence-root>/results.json`，在 checkout 根执行：

```text
cargo +1.99.1 test --locked -p laneflow-lust-converter --lib convert::routes::diagnostics::tests::evidence::a1_route_evidence -- --exact --nocapture
cargo +1.99.1 test --locked -p laneflow-lust-converter --lib
cargo +1.99.1 fmt --all -- --check
cargo +1.99.1 check --locked -p laneflow-lust-converter --tests
cargo +1.99.1 clippy --locked -p laneflow-lust-converter --all-targets -- -D warnings
```

代码入口为 [diagnostics.rs](../../tools/laneflow-lust-converter/src/convert/routes/diagnostics.rs)
和 [evidence.rs](../../tools/laneflow-lust-converter/src/convert/routes/diagnostics/tests/evidence.rs)。
不修改公开 API、CLI/schema、LFCA、路线选取、排序、截断、绑定或依赖。
完整 LuST TOPO/DEMAND、Runtime 注册/准入及安全验证未执行；本结果不表示全网路线可用。
归属边界见 [ADR 0029](../../docs/adr/0029-retire-precompiled-static-route.md)。
