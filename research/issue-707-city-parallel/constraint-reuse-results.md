# #707 C1：运动约束查询诊断与门控规则复用结果

2026-09-22。状态：**完成有限实验，淘汰本次静态门控规则表候选，保留 B1 + H1。**
两组 100k 顺序对照均没有稳定净收益；查询重复比例高不能代替端到端性能证据。
本轮不延伸到 H2 观测降频、红灯 ETA 模型修正或其他缓存实现。
设计与生命周期边界见 [C1 计划](constraint-reuse-plan.md)，上一保留基线见
[H1 完整观测去重](harness-dedup-results.md)。

## 1. 先计数，再选候选

独立 `query-work` 诊断构建，在 256 拍中的 65、81……241 拍采样，共 12 拍。
采样仅覆盖 Core step，进程内只运行一个世界；每拍清空键集合。全局互斥量和
HashSet 只用于计数，诊断耗时不用于性能比较。前车查询键包含 occupancy 视图、
完整车辆句柄、路线切片及出现位置、进度、长度表和查询窗口；门控键包含世界绑定、
已提交视图、gate、profile、调用方信号切片地址与长度。当前与下一信号视图区分。
计数不包含资源 owner 复用，也没有按 tick 缓存最终通行授权。

| 夹具 | 门控调用 / 唯一键 | 重复比例 | 前车调用 / 唯一键 | 重复比例 |
| --- | ---: | ---: | ---: | ---: |
| diagnostic-10k | 105,833 / 12,329 | 88.35% | 90,044 / 89,546 | 0.55% |
| diagnostic-100k | 1,101,752 / 127,379 | 88.44% | 887,131 / 879,324 | 0.88% |

100k 样本另记录 882,724 次完整运动计算、903,055 次停车绑定读取、579,627 次
受控门链访问、241,109 次前车后续路线出现项行走；169,822 次前车查询返回 None。
原有 MotionCache 已复用部分 P2/P5 预览，前车精确重复比例很低，因此不再加逐车缓存。

选定候选是在世界安装时派生稀疏 gate/class 规则表，同时保存 profile→class 与
gate→signal group。预先解释 None/Red/Yellow/Green 四种输入，每次查询仍读取
调用方信号切片。表返回 Candidate 或 Deny，不代表最终 grant。
门控调用数量没有减少；减少的是每次调用中重复的静态绑定读取和规则解释。

未知 gate/profile、缺失规则/灯态保持失败关闭。表不存车辆句柄、位置、owner、
grant 或 ETA，故不新增车辆代际失效逻辑。世界安装及修订迁移都经
WorldPolicyBinding::install 重建表；没有逐拍准备、更新或清理工作。
表仅在研究 feature `constraint-reuse` 下存在，由安装时的
`LF707_CONSTRAINT=direct/reuse/audit` 选择；默认 direct。

## 2. 性能：与已封存 H1 二进制直接对照

固定 `approx/both/combined/boundary=on/decision=every`、4 workers、
`LF707_OBSERVE=fused`，既有 `junction_long_tail-100k` 输入与 33ms 步长。
每臂 512 拍，统计 65–512 共 448 拍；Active 范围均为 65,577–74,392。
所有臂串行；构建在运行前完成，无 WPR、阶段 profile 或查询/观测计数插桩。
两个既有 Python 后台进程在每臂中的累计 CPU 增量均为 0；这不等于整个系统
绝对空闲，也没有实测动态频率，因此不把运行间波动归因于某个确定外因。

A 为 H1 原始 release 二进制，B 为本次 C1 reuse 二进制，避免用新增分支后的
direct 模式冒充 H1 成本。第一组 ABBA 出现 Core 与未修改观测阶段共同变慢，
因此补一组 BAAB；所有臂均保留，没有删掉较慢样本。
均值先逐臂求值再对相同模式取平均；p95/p99 为 `(n-1)*p` 线性插值。

| 对照 | H1 Core mean | C1 Core mean | C1 耗时增加 | H1 iteration mean | C1 iteration mean |
| --- | ---: | ---: | ---: | ---: | ---: |
| balanced-100k | 34.980ms | 40.730ms | 16.44% | 114.466ms | 130.530ms |
| reverse-100k | 34.927ms | 36.929ms | 5.73% | 114.908ms | 121.569ms |

| 顺序与臂 | Core mean | p95 | p99 | max | observation mean | iteration mean | 进程 wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ABBA 1-direct | 33.426 | 37.964 | 43.490 | 49.719 | 71.345 | 110.926 | 61.79s |
| ABBA 2-reuse | 40.428 | 45.153 | 47.872 | 52.806 | 80.980 | 129.802 | 69.95s |
| ABBA 3-reuse | 41.032 | 45.384 | 47.356 | 50.354 | 81.790 | 131.258 | 71.82s |
| ABBA 4-direct | 36.533 | 43.594 | 46.879 | 48.692 | 74.987 | 118.007 | 66.02s |
| BAAB 1-reuse | 39.845 | 45.698 | 48.022 | 53.121 | 82.134 | 129.942 | 72.23s |
| BAAB 2-direct | 35.968 | 41.851 | 44.294 | 51.023 | 74.335 | 116.994 | 64.34s |
| BAAB 3-direct | 33.887 | 39.447 | 42.393 | 46.910 | 72.551 | 112.822 | 62.18s |
| BAAB 4-reuse | 34.012 | 40.549 | 45.084 | 48.615 | 72.640 | 113.197 | 62.14s |

除进程 wall 外，上表单位为 ms。Core 为完整 public step，包括准备、维护、
清理和收尾；iteration 包括 advance 和缓冲语义输出，扩展质量采样、诊断 CSV、
最终 flush/checkpoint 不在 iteration 内，但在进程 wall 内。

