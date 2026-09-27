# #707 WP C.1 输入差异与冻结计划重放

> 结论：10k/100k 重建制品的七个内容文件与版本化冻结 manifest 逐字节一致；
> 重建 manifest 只改变两个共享构建保留内存统计字段。使用 `4de40e04` 的
> 导出源码、版本化冻结 manifest 和这些内容文件重新生成 correctness 计划后，
> 两档计划的字节数与 SHA-256 均精确匹配 `fixtures/v3/plans.json`。再将重放
> 计划与重建制品计划逐行比较，两档都只有 `manifest_digest` 一行不同。
> 因此计划差异已经由实际重放闭合，不再依赖“旧计划本体不可得”时的推断。

完整的小型机器可读记录见
[`evidence/frozen-plan-replay.json`](evidence/frozen-plan-replay.json)。文件身份使用
SHA-256；Git 对象身份使用 `git rev-parse <commit>:<path>`，两种口径不混写。

## 1. 冻结 manifest 身份

冻结 manifest 位于
`tools/laneflow-urban-generator/fixtures/v1/{10k,100k}/manifest.toml`。
两档在 `40127704` 与 `4de40e04` 之间各自保持同一 blob；10k 与 100k 是两个
不同对象：

| 规模 | 字节数 | SHA-256 | Git blob（两提交相同） |
| ---- | -----: | ------ | ---------------------- |
| 10k | 12,563 | `e79e75a58c48da85fbe8a5dc2ddb6dba3af82129ca095d7a6b7cf60270ead43d` | `591db2887d8e6ff82f832068b27606884b665bb0` |
| 100k | 13,832 | `694808d79940d5e9457c19280c56e3ad30cc1f609e863f3f761dfefe911f79f7` | `7bfb5755356120407f2057079c9cc107809f85d6` |

此前把 `591db288…` 同时写给两档是错误记录；本表和机器记录已分档修正。

## 2. 内容文件核对

证据根 `issue-707/4de40e04/inputs/urban-{10k,100k}` 中的七个内容文件逐个按
字节数与 SHA-256 对版本化冻结 manifest 核验，14/14 匹配：

| 文件 | 10k 字节数 | 100k 字节数 |
| ---- | ---------: | ----------: |
| `common.lfre` | 1,024 | 1,024 |
| `config.toml` | 1,301 | 1,301 |
| `genesis.lfsd` | 17,123,970 | 171,296,210 |
| `network.lfca` | 16,240,189 | 162,397,835 |
| `routes.toml` | 932,336 | 9,390,023 |
| `source-map.lfsm` | 21,305,129 | 213,077,371 |
| `topology.lfre` | 2,487,784 | 24,870,952 |

`network_revision` 也分别保持
`bef82350bc89578b0565f56a05d0e5c8afe82ff066464aca855a0148b24c336a`
与 `cd9cbe68dd9686216a9303ab95aef5ad26b5a819158fa507f8c46d0f238e58ee`。

## 3. 重建 manifest 的实际差异

版本化冻结 manifest 与证据根重建 manifest 逐行比较，每档都只有以下两个字段
不同，其余行相同：

| 规模 | `shared_headless_retained_bytes` | `shared_spatial_retained_bytes` | 重建 manifest SHA-256 |
| ---- | --------------------------------: | -------------------------------: | ---------------------- |
| 10k | 1,351,804 → 1,334,868 | 3,591,980 → 3,575,044 | `d9f08f511b50f19eab18a5a87d6f9db5a3836b34f8d82054ab3323ac1583d9dd` |
| 100k | 13,488,124 → 13,318,548 | 35,900,620 → 35,731,044 | `5e2a676fbb3da4d6b66a3966b66a21bb2abedad2c61634a1446a0f53c2812140` |

它们是共享构建的保留内存统计，不被 `TrafficWorld` 当作交通输入。冻结 manifest
及 `plans.json` 均未改写。

## 4. 冻结计划重放

重放使用以下闭合步骤：

1. 从 `4de40e045398e4b010b2aa36522afc02a4094c4d` 执行 `git archive`，不叠加
   工作树文件；使用 Rust/Cargo 1.98.0、release、locked、offline、关闭增量构建
   `laneflow-urban-harness`。二进制 SHA-256 为
   `4d0a060e43c396148c9a347f327cc04fb09bd2c3cc740a472a5f19437360171e`。
2. 对证据根七个内容文件逐个通过冻结 manifest 校验，再把版本化冻结 manifest
   与这些内容文件组合为只读重放输入。
3. 用归档 Harness 分别生成 10k/100k `MIXED-PEAK` correctness 计划，并与
   `tools/laneflow-urban-harness/fixtures/v3/plans.json` 比较。

| 规模 | 重放计划字节数 | 重放 SHA-256 | `plans.json` |
| ---- | -------------: | ------------ | ------------ |
| 10k | 4,057,834 | `d0e58a7c9515ca22bfbee4352e9685279709d43b488e7b34dc0c6b7b389998c9` | 精确匹配 |
| 100k | 40,845,093 | `4f2dddc266b9f1a0dfd0c60c707ad0de56d1112b489df687eba22a3904e8fd49` | 精确匹配 |

重放计划再与证据根的重建 correctness 计划逐行流式比较：

| 规模 | 重建计划 SHA-256 | 差异行 |
| ---- | ----------------- | ------ |
| 10k | `1fe166ef8892e56fbe5316de83edfe8ed8ea74087cf0f20ac24b11ae92448880` | 仅 `manifest_digest` |
| 100k | `71cfb0029a5143c5a9cf04d60861cec6514822c4e2afe5add2da3b70c4a83fb2` | 仅 `manifest_digest` |

两档的计划字节数不变；除该定长摘要行外，全部计划行一致。这里直接比较了实际
重放结果，不再用相同字节数或未变化的生成器间接排除其他等长差异。

## 5. 接受边界

- 冻结计划身份、内容文件身份及重建计划的唯一差异均已复核；本项没有未解释的
  输入或计划差异。
- 重建制品可用于本 PR 已声明的 pilot/阶段研究范围；这项结论不替代 #707 的
  worker 矩阵、稳定 Active、执行归因或最终性能认证。
- 版本化 manifest、`fixtures/v3/plans.json` 与既有原始证据均保持原样；提交的
  JSON 只保存复算身份、逐文件核对和差异字段，不把大型重放计划加入 Git。
