# #707 WP C.1 输入差异核查（input-diff）

> 结论先行：10k/100k 两档的冻结制品与本次重建制品**仅两个构建内存统计
> 字段不同**（`shared_headless_retained_bytes`、`shared_spatial_retained_bytes`），
> 全部交通消费字段（network_revision、全部内容文件摘要、config、routes、
> LFCA/LFSD/LFSM/LFRE 源）逐字段/逐字节一致。差异是**非交通元数据**，
> Runtime 消费字段等价——按审阅接受标准**接受重建制品**。plans.json 计划
> 摘要不匹配已由本文件完整解释（唯一计划差异是内嵌的 `manifest_digest`，
> 其输入即整张 manifest 的 SHA-256）。
> 方法口径：文件身份用 SHA-256；Git 对象身份用 `git rev-parse <commit>:<path>`
> 与 `git hash-object --no-filters`，两口径不混写。

## 1. manifest 逐字段比较

冻结 manifest：`tools/laneflow-urban-generator/fixtures/v1/{10k,100k}/manifest.toml`
（blob 身份经审阅核验：`40127704` 与 `4de40e04` 两提交均为
`591db2887d8e6ff82f832068b27606884b665bb0`，即冻结值未漂移）。
本地 manifest：`E:/projects/laneflow-evidence/issue-707/4de40e04/inputs/urban-{10k,100k}/manifest.toml`。

| 规模 | 比较结果                                                                                                                                                                                                                                                         |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 10k  | 全部字段一致，**仅 2 个字段不同**：`shared_headless_retained_bytes` 1351804 → 1334868（−16936）；`shared_spatial_retained_bytes` 3591980 → 3575044（−16936）。`network_revision = bef82350bc89578b0565f56a05d0e5c8afe82ff066464aca855a0148b24c336a` 两边完全相同 |
| 100k | 同样仅 2 个字段不同：`shared_headless_retained_bytes` 13488124 → 13318548；`shared_spatial_retained_bytes` 35900620 → 35731044。network_revision 一致                                                                                                            |

- 冻结 manifest 自身 SHA-256：10k `e79e75a5…`、100k `694808d7…`；本地：
  10k `d9f08f51…`、100k（见 evidence/ 内副本复测）。**首个差异字段**：
  `shared_headless_retained_bytes`。
- 两档漂移量同为 −16936 字节，指向共享构建实现的构建统计口径演进
  （制品冻结于 `026e09ea`「deliver connected urban topology artifacts」
  时代；此后共享构建工作区统计随实现演进，#706 期间 workspace 字段
  增删亦属同类）。该字段是**构建过程的内存统计记账**，描述共享构建
  暂存规模，不被 TrafficWorld 消费。

## 2. 消费文件逐文件摘要（本地实测复核 vs 冻结 manifest 记录）

冻结与本地 manifest 的 `files` 表**逐条目完全一致**（上节逐字段比较
已覆盖 `files` 键）；本地文件实测 SHA-256 与字节数对 manifest 记录
**全部匹配**：

| 文件            | 10k 字节数 | 100k 字节数 | 摘要匹配 |
| --------------- | ---------- | ----------- | -------- |
| common.lfre     | 1 024      | 1 024       | ✓        |
| config.toml     | 1 301      | 1 301       | ✓        |
| topology.lfre   | 2 487 784  | 24 870 952  | ✓        |
| routes.toml     | 932 336    | 9 390 023   | ✓        |
| network.lfca    | 16 240 189 | 162 397 835 | ✓        |
| genesis.lfsd    | 17 123 970 | 171 296 210 | ✓        |
| source-map.lfsm | 21 305 129 | 213 077 371 | ✓        |

（表内 sha256 值以证据根 inputs 目录内 manifest.toml 为准，此处不重复
展开；逐文件值已由脚本复核 match。）

## 3. 计划来源字段

`ResolvedPlan` 内嵌的 `files` 表（plan 内）= 上列 5 个来源文件
（common.lfre、config.toml、network.lfca、routes.toml、topology.lfre）
的摘要——这些键的值与冻结一致（来源文件未变）。**唯一起变化的计划
字段是 `manifest_digest`**：

- 本地 10k smoke 计划内嵌 `manifest_digest = d9f08f51…` == 本地
  manifest.toml 的 SHA-256（实测 `sha256sum` 一致）。两份 correctness
  参考计划本体已钉身份（evidence-index.toml）：10k
  `1fe166ef…`（4 057 834 字节）、100k `71cfb002…`（40 845 093 字节），
  与本文所引字节数一致。
- 冻结 plans.json 的 `10k-mixed-peak` 计划字节数与本地生成**完全相同**
  （4 057 834）而 SHA-256 不同：两张 manifest 的差异数字同为 7 位十进制
  （1351804/1334868），经 `manifest_digest`（64 位十六进制、定长）进入
  计划后**字节数不变、内容哈希变**——与观察完全吻合。
- 计划展开逻辑 `tools/laneflow-urban-harness/src/plan.rs` 的 blob 在
  `40127704` 与 `4de40e04` 相同（`5a00fa1d…`，审阅已核验），同一制品
  重复生成计划逐字节一致（本切片已验证）→ 计划本体除 `manifest_digest`
  外无其他差异来源。

## 4. 交通需求字段

route_edges（来源 routes/topology.lfre + plan.rs 展开规则）、初态、
profile、departures、leaves、arrivals、角色请求、重试规则、窗口、验收
下限：全部由上述内容文件（逐字节一致）与未变化的 plan.rs 决定 →
**无变化**。旧计划本体不可得，但差异已定位到唯一计划字段
（`manifest_digest`，非交通字段），且其输入差异（两个构建统计字段）
非交通消费字段——不存在无法推断的交通字段差异。

## 5. 差异原因与接受结论

| 差异字段                         | 性质                           | 原因                                         | 是否交通字段         |
| -------------------------------- | ------------------------------ | -------------------------------------------- | -------------------- |
| `shared_headless_retained_bytes` | 构建内存统计记账               | 共享构建实现演进（制品冻结于 026e09ea 时代） | 否                   |
| `shared_spatial_retained_bytes`  | 构建内存统计记账               | 同上                                         | 否                   |
| 计划 `manifest_digest`           | 计划对来源 manifest 的绑定摘要 | 上述两字段漂移的传导                         | 否（计划绑定元数据） |

- **未解释差异：无。**
- 按审阅接受标准：仅来源/非交通元数据变化且 Runtime 消费字段等价 →
  **接受重建制品**；完整新摘要与差异说明已存档（本文件 + evidence/
  内 manifest 副本），**未覆盖旧 golden**（fixtures 与 plans.json 原样）。
- WP D 放行建议：**放行**（见提交说明与 README 更新）。