BAAB 最后一对相邻 H1/C1 的 Core 均值为 33.887/34.012ms，接近但仍无净收益。
C1 自身从 39.845ms 降到 34.012ms，说明波动显著；本轮**不能声称门控原语本身
固定慢了 5.73% 或 16.44%**，也未隔离代码布局、缓存与系统状态的各自贡献。
能支持的决策是：这个完整候选没有展示稳定净收益，因此不纳入后续保留组合。
全部 C1 p95 仍高于 40ms，离 16ms 目标很远。

## 3. 成本和交通结果

100k 表包含 11,000 个 gate、11,000 个实际 class 单元、3 个 profile，额外
堆容量逻辑字节为 **352,024 B（约 343.77 KiB）**，不含 Vec/Option 内联头部。
四个性能候选臂的安装派生耗时为 0.354–0.511ms，已纳入各自进程 wall；不是
每拍成本。10k 对应 35,224 B、1,100 gate/class 单元和 3 个 profile。
没有分配 gate×profile 的车辆规模矩阵，释放随世界退出纳入进程 wall。

性能臂工作集峰值观测如下；每 250ms 轮询进程高水位，可能遗漏最终尾部，
不是精确全生命周期峰值，不能据此反推表没有内存成本。

| 顺序与臂 | 工作集峰值观测 MiB |
| --- | ---: |
| ABBA 1-direct | 451.348 |
| ABBA 2-reuse | 447.852 |
| ABBA 3-reuse | 448.633 |
| ABBA 4-direct | 447.836 |
| BAAB 1-reuse | 448.633 |
| BAAB 2-direct | 448.145 |
| BAAB 3-direct | 448.078 |
| BAAB 4-reuse | 448.629 |

四组有限审计：balanced 10k×512、long-tail 10k×512、mixed 10k×768、
long-tail 100k×256，每组 direct/reuse/audit 三臂，共 12 臂、6,144 拍。
audit 每次门控查询都与保留的直接实现断言相同；八类语义文件、扩展质量
（排除其耗时字段）及初末 checkpoint 在每组内一致。
八个性能臂另有 4,096 拍，跨 ABBA/BAAB 的语义结果也一致。
加两次诊断 512 拍，总计 **22 臂、10,752 拍**。

100k 512 拍结果均为 completed 10,269、released reservations 1,153、
open reservations 1,042、open passages 4,120；multi-owner 暴露、无匹配 clear、
Completed 仍持资源均为 0。最长连续停车 15,345ms，最长未释放预留 12,144ms。
这些是相同规则复用的有限检查，不恢复旧版轨迹作为新模型验收门槛，也不构成
连续几何无碰撞认证。原有红灯 ETA 与长等待问题没有修复。

## 4. 验证与证据身份

- 新规则对照测试：所有夹具 gate/profile（包含越界身份）、空信号与红黄绿视图、
  未绑定 policy 的失败关闭，1 passed。
- signal-boundary 失败发布/重试：1 passed；cutover 筛选：108 passed、1 既有 ignored；
  policy 筛选：4 passed（与 cutover 有重叠，不累加为互异测试数）。均启用 C1 audit。
- Harness lib：23 passed、3 既有 ignored；multi_gap integration：1 passed，启用 C1 audit。
- 默认 feature `cargo check --locked --offline`、`cargo fmt --all -- --check`、
  `git diff --check` 通过。默认检查有继承的 `old_slot` unused warning，未为消除
  无关警告改写既有研究源码。

独立源码：`target/issue707-c1-source`，基线提交
`4de40e045398e4b010b2aa36522afc02a4094c4d`，继承封存 H1 原型；
构建：`target/issue707-c1-build`；证据：`target/issue707-c1-results`。
Rust 1.98.0、release、locked/offline、CARGO_INCREMENTAL=0。
性能 feature 为 `entry-frontier,constraint-reuse`；诊断先于候选，feature 为
`entry-frontier,query-work`。诊断源码的构造链为封存 H1 源码 + `instrument.py`
再 cargo fmt；它不是最终候选源码。`implement.py` 与新增 gate_rules.rs 属于其后阶段。

二进制 SHA-256：

```text
H1 parent: 5525f9aea127793c69ab08d4891fd05e3049dd79585199e00272b5d050391080
C1 release: d9e5e1d85671e900d99ea9dab875b36cff23c7ab2d23bf63e363d5fb6a685265
query-work: a538dfd24cb0f631ca4f0da360c3ad84937fcbc4f3b66bdb35c5abb6e4e47651
```

`summary.json`、`analyze.py` 保存指标及失败关闭检查；每组 environment/process
记录保存输入和二进制身份、模式、WPR 状态及进程边界。`experiment.patch` 包含
基线之上的完整研究差异（含未跟踪源码），`provenance.json` 与 `verification.json`
用于核对父源码、输入、结果、报告和脚本。父 H1 源码与既有封存报告保持不变。

复测需使用新 Group，脚本拒绝覆盖已有结果；例如：

```powershell
./target/issue707-c1-results/run.ps1 -Group new-abba -Fixture junction_long_tail-100k -Modes 'direct,reuse,reuse,direct' -Ticks 512 -ParentBaseline
```

## 5. 收口

研究源码和失败结果保留为证据，后续选用组合继续是 B1 + H1，C1 不进入默认路径。
下一次 Core 工作应先分开量出运动计算的输入读取、跟车求解与下一状态构造成本，
不要继续仅凭调用次数扩展缓存。H2 降频和 ETA 质量修正保持各自独立范围。
本轮没有创建 PR、提交或推送；16ms p95 目标尚未达成。
