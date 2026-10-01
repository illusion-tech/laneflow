# 整拍 SIMD 数据流与直接热输入研究

对应 #808，正式基线 `fcd803f2cb28ba7a94f3f0ff589e7e9b7bdbd7d3`。
研究工具建立在 #807 的已冻结工具提交 `8a152d6614932f915268d65179e97a919428bb34`
之上；#807 已合并，本轮保留它的原始证据。已完成结果见 [results.md](results.md)。

本轮先记录整拍的数据与依赖，再执行 P5 最小布局切片。设计见
[data-flow.md](data-flow.md)。正式 Runtime、公开 API、格式和已接受并行合同不变。

## 采集前方案

- A=`base` 原 P5；B=`layout` 直接热字段标量；C=`candidate` 同布局稀疏 SIMD。
- 准备阶段按需直接写入字段数组，移除四车 `IidmInput` 中间数组以及冷准备对象内
  的 IIDM 副本。复用预览的车辆不写输入，全复用批次不初始化/调用 IIDM。
  内核借用热字段；至少两个可向量化通道才使用四路 SIMD，其他通道按原标量处理。
  复用结果与待算冷字段分别暂存，复用车辆不经过大尺寸计算状态枚举；分离数组仍有
  栈空间和判别位成本，需在整拍测量中验证。
  两个布局臂沿用公共运动原语的准备/完成拆分；融合路径与 Waiting 预览保持标量，
  但也经过该拆分，因此布局对照不是单独测量 P5 输入收集。
- Rust 1.98.0，`wide = 1.5.0`，默认目标特性，无全局 AVX2、FMA、fast-math 或近似倒数。
  普通 release、locked/offline、workers=4、MIXED-PEAK seed 544。
- 10000 总车辆初始 7500 Active / 2500 Parked，16 ms；100000 总车辆初始
  75000 Active / 25000 Parked，33 ms。每进程 256 拍，无预热。
- 全窗 1–256、进入窗 1–64、筛选窗 65–256 分列；先求进程均值和 nearest-rank p95，
  再平均六个进程的统计。
- 每规模三组六进程：`ABC CBA / BCA ACB / CAB BAC`；共 36 个进程。
  全部构建和回归完成后串行采集，不选择性重跑或删除轮次。
- 只有十万筛选窗候选相对基线三组均改善、平均改善超过所有臂最大组内同臂均值
  跨度，且一万平均进程 p95 回退不超过 2%，才列为后续接入候选。
  布局与 SIMD 各自贡献分列；没有每项优化必须改善 2% 的要求。
- 若数值、交通、状态或原子性不一致，保留失败证据并停止候选；性能不满足则停止
  本轮接入，不追加正式长窗。本轮不等于 #707 最终性能/交通/资源认证。

## 复现入口

执行逻辑位于 `research/issue-762-post-p3-hotspots`，协议为 `p5-hot-input-simd-v1`。

```text
laneflow-p5-hot-simd-research build-collector <new-tools>
<tools>/laneflow-p5-hot-simd-research prepare base <new-builds>
<tools>/laneflow-p5-hot-simd-research prepare layout <builds>
<tools>/laneflow-p5-hot-simd-research prepare candidate <builds>
<tools>/laneflow-p5-hot-simd-research run <builds> <frozen-inputs> <new-raw>
<tools>/laneflow-p5-hot-simd-research analyze <raw> <new-results>
<tools>/laneflow-p5-hot-simd-research verify <raw> <results>
```

采集器从干净提交 Git 归档受控构建，绑定实际 EXE、源码、工具链、命令、清空后环境
与日志。三臂分别使用全新外部目标目录。分析重派生全部矩阵、来源、交通和状态摘要，
拒绝缺失成员、争用、构建/采集器身份漂移和发布统计不一致。原始数据封存并全新解压验封。

实际采集提交为 `70bfbffbdbb35aafede89fdc63f738b62d918da8`，保留在
`codex/808-hot-input-source-freeze`。重现 prepare/run 时使用该干净提交；报告 PR 的
主线重放和后续文档提交不替代它。解压归档后的 verify 无需重建或当前工作树匹配。
