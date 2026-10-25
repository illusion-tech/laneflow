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

## 冻结构建与两次独立进程

测量日期：2026-10-26（Asia/Tokyo）。使用
[`9a0bde5974bf4e6ac6cc6b298f145d62aab20455`](https://github.com/illusion-tech/laneflow/commit/9a0bde5974bf4e6ac6cc6b298f145d62aab20455)
的同一 release 测试二进制；后续提交只补充本文和证据数据。身份见
[`verification.json`](verification.json)，全部 88 个批次样本见 [`samples.csv`](samples.csv)。

- 人口实现 Git blob：`7eeb22c0b31a10c715dfe4e1d438461c00d18c90`。
- 迁移测试 Git blob：`f90dbc65d3ee1513e32902da33eaf6a8ac64c1c3`。
- Cargo.lock Git blob：`a35e035cb7bc0e2c2d9814c5ab615d82bab9fdfc`；依赖锁未因生产迁移改变。
- 测试二进制 SHA-256：`c5deac8d57c5863c9ba139ead9527082cfa0ec3647e5978259d37278f38098ee`。
- AMD Ryzen 9 9955HX，16 核 / 32 逻辑处理器；Windows NT 10.0.29683.0。
  Rust 1.99.1（`58cf8f9c35b355199727627d07ae34a4e2846d67`）、LLVM 23.1.1、
  `x86_64-pc-windows-msvc`；`twox-hash 2.1.3`、`sha2 0.11.1`。
- 两次 fresh process 均通过固定基线断言。使用 `fc.exe /b` 比较 11 份文件，全部
  逐字节相同；各自 build provenance 都指向上述提交和相同 lockfile。原始配置摘要
  因输出目录不同而不同，semantic provenance 保持相同。
- 测量期间本会话未并行启动本机构建或其他基准；未改变电源策略或 CPU 亲和性。
  主机并非独占环境，两轮结果不是统计显著性或跨平台性能证明。

下表为每进程 11 个样本的中位数，单位 **ms / 整次调用**：

| 项目                               |  第 1 轮 |  第 2 轮 |
| ---------------------------------- | -------: | -------: |
| 候选哈希：旧 SHA-256，10,592 个 ID | 0.452765 | 0.452089 |
| 候选哈希：v2 XXH3，含正式前缀拼接  | 0.106845 | 0.106428 |
| 完整选取及释放：旧 SHA 参考        |  63.8161 |  62.2589 |
| 完整选取及释放：v2 生产函数        |  62.1823 |  59.4771 |

最终前像下的候选哈希快 **4.24–4.25 倍**，每次构建节省 **0.3457–0.3459 ms**。
SHA 项校准为每批 128 次，XXH3 项为 256 次；后者先达到迭代上限，实际批次约
27 ms，未达到 40 ms。完整选取每批一次，测量区间仍有明显重叠。

两轮完整选取中位数分别下降 1.6338 ms 与 2.7818 ms，但该差值还包含新人口、
复制量、排序键宽和主机波动，不能全部归因于哈希。这里只确认持续发生的局部收益；
没有测量或宣称转换器端到端、Traffic Runtime 或交通质量提升。

本切片验证结果为：162 项 release 测试通过、9 项默认忽略；两个显式迁移进程各
1 项通过；cargo check、Clippy、rustfmt、Markdown 表格及根 cargo-deny 四项通过。
Dependabot security updates 为 enabled、未 paused，读取时开放告警为 0；根依赖
审计的既有重复版本警告仍保留。完整 v2 workload 认证与实际 DEMAND 位置仍属于
#254/#255 及既有路线/几何前置工作。
