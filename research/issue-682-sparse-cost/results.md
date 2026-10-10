# #682 低活动率成本测量结论

> **证据归档**：本目录的历史诊断 JSON 已迁入[冻结归档](../archives/2026-10-01-json-migration.md)。
> 本文的 JSON 链接指向原提交；文中相对 JSON 路径及依赖它们的历史命令按归档内 `source/` 目录解释。

**优先验证 Conflict 资格表的容量维护路径。** 在 Active、live 和历史槽位长度均为
1000 时，仅把配置容量从 10000 提高到 100000，三轮正常库整拍 mean 从约
0.18 ms 升至 0.50 ms，world-owned retained memory 从 10.82 MiB 增至
105.92 MiB。高水位单独增加的差异没有与本批波动分离。当前证据不支持优先重做
热冷布局或冲突排序。本项没有实现优化候选，也没有证明可兑现的节省比例。

## 冻结身份与证据

- Runtime 基线：`03da6f57d40dceef59c86e86d3532a28751e5b15`，包含 #713。
- 采集源码：`2f5c51a8c068bda10462daf650f12470f55b46bc`；tree：
  `b922d2af85c7647a74cd9cfcbb86a0ae768bdf89`。采集前已推送，全部进程前后 HEAD
  相同且 Git 干净。报告提交只补核验、文档与证据，测量代码不变。
- 正常库二进制 SHA-256：
  `7999554a2c6518db573afbfc8ef6587a44c5b0884cb0ff73dbcdc92cf66bf8ad`；
  测试诊断二进制 SHA-256：
  `643aca50193aa3503142d2479a052078bc2ac8bc755ecb2a44bad7f3fc45cd2b`。
- Rust 1.98.1 / LLVM 22.1.8，Windows x86_64，Ryzen 9 9955HX（16 核/32 逻辑
  处理器）、约 61.68 GiB RAM、Windows 11 Insider 29671、平衡电源方案。
  前后环境记录均为接通电源且电量 100%；没有连续温度、频率或厂商模式遥测。
- 每组 workers=1，暖机 40 拍、观察 128 拍，每拍 100 ms；三轮独立进程，第二轮
  反向组序。27 次正常库墙钟、27 次独立测试库诊断，以及一个包含六个资源窗口的
  诊断进程，共 55 个进程记录。安装、编译、暖机和输出不计入 step 墙钟。

[机器核验结果](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-682-sparse-cost/evidence/results.json)、[完整数表](measurements.md)、
[schema 2 原始日志与元数据](evidence/v2)、[原 schema 1 记录](evidence/v1) 均随 Git
保存。每个进程记录 UUID、输入摘要、源码/tree、manifest/lock、rustc、二进制摘要、
日志摘要和前后负载。环境记录见 [采集前](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-682-sparse-cost/evidence/environment-before.json) 与
[采集后](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-682-sparse-cost/evidence/environment-after.json)。后续核验在采集源码的后代提交上运行。

从仓库根目录复核封存记录：

`cargo run --locked -p laneflow-sparse-cost-research -- verify research/issue-682-sparse-cost/evidence/v2 target/682-reverified.json`

## CPU 门禁修正与波动

原固定 20% CPU 硬门禁没有完成机器校准，容易把正常背景波动当作不合格。
schema 1 留下五条记录：两条墙钟在结束采样超过 20% 后拒绝、两条已接受记录，以及
一条启动前拒绝。它们保持原样，不混入本批结果，也不在事后改成接受。

schema 2 先取 30 个每秒样本，空闲基线为 4.25%–13.14%，p95 为 12.590467%。
本批告警线为 p95 加 10 个百分点，即 22.590467%；这是透明的经验告警尺度，
不是性能合格线。其他 Cargo/rustc/link/cl 或已知 Runtime/harness 进程仍是硬阻断；
每秒检查一次，不能排除更短的竞争或普通应用干扰。

55 个进程没有观察到竞争进程，只有 vacant 第二轮启动前出现一次 28.087078%
CPU 告警；该轮完整保留，mean 为 0.190131 ms，其余两轮为 0.185710/0.187632 ms。
不能凭这一点建立 CPU 占用与时延的因果关系。大多数组 mean 极差/中位数为
2.26%–6.03%，ring-10m 达 13.04%；没有挑最低值、删除异常轮次或把单次 p95
池化成一个“更稳”的分位数。接受记录只表示结构与可观察环境通过筛查。

## 正常库整拍

mean 是三轮等长窗口均值的等权平均；p95 分别按各轮 128 拍 nearest rank 计算。

| 组          | 三轮 mean 平均 ms | 三轮 mean 范围 ms |  三轮 p95 范围 ms |
| ----------- | ----------------: | ----------------: | ----------------: |
| compact     |          0.181920 | 0.178110–0.188891 | 0.186000–0.232700 |
| vacant      |          0.187824 | 0.185710–0.190131 | 0.192000–0.222800 |
| parked      |          0.265847 | 0.262827–0.268842 | 0.275100–0.311300 |
| capacity    |          0.499871 | 0.488796–0.514630 | 0.544200–0.630400 |
| active      |          1.746344 | 1.711748–1.779188 | 1.856500–2.173900 |
| edges-small |          0.181501 | 0.179923–0.184464 | 0.206900–0.219900 |
| edges-large |          0.196662 | 0.192203–0.199994 | 0.198700–0.268300 |
| ring-10m    |          0.050027 | 0.047624–0.053952 | 0.056200–0.068000 |
| ring-1m     |          0.063028 | 0.061890–0.065123 | 0.073300–0.080500 |

