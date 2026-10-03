# 公共运动原语拆分对照

对应 #810；只隔离 #808 公共 prepare/finish 拆分的整拍成本。正式 Runtime、公开 API、格式、Adapter 与资源合同不变。

已完成结果见 [results.md](results.md)。本轮方向筛选未通过，不能量化归因，
`continue_candidate=false`；保留全部波动与正确性/封存证据。

## 采集前方案

- 正式基线 `fcd803f2cb28ba7a94f3f0ff589e7e9b7bdbd7d3`；与开工主线 `bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567` 的 Runtime/harness 代码无差异。
- A=`base`；B=`candidate` 只有公共 `calculate_active_vehicle_motion` 的 prepare/finish、标量 `IidmInput`、冷 `PreparedActiveMotion`，以及接收 IIDM 结果的 `si_comfort_travel_precomputed`。三段函数之外的原 tick 源码逐字保留；P5 原逐车循环、复用、首错、完整 join 与规范消费不变。
- 公共函数直接从 #808 研究变换结果取出，研究测试验证与热布局臂逐字相同；不收集四车数组，不改变导出树依赖。
- Rust 1.98.0，普通 release、locked/offline、默认目标特性，workers=4、MIXED-PEAK seed 544。
- 10000 总车辆初始 7500 Active / 2500 Parked，16 ms；100000 总车辆初始 75000 Active / 25000 Parked，33 ms。沿用同一冻结输入，每进程 256 拍，无预热。
- 各规模三组 `ABBA / BAAB / ABBA`，共 24 进程；串行单次采集，不选择性重跑。先求每进程均值与 nearest-rank p95，再平均六进程统计。全窗 1–256、进入窗 1–64、筛选窗 65–256 分列。
- 只有十万筛选窗三组方向一致且平均差值绝对值超过所有臂最大组内同臂均值跨度，才称可重复拆分成本/收益；否则不能量化归因。这是筛选判据，不是置信区间或统计显著性检验。接入候选还需十万为收益且一万 p95 回退不超过 2%。
- 正确性失败立即停止；性能不满足不扩大正式接入。未覆盖持续十万 Active、最终长窗交通与资源认证。

## 复现入口

执行逻辑在 `research/issue-762-post-p3-hotspots`；协议 `motion-split-control-v1`。

```text
laneflow-motion-split-research build-collector <new-tools>
<tools>/laneflow-motion-split-research prepare base <new-builds>
<tools>/laneflow-motion-split-research prepare candidate <builds>
<tools>/laneflow-motion-split-research run <builds> <frozen-inputs> <new-raw>
<tools>/laneflow-motion-split-research analyze <raw> <new-results>
<tools>/laneflow-motion-split-research verify <raw> <results>
```

两臂从同一清空后环境构建，绑定 Git 归档、逐文件源码、实际工具链/命令/EXE。分析重派生矩阵、来源、交通与逐拍状态，检查已知争用及统计身份；原始载荷完整封存后全新解压逐文件验封。

实际采集提交为 `354bd6023561e027defb57aa0f401ed0426e0bb0`，保留在
`codex/810-motion-split-source-freeze`。prepare/run 使用该干净提交；报告 PR 和
合并提交不替代它。解压归档的 verify 无需当前工作树匹配。

## 证据保存

仓库保留源码、采集前方案、结果报告、小型统计摘要与验封索引。完整源码清单、
构建环境、逐进程结果和原始日志放在仓库外归档，不随每个研究切片重复入库。
归档下载地址、长度和 SHA-256 见 [archive.toml](evidence/archive.toml)；全部六个
窗口的统计摘要见 [summary.json](evidence/summary.json)。摘要不替代原始证据，
也不能作为采集器 `verify` 的输入。

完整原始证据与发布凭据已保存为 [Research Evidence Release](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-810-motion-split-354bd602)。原始归档字节与封存一致；发布凭据 ZIP 同时保留完整 JSON 原件及本次迁移前的报告与索引。下载后核对 `archive.toml` 的摘要，再把原始归档解压到全新目录，使用以下入口复核：

```text
<unpacked>/tools/laneflow-motion-split-research verify-collector <unpacked>/tools
<unpacked>/tools/laneflow-motion-split-research verify <unpacked>/raw <unpacked>/results.json
```

验封仍需按归档 `seal.json` 核对全部文件的路径、长度、SHA-256 与数量；
公开下载的两份资产均已核对长度和 SHA-256；全新解压核对了全部 6671 个载荷，采集器及 24 轮结果重派生通过。详细验封和负向验证凭据从 ZIP 的 `current-pr/evidence/` 查阅。
