# LuST v2 人口迁移与诊断重建

关联 [#1053](https://github.com/illusion-tech/laneflow/issues/1053)；设计依据为已接受的
[#1051](https://github.com/illusion-tech/laneflow/issues/1051) 与
[`real-road-workloads.md` §4.1、§7.1](../../docs/design/real-road-workloads.md)。

本切片落实共享人口选取与报告身份，不交付 TOPO/DEMAND plan、实际初始位置或完整
可运行工作负载。来源和制品的 SHA-256 校验保持不变。

## 固定合同与基线

- 主键为 `XXH3_64bits_withSeed(UTF-8("LF-REAL-LUST-POPULATION-v2\0") || UTF-8(ID), 0)`，
  标准 secret，无符号 u64 升序；碰撞后按完整原始 ID 字节排序。
- 每候选只计算一次，拼接缓冲区在单次选取内复用；不假定 ID 只有固定来源的
  7–14 字节，不提供生产算法选择或旧路径回退。
- 三个固定来源文件共 215,526 条源记录，过滤后 10,592 个候选，选取 10,000 条。
- 报告 `formatVersion=0.3`，`population.derivationId=LF-REAL-LUST-POPULATION-v2`。
- [机器可读基线](baseline.json)固定完整人口表、路线失败诊断、转换报告和几何诊断
  的 SHA-256；忽略测试直接消费该基线，不能只改文字而不重建证据。

与旧 SHA 人口共有 9,443 条，移出/移入各 557 条；共同记录中 9,442 条 rank 改变，
1 条 rank 相同。候选主键未出现碰撞。新人口复制 375,891 个 edge 字符串，旧 SHA
人口为 377,108 个，因此完整调用的耗时变化还包含集合和复制量变化。

新人口有 **9,344 条路线无法车道级展开**，旧 SHA 对照为 9,350 条；差值不代表交通
质量提升。诊断逐条调用现有生产边引用校验与路线展开；批量生产入口仍遇错即拒绝。
不改 seed、扩窗、跳过错误或补位。完整转换只交付既有诊断集合，不生成
`routes.toml`、`network.lfca` 或 `lust-static.tar`；几何诊断仍与既有仓库基线一致。

`baseline.json` 还保留候选键的 16 箱计数、入选类型、来源文件和每 300 秒出发时间
分箱。它们是固定样本的描述，不证明等概率抽样、独立性、代表性或交通公平性。

## 验证入口

设置 `LUST_SOURCE_DIR` 为固定 commit
`c4bd5bd3751d426d42a9a1749c815e47ea188549` 的 LuST checkout。每个独立进程设置
不同且尚不存在的 `LUST_MIGRATION_OUTPUT_DIR`，运行：

```text
cargo +1.99.1 test --release --locked -p laneflow-lust-converter --lib convert::population::migration::pinned_lust_population_migration -- --exact --ignored --nocapture
```

测试先验证全部固定文件、消费时重核字节并重查 revision；比较生产与参考选取的全部
字段、反转输入后的输出，再写出 `population.json`、`route-failures.json` 和
`summary.json`。完整人口表包含顺序、rank、ID、类型、精确 departure 文本、完整
road-edge 序列及两个 source ordinal。`converted/` 保留真实转换报告、provenance、
几何诊断与来源包。大体量来源和人口数据不提交 Git。

同一构建的两个 fresh process 应逐字节比较人口表、路线失败、summary、转换报告、
几何诊断和 semantic provenance。build provenance 的原始配置摘要可因输出目录而
不同；报告 SHA-256 必须同时与 semantic provenance 和 build provenance 的记录
一致，测试已对此断言。

普通测试覆盖 §4.2 独立 C 参考实现的三个固定人口向量、Unicode、任意长度原始 ID、
输入置换、强制主键碰撞时的完整 ID 次键和每候选一次计算。空/重复 ID、departPos、
缺少路线、源字节变化与源 revision 失败继续由原有测试覆盖。

## 测量边界

另设 `LUST_POPULATION_BENCH=1` 才启用 release 局部测量。使用同一真实候选集对照
历史无前缀 SHA 与最终 v2 前缀的生产 XXH3；后者包含实际拼接，复用缓冲区。
完整选取对照包含 215,526 条记录过滤、排序、截断、入选校验、一万条记录复制及
输出释放，不含来源校验、XML 解析、路线展开、静态编译或报告写出。

每项先校准至约 40 ms 或 256 次上限，随后轮转次序取 11 个样本。两项候选哈希和
两项完整选取共四项；不从局部倍率推断转换器端到端加速。旧 #1029 的无前缀 XXH3
结果及 559 条人口替换不能沿用为本次数据。
