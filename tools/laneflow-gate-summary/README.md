# 统一五轴汇总 Gate 执行器

交付 [#1268][issue-1268]：以 `LF-SYNTH-v2`
与中国特色城市负载各轴（behavior / determinism / performance / snapshot / routing）
既有正式记录为**声明式输入**，产出统一汇总回执——逐格携带来源提交、被测二进制、
输入身份、覆盖边界与例外状态，供 [#305][issue-305]
最终验收消费。

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

# L1 输入 #1271 拓扑核查（对一份 LF-SYNTH 输入包目录）
target/release/laneflow-gate-summary check-transitions `
  --bundle <LF-SYNTH 输入包目录> `
  --output <尚不存在的 json 记录文件>
```

- `--repo-root`：manifest 内仓库相对路径的根（默认当前目录）。
- `--evidence-root`：manifest 内 `<evidence-root>/` 前缀占位路径的根。仓库外证据
  目录一律用该占位书写，不写采集机本机布局（`documentation-policy` §3）。
- `--mode formal`：正式模式，拒绝 `pending` 状态格；`--require-complete`：拒绝
  任何带未完成项的格。#305 正式汇总两者都开。
- 输出目录/文件必须不存在（沿 harness 家族「不覆盖旧交付」惯例）。

产物：`receipt.md`（人读）+ `receipt.json`（机器可读，`lf-unified-gate-receipt-1`）。
回执是确定性的——同一 manifest 与同一批记录产生逐字节相同的回执，便于试跑与
正式汇总对拍。

## 声明式输入（`lf-unified-gate-manifest-1`）

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
- `[[scope_adjudication]]`：#1268 正文九项裁定表的声明式副本（豁免须带授权）。
- `[[capability_limits]]`：能力限制登记（#1217 结构化拒绝、#1220 保守拒绝、
  #1271 平行边盲区、#1272 硬停限制）。
- `required_l1_identities` + `[[l1_checks]]`：L1 输入 #1271 拓扑核查——顶层声明
  本轮要求完成核查的全部输入身份，挂接记录必须**恰好覆盖**该集合（缺失、
  重复或多挂都失败），防止漏检身份带着「已核查」外观进入结论；每条记录的
  `verdict` 无条件要求 `clean`，且 `bundle.identity` 须与声明逐字符串相等。

证据锚点种类：`repo-doc`（存在 + 锚点子串）、`github`（URL 形态）、
`synth-summary-json`（F 矩阵 `summary.json` 身份对账；二进制摘要同时消费语义
矩阵的顶层 `binary_sha256` 与 F1/F2/F3 的嵌套 `binary.sha256`，两者同时存在即
失败）、`gate-json`（城市 `gate.json` 身份对账）、`routing-kv`（墙钟记录头行：
识别 §7.1 的 `routing-g2-wall-clock` 与 #1279 运行的 `routing-lfsynth-wall-clock`；
标记前后落在词边界即可定位，容忍 `--nocapture` 捕获的 libtest 前缀，带
`operation=` 的统计行不算头行；同一文件不得混用两种标记，多轮拼接时每个头行
都须命中；`expect` 须含 `workload_id`，LF-SYNTH 记录还须含 `identity`；期望键
缺失不容忍、未知键前向容忍）、`l1-record`（本工具 `check-transitions` 记录）。

## #305 正式汇总执行记录（已完成）

v0.10 统一验收线已收尾：#1282 冻结验收提交，#1279 完成冻结后计时并形成记录，
两份正式 L1 输入包（`10000-L1-16`、`100000-L1-33`）完成拓扑核查，#305 于
2026-11-09 完成（completed）关闭。正式 manifest 逐格更新、L1 核查与正式汇总的
一次性操作明细见基线提交 3f80b65f 的本文副本；通用命令、manifest 结构、身份
校验与 fail-closed 语义以上文各节为准，未来新验收轮按同语义新建 manifest，
不复用历史试跑清单。

## L1 拓扑核查的判定（#1271）

对每条横向过渡段 `T`（与运行时同判据：不属于任何编制车道、不属任何路口）：

1. 可见邻域 `N(T) = {T} ∪ 前驱 ∪ 后继`（运行时 P4 的可见口径）；
2. 汇入窗口锚定在后渡编制车道自身的链坐标上——车道链原点不共享（L1 的
   目标车道链物理起点偏移 5 m），跨链比较必须以邻域边为锚：
   `[锚边链起点 − len(T), 锚边链起点)`；无锚车道的边不参与判定；
3. 锚车道内纵向跨度与窗口相交、且不在邻域内的编制车道边为平行边候选
   （候选携带锚边与窗口，便于独立复核对齐关系）；
4. 存在**不经过邻域任何边、却经过平行边候选**的已注册路线 → 违规
   （P4 看不见的直行车实际在跑）；
5. 后继编制车道链上、汇入边之前出现分岔（多后继或后继跨多条编制车道）→
   使能结构，一并计入 `violations-found`。

汇总侧同时强制：`[[l1_checks]]` 声明的身份必须与记录自身携带的
`bundle.identity`（`<scale>-<label>-<delta>`，`LaneChange` 归一为 `L1`）逐字符串
相等，挂错记录即失败。结论 `clean` 表示该输入包不含 #1271 形制的拓扑；含违规
拓扑的路网不得用于安全验收（#1271 处置）。核查只读静态包（LFCA 受检读取 →
共享静态路网），不构造世界、不推进 tick、不含横向几何判定（运行时同样没有）。

## 测试

```powershell
cargo test --locked -p laneflow-gate-summary
cargo clippy --locked -p laneflow-gate-summary --all-targets -- -D warnings
```

`tests/summary.rs` 以示例 manifest 覆盖汇总与全部失败关闭路径（性能判定词、锚点未命中、
身份不符、未完成项拒绝、占位路径缺根、输出目录已存在）；`tests/l1_generated.rs`
为忽略级进程内全链（编译 LFCA 属重负载，按窗口手动运行）。

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
