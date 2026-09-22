# #707 多 gap profile 夹具与覆盖验证

2026-09-21。Refs #707。独立研究夹具；不是优化收益或正式性能认证。

按用户补充要求，局部时窗实验不再仅使用单一 `urban-conservative-v1` 夹具。
已新增多 profile 配置、真实编译绑定、加载检查和动态查询审计。优化仍以性能
与合理交通结果验收，不要求复现旧版逐拍状态或跨 worker 摘要。

## 1. 夹具设计

三种布局保留相同城市拓扑、几何、路线、车辆物理参数、信号和外生出行请求，
改变 gap 参数的分布。它们是针对异质工作负载的合成夹具，不是现实城市统计标定。

| 布局 | 参数分布 | 主要用途 |
| --- | --- | --- |
| junction_balanced | 按 tile 和 slot 在路口间轮换短/中/长档 | 检查普遍存在异质时窗时的工作量 |
| junction_long_tail | 每类路口跨十 tile 周期按短 60%、中 30%、长 10% 分配 | 检查少数长时窗把全局时窗拉长的代价 |
| movement_mixed | 再按入口方向和转向分配，同一路口可有多个档位 | 避免只支持“一路口一个时窗”的简化 |

只给具有真实 yield targets 的流规则绑定 gap；三档均须有非空规则与目标引用。
分布跨路口模板轮换，避免把某个 profile 固定成某一种路口类型的代名词。

| profile | min lead | min lag | clearance | 100k 实际 lead | 100k 证明时窗 |
| --- | ---: | ---: | ---: | ---: | ---: |
| short | 1500ms | 500ms | 250ms | 1783ms | 1784ms |
| medium | 3000ms | 1000ms | 350ms | 3383ms | 3384ms |
| long | 5000ms | 2000ms | 500ms | 5533ms | 5534ms |

100k 步长 33ms；10k 步长 16ms，因此 10k 的 lead/证明时窗各比表中少 17ms。
这里的 lag 也存在差异，后续仅研究 lead 时窗计算组织时，应在**同一夹具、同一
组参数**下比较全局/局部实现；不能将换了 gap 参数造成的交通变化直接算作算法加速。

## 2. 实现与身份

- 独立 worktree：`E:/projects/laneflow/target/issue707-aggressive-source`，基于
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 加上一轮诊断补丁与本轮变更。
- 工具和三份配置：该源码的 `research/issue-707-multi-gap/`。
- 构建：`E:/projects/laneflow/target/issue707-aggressive-build`；Rust 1.98.0，
  offline/locked release，计数构建启用 `gap-audit,short-profile`。
- 完整结果：`E:/projects/laneflow/target/issue707-multi-gap-outcomes/`。
  原始首次生成及粗计数结果另存于 `target/issue707-multi-gap-results/`，未覆盖。
- `experiment.patch` 保存包含上一轮诊断在内的完整源码差异；`provenance.json`
  记录基线、补丁、源文件、工具链、输入、计划、二进制及输出摘要。修改后的源码
  不称为干净基线。原冻结测量 worktree、正式计划和 ETL 未修改。

完整补丁 SHA-256：`dfd4a95e37d571d6789b3fb5e731747fc61ec43501ee3add34997c4b5cba42f8`
（117829 字节，43 个源文件；包含继承的诊断文件）。反向应用检查通过。

生成器使用 RoadEditing → compiler → LFCA 正常链路构造多个 gap profile，
在实际有让行关系的流规则中绑定。Harness 不再硬编码“每个 profile 必须是同一
组数值”，而是核对哈希配置与编译后的 profile 名称、参数和安装后派生间隙。
原始单 profile 输入仍可加载，不靠关闭输入校验接受新夹具。

`gap-inventory` 按全部实际查询者收集每个目标 cell 的时窗需求，再取最大值。
局部时窗属于被查询目标的需求集合，**不是目标车辆自身的 gap profile**。
动态审计分 profile 记录 Accepted、Occupied、LagGap、LeadGap、Unprovable。
这是求值次数，不是不同车辆数、通行次数或最终裁决成功次数。

## 3. 代表性与结果边界

4 workers，各从初态运行 512 拍，三组 10k 加一组 100k 均完成。带结果分类的
覆盖运行发生于 17:24:42–17:26:28（+0900），各臂串行，运行期间未并发本任务
编译或 WPR。每档 profile 均出现实际 lead 拒绝与查询通过。