固定配置容量为 10000、Active/live 为 1000，将历史槽位长度从 1000 增至 10000
（compact→vacant），逐轮差异为 +3.89%、+6.75%、−0.67%，不能判为稳定收益机会。
在相同高水位上把空槽换成 parked（vacant→parked），逐轮增加 38.23%–44.76%；
不能据此删除对非 Active 车辆的正式一致性检查。

## 容量、占用索引与布局判断

配置容量扩大十倍，逐轮正常库 mean 增加 158.77%–187.89%。独立诊断三轮平均中，
Commit 从 0.021262 ms 增至 0.295843 ms，ConflictPrepare 从 0.009785 ms 增至
0.066754 ms。其中单独计时的两张 Conflict 数组清空从 0.005988 ms 增至
0.062653 ms；Waiting 计划索引清空从 0.000308 ms 增至 0.002914 ms。
后三者分别嵌套于父阶段，不相加制造整拍节省。还存在未单独计时的清空路径。

源码 `commit_conflict_step` 每拍先把整个 `conflict_next_eligibility` 复制到
committed，再由 `normalize_conflict_eligibility` 扫描全空并清空逻辑长度。
**阶段证据与这条容量线性路径相符，但本次没有把 Commit 内部逐项计时，因此不能
把全部 Commit 差额精确归给该函数。** 后续应先缩小这一提交路径的归因和候选范围，
而非只优化准备阶段的 `fill(None)`。

三个已计时清空数组在 compact/vacant/parked 中每拍分别访问 10000 和 20000
个元素；capacity 为 100000 和 200000。说明它们跟随配置容量，而非当前车辆槽位
长度。只增容量时，world-owned retained bytes 增加 99720000（95.10 MiB）。
该账本按 heap capacity 和唯一 owner 记账，独立 shared network 不计入 world-owned；
它不等于 RSS、峰值或每拍实际写入的总线字节数。

`VehicleSlot` 为 96 B、`VehicleState` 为 88 B，10 万容量下车辆槽位预留约
9.16 MiB，仅是世界内存的一部分。没有 PMU cache-miss、逐字段访问或候选布局实验；
不能从类型大小或 parked 组的阶段差异推出 AoS cache miss，更不能承诺热冷拆分收益。

固定 1000 Active、仅增未使用静态边（16→4096），逐轮整拍增加 4.20%–11.16%。
诊断 Occupancy 从 0.011494 ms 增至 0.023942 ms，主要增长落在 layout 与
sort/suffix 段；count/fill 的记录数仍相同。可保留 touched-bucket 作为次级研究方向，
本批绝对成本小于高容量资格表路径，不交付重建方案。

短边组的 128 拍记录数为 11968→45440，出现项遍历为 121280→1080896，
动态分配次数为 3072→8640，累计分配字节为 131072→368640；所有组 reallocations
均为零。短边同时改变环长、占用跨边和前视访问，不能把 14.71%–36.74% 的整拍增加
只归给重建或排序。其分配来源也未单独定位。长边七组观察窗内 allocation/reallocation
均为零，但不代表安装、暖机或其他交通输入不分配。

## 排序与后续候选

资源对照为 Waiting 单车、Conflict 双车，每轮 16 拍。Conflict 的候选排序、cell
排序去重、公开决定排序均为每轮 16 次、累计 16 个条目；各段累计计时仅
300–1400 ns，接近计时粒度与插桩开销。Waiting 输入没有 Conflict 候选，部分记录
只是空排序调用。这里没有能支持“重复排序是主瓶颈”的非平凡规模证据，不能外推到
密集资源争用。完整 calls/items/ns 和状态摘要保留。

下一项建议限定为 **资格表提交和暂存清理的精确串行候选**：先细分 P7 复制/全空
规范化，再以完整句柄代次记录实际触及项，覆盖上一尝试/上一拍的旧资格撤销；重试
代次不能直接使用 committed tick，失败后必须重新失效。保留
`conflict_state_valid` 的正式等价保障、首错、失败原子性、资源释放和迁移/快照语义，
不得删除检查或改成 `debug_assert`。是否采用 touched-slot 或独立 attempt epoch
应在后续 G1/实现任务里决定，不在本报告中预先批准。

后续候选需覆盖稀疏/满载两端、小/大容量、真实 Waiting/Conflict 持有与撤销、
同槽新代次和同拍失败重试，并至少三组平衡正常库 A/B 比较整拍与保留内存。
没有净收益、正常规模回退或正确性不闭合时放弃。该建议不代替 #707/#537/#539
产品认证，也没有开始实施或自动建立第二项优化。

## 验证

- 55 个进程全部完成；输入、维度、最终摘要在同组各轮及两种构建之间一致。
  各组工作量和内存账本跨轮一致，全部 owner 加总通过；原始日志摘要核验通过。
- 稀疏夹具轴验证；高水位槽位重用代次及两个失败点同拍重试；4 项 Gate scope、
  2 项 Conflict retry、6 项 eligibility（含撤销）、完整 retained-memory smoke 通过。
- 工具 3 项单元测试、Runtime/工具 all-targets Clippy `-D warnings`、fmt 通过。
  [实际封存证据负例](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-682-sparse-cost/evidence/negative-verification.json) 的日志改动、缺拍、重复轮次、
  隐藏 CPU 告警、混合资源源码、账本不平和资源状态变化七种情况均被拒绝。
- 没有改变正式 Runtime API、交通行为、数据格式、Adapter 或资源权威。
  无插桩墙钟不与诊断二进制的绝对时间相减，不把这些小型串行场景当作城市认证。
