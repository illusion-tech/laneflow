# 统一五轴汇总 Gate 执行器

汇总器读取 `LF-SYNTH-v2` 与中国特色城市负载的五轴记录（behavior / determinism /
performance / snapshot / routing）。回执逐格携带来源提交、被测二进制、输入身份、
覆盖边界与例外状态。当前清单用动态换道步进记录核对 L1 的实际执行。

边界：不运行正式汇总、不替代各轴分立 harness、不重跑正式测量、不批准例外；
全部来源身份从既有记录**原样携带**，本工具不重算摘要（ADR 0033 §3）、不联网
核验、不执行测量；离线运行。manifest 引用的仓库文件只在运行时读取，不编译期嵌入。

## 命令

```powershell
cargo build --release --locked -p laneflow-gate-summary

# 汇总（自包含示例，试跑模式允许未完成项）
target/release/laneflow-gate-summary summarize `
  --manifest tools/laneflow-gate-summary/fixtures/example/unified-gate.toml `
  --repo-root tools/laneflow-gate-summary/tests/fixtures/repo `
  --evidence-root tools/laneflow-gate-summary/tests/fixtures/evidence-root `
  --output <尚不存在的输出目录>

```

- `--repo-root`：manifest 内仓库相对路径的根（默认当前目录）。
- `--evidence-root`：manifest 内 `<evidence-root>/` 前缀占位路径的根。仓库外证据
  目录一律用该占位书写，不写采集机本机布局（`documentation-policy` §3）。
- `--mode formal`：正式模式，拒绝 `pending` 状态格；`--require-complete`：拒绝
  任何带未完成项的格。#305 正式汇总两者都开。
- 输出目录/文件必须不存在（沿 harness 家族「不覆盖旧交付」惯例）。

产物：`receipt.md`（人读）+ `receipt.json`（机器可读，`lf-unified-gate-receipt-2`）。
回执是确定性的——同一 manifest 与同一批记录产生逐字节相同的回执，便于试跑与
正式汇总对拍。

## 声明式输入（`lf-unified-gate-manifest-2`）

自包含示例在 `fixtures/example/unified-gate.toml`，证据取自本 crate 的
`tests/fixtures/`，数值与提交号均为测试用值；`tests/summary.rs` 以它为基础
manifest，所以默认命令随测试持续可执行。v0.10 两负载的历史试跑清单不在当前树，
见固定提交中的[试跑副本][v0-10-trial]；它引用的仓库文档只存在于该提交，重放时
`--repo-root` 须指向该提交的 checkout。结构说明：

- `required_loads`：本轮汇总必须覆盖的负载（含延后负载）；`loads` 须与该集合
  **恰好一一对应**（缺失、重复或多挂都失败），防止整个负载被误删后静默通过。
- `[[loads]]`（`kind = "axes"`）：一套负载 × 五轴逐格——`status`（`semantic-pass`
  / `witness-pass` / `uncertified-performance` / `authorized-exemption` /
  `pending`）、结论、来源提交、被测二进制、输入身份、覆盖边界、例外声明与
  证据锚点；`kind = "deferred"` 为整负载延后（LuST）。
- `[global.performance]`：性能口径（#1266 回执）。判定词被机械校验：`verdict`
  必须精确为「未认证」、`product_status` 必须为 `Product TBD / Uncertified`；
  performance 轴的格状态必须为 `uncertified-performance`——写成通过即失败。
- `[[scope_adjudication]]`：本轮范围裁定与未覆盖项，豁免须带授权。
- `[[capability_limits]]`：登记本轮适用的能力限制和证据来源。
- `required_l1_identities` 与 `[[l1_checks]]`：每份声明的动态换道输入须挂接一份完成的
  `lf-synth-semantic-run-1` 回执。身份写作 `<scale>-L1-<dt>-clusters-<簇数>`，
  缺失、重复或额外身份均拒绝。

证据锚点种类：`repo-doc`（存在 + 锚点子串）、`github`（URL 形态）、
`synth-summary-json`（F 矩阵 `summary.json` 身份对账；二进制摘要同时消费语义
矩阵的顶层 `binary_sha256` 与 F1/F2/F3 的嵌套 `binary.sha256`，两者同时存在即
失败）、`gate-json`（城市 `gate.json` 身份对账）、`routing-kv`（墙钟记录头行：
识别 §7.1 的 `routing-g2-wall-clock` 与 #1279 运行的 `routing-lfsynth-wall-clock`；
标记前后落在词边界即可定位，容忍 `--nocapture` 捕获的 libtest 前缀，带
`operation=` 的统计行不算头行；同一文件不得混用两种标记，多轮拼接时每个头行
都须命中；`expect` 须含 `workload_id`，LF-SYNTH 记录还须含 `identity`；期望键
缺失不容忍、未知键前向容忍）。动态换道证据使用 `handoff-run`，详见下文。

## #305 正式汇总执行记录（已完成）

v0.10 五轴正式汇总绑定验收提交 `c0045f8f`（#1282），#1279 计时也只适用于其被测实现及输入。
两份旧 L1 输入包完成当时的拓扑核查，#305 于 2026-11-09 关闭。
原始的正式结论、产品性能“未认证”及 LuST 范围以[验收记录][v0-10-record]为准。
历史试跑另见[当时的清单][v0-10-trial]。
本节记载的历史证据只适用于当时生成的输入。

## 动态换道运行回执

每个 `handoff-run` 声明运行回执 `path`、被测输入包的 `input_manifest` 路径和
`implementation_commit`。汇总器先逐字段核对内嵌与外部输入清单，包括规模、步长、
簇及车辆数、种子、来源与生成器提交、配置摘要、计数和各文件记录。随后核实执行提交、
语义合同、完整运行窗口，以及 0、预热末拍、两倍预热拍和窗口末拍的历史前缀。
每簇两辆车须依次进入、交接前杠并清空车尾。逐簇核对槽位与路线身份，
不同簇可以在同一拍推进。所验回执只覆盖该输入的动态换道竞争。

## 测试

```powershell
cargo test --locked -p laneflow-gate-summary
cargo clippy --locked -p laneflow-gate-summary --all-targets -- -D warnings
```

`tests/summary.rs` 以示例 manifest 覆盖汇总与全部失败关闭路径（性能判定词、锚点未命中、
身份不符、未完成项拒绝、占位路径缺根、动态交接历史不完整与输出目录已存在）。

## 关联

[#1268][issue-1268]、
[#539][issue-539]、
[#305][issue-305]、
[#1279][issue-1279]、
[#1271][issue-1271]、
[#1266 回执][issue-1266-receipt]、
ADR 0033（摘要角色；本工具不新增摘要）、
`docs/governance/documentation-policy.md` §3（研究结论与过程数据边界）。

[issue-1268]: https://github.com/illusion-tech/laneflow/issues/1268
[issue-305]: https://github.com/illusion-tech/laneflow/issues/305
[issue-539]: https://github.com/illusion-tech/laneflow/issues/539
[issue-1279]: https://github.com/illusion-tech/laneflow/issues/1279
[issue-1271]: https://github.com/illusion-tech/laneflow/issues/1271
[issue-1266-receipt]: https://github.com/illusion-tech/laneflow/issues/1266#issuecomment-6651487168
[v0-10-trial]: https://github.com/illusion-tech/laneflow/blob/e267d4c9eead22d14f7111555a70c70637cd9f04/tools/laneflow-gate-summary/fixtures/v0.10-trial/unified-gate.toml

[v0-10-record]: https://github.com/illusion-tech/laneflow/blob/3f80b65f3fee19cd6085fc0d8be8f58b7a14d551/docs/roadmap.md
