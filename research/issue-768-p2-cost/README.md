# P2 预览与 Waiting 准备成本

Refs #768、#707。基线固定为 #763 合入的 `7bdf1f0ee4ae436ffc688903899ce9d16f89e41b`。
使用既有 Rust package 中的 `laneflow-p2-cost-research`；不修改历史 #762/#763 协议或证据。

成本排名、运行间波动和下一候选见[冻结结果](results.md)。

## 有界问题

WaitingPrepare 包含初始化、P2 独立预览与规范组装。分别计时发现、槽位准备、
计算/join、规范消费、融合路径与 Waiting 组装，判断主要成本是否可以减少。
每个导出诊断批次记录 Entries、NoGate、Outside、Preview、Horizon、GateCalc。
计数在线程本地累积、每块归约一次；不在每车计算中放时钟或共享原子。
协调器批次墙钟包含完整 join，不相加 worker CPU，也不把父子段重复相加。

两尺度各 plain/detail/detail/plain/plain/detail，256 拍、workers=4。实际 Active、
1–64 与 65–256 分窗单列；不是正式暖机、稳态或性能认证。
无插桩程序只在 Harness step 计时外记录时间，Runtime 源码保持基线原样。
诊断程序只在隔离导出树注入，不进入正式 Runtime。

```powershell
cargo +1.98.0 run --locked --offline -p laneflow-post-p3-research --bin laneflow-p2-cost-research -- prepare plain target/768-build
cargo +1.98.0 run --locked --offline -p laneflow-post-p3-research --bin laneflow-p2-cost-research -- prepare detail target/768-build
```

两树使用独立 Cargo target 目录，`CARGO_INCREMENTAL=0`、release/locked/offline。
把各自 `laneflow-urban-harness.exe` 复制为 `768-build/{plain,detail}.exe`，所有构建
结束且采集工具提交干净可达后再串行运行，期间不构建/修改源码：

```powershell
target/debug/laneflow-p2-cost-research.exe run target/768-build E:/projects/laneflow-evidence/issue-707/4de40e04 target/768-runs
target/debug/laneflow-p2-cost-research.exe analyze target/768-runs target/768-results.json
target/debug/laneflow-p2-cost-research.exe verify target/768-runs target/768-results.json
```

原始包保留在独立工作树 target，封存结果包含 source/manifest/lock、二进制、输入、
执行 UUID、前后 collector HEAD/clean、原生输出摘要和完整 raw 文件清单。
分析器拒绝身份、行序、时钟形状、父子嵌套、计数路径及封存结果差异，不覆盖证据。
正式整拍使用 plain；detail 仅用于成本归因，不能把诊断局部成本当成可兑现收益。

后续实现须独立立项，验证身份代次、近门/跨边、资源持有、容量退化、失败与重试，
并以两尺度平衡整拍结果判断采用。100k 正式长测不作为本项门禁。