| 布局与规模 | short 查询次数 | medium 查询次数 | long 查询次数 | 更短局部时窗的目标 cell |
| --- | ---: | ---: | ---: | ---: |
| junction_balanced，10k | 6076 | 9380 | 8598 | 204 / 300 |
| junction_long_tail，10k | 13626 | 7701 | 2727 | 270 / 300 |
| movement_mixed，10k | 7652 | 6969 | 9433 | 196 / 300 |
| junction_long_tail，100k | 165453 | 84042 | 29219 | 2700 / 3000 |

100k 长尾场景的具体查询结果：

| profile | Accepted | Occupied | LagGap | LeadGap | Unprovable |
| --- | ---: | ---: | ---: | ---: | ---: |
| short | 40067 | 94568 | 5040 | 25778 | 0 |
| medium | 20588 | 47540 | 2700 | 13214 | 0 |
| long | 6983 | 15724 | 900 | 5612 | 0 |

三组 10k 前缀未触发 LagGap，100k 前缀已触发；所有场景均未触发 Unprovable。
Accepted 仅为这一次目标查询通过，不表示整个候选通过联合裁决。
100k 场景 Active 为 65577–75000，不是稳定 100k Active。

检查已通过：

- 不启用计数 feature 的 `cargo check`，以及启用计数与阶段诊断的 release 构建。
- 配置反例：重复 key、溢出、非递增时窗、profile 数量不足均拒绝。
- 三种布局的小夹具均经生成、编译、安装和 8 拍步进；配置字节被篡改后加载拒绝。
- 四个 512 拍覆盖记录完整，各 profile 有实际绑定、目标引用、LeadGap 和 Accepted；
  分类计数与总数闭合。
- 三组 10k 的 catalog 去除 network revision、plan 去除文件/manifest 身份后
  内容全等：固定的是外生输入，不是模拟输出。
- 分析器的四个反例：profile 未参与、分类总数错误、窗口缺拍、外生 seed 被改，
  均被拒绝。没有用旧版逐拍摘要作为通过条件。
- Rust 格式检查通过。未运行全 workspace 测试或数小时正式性能测量。

静态 100k 长尾布局有 3000 个被规则查询的目标 cell：

| 目标 cell 所需最大证明时窗 | 数量 |
| --- | ---: |
| 1784ms | 1800 |
| 3384ms | 900 |
| 5534ms | 300 |

其中 90% 的目标 cell 所需时窗短于全局最大值，说明夹具具备局部时窗实验所需
的异质性。这个比例不等于 frontier 工作量或时间能够降低 90%；需测实际访问、
ETA 求值、提前退出和局部表维护成本。

当前生成器是成对冲突区。审计的 `shared_multi_profile_targets` 为 0：同路口
有多 profile 不代表多个 profile 查询同一目标 cell。后续实现局部时窗时须补
一个多 reader 共用目标的定向场景，覆盖取最大需求及 profile/路线失效边界。
这不是阻止激进原型，而是避免用当前夹具证明它尚未覆盖的情况。

512 拍前缀仅覆盖 10k 的 8.192 秒、100k 的 16.896 秒仿真时间。它验证输入
构造和实际查询参与，不能代表完整信号周期、晚段生命周期或稳定 100k Active。
完整交通质量仍需通过吞吐、排队、停滞、重叠与纠正等指标评价。

计数 feature 使用跨 worker 原子计数，故本轮时间不用于任何净收益结论。
候选性能验证应关闭 `gap-audit` 和阶段插桩，在同一新夹具上进行平衡次序的
重复比较；继续记录外生请求、实际 Active 与完成量，不做旧输出等价验收。

## 4. 接续工作

以三种布局作为[激进优化计划](performance-first-next-plan.md)的输入集，推进
入口队列、局部许可与多频更新。局部时窗可以作为独立候选或原型内可关闭的研究
机制，用消融实验判断贡献；不要求它先通过旧版严格对拍。

相同输入下比较全局最大时窗、按目标需求的局部时窗及激进近似方案，报告净耗时
和交通结果。若缩短时窗或改精度，再单列参数变化，避免把工作组织收益与行为
变化混成一个无法解释的数字。单 profile 夹具保留作对照，不再作为唯一依据。
