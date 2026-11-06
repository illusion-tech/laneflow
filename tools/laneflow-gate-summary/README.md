# 统一五轴汇总 Gate 执行器

交付 [#1268](https://github.com/illusion-tech/laneflow/issues/1268)：以 `LF-SYNTH-v2`
与中国特色城市负载各轴（behavior / determinism / performance / snapshot / routing）
既有正式记录为**声明式输入**，产出统一汇总回执——逐格携带来源提交、被测二进制、
输入身份、覆盖边界与例外状态，供 [#305](https://github.com/illusion-tech/laneflow/issues/305)
最终验收消费。

边界：不运行正式汇总、不替代各轴分立 harness、不重跑正式测量、不批准例外；
全部来源身份从既有记录**原样携带**，本工具不重算摘要（ADR 0033 §3）、不联网
核验、不执行测量；离线运行。`research/` 等仓库文件只在运行时读取，不编译期嵌入。

## 命令

```powershell
cargo +1.99.1 build --release --locked -p laneflow-gate-summary

# 汇总（试跑：允许未完成项）
target/release/laneflow-gate-summary summarize `
  --manifest tools/laneflow-gate-summary/fixtures/v0.10-trial/unified-gate.toml `
  --repo-root <checkout> `
  --evidence-root <evidence-root> `
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

试跑副本在 `fixtures/v0.10-trial/unified-gate.toml`，其结构即文档：

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

## #305 正式汇总的命令与输入

前置：#1282 冻结验收提交；#1279 完成冻结后计时并形成记录（stdout 捕获为
key=value 文本）；L1 两份输入包（`10000-L1-16`、`100000-L1-33`）完成核查。

1. 制作正式 manifest：复制试跑副本到仓库外工作目录，改 `mode_note`；逐格更新：

   - routing 两格：补 `#1279` 记录与 #1267 证据结论，`status` 离开 `pending`。
     #1279 每个身份一条 `routing-kv`，`expect` 至少含
     `workload_id=LF-ROUTING-CANDIDATE-LFSYNTH-v1` 与 `identity=<身份>`；
   - 城市性能格：写入验收提交适用性核对结论，清空或改写 `pending`；
   - 各语义格按需挂接 `synth-summary-json` / `gate-json`（`expect` 绑定
     `implementation_commit` / `source.commit` 等身份）。

2. 对冻结的正式 L1 输入包运行核查（两份身份各一次）：

   ```powershell
   target/release/laneflow-gate-summary check-transitions --bundle <formal-inputs>/10000-L1-16 --output <evidence-root>/1268-formal/l1-check-10000-L1-16.json
   target/release/laneflow-gate-summary check-transitions --bundle <formal-inputs>/100000-L1-33 --output <evidence-root>/1268-formal/l1-check-100000-L1-33.json
   ```

3. 运行正式汇总：

   ```powershell
   target/release/laneflow-gate-summary summarize `
     --manifest <正式 manifest> `
     --repo-root <checkout@验收提交> `
     --evidence-root <evidence-root> `
     --output <新目录> `
     --mode formal --require-complete
   ```

4. 回执结论回写 #305（GitHub 可链接结论；如需入仓，research/ 结论目录只放
   结论 md 与紧凑 receipt 副本，遵守单文件 ≤ 1,000 行限额，不拷过程数据）。

正式 L1 输入包从 `laneflow-synth-generator generate` 按冻结身份重建，或直接使用
正式矩阵的输入缓存目录；核查对象身份（manifest 摘要与来源提交）由记录原样携带。

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
cargo +1.99.1 test --locked -p laneflow-gate-summary
cargo +1.99.1 clippy --locked -p laneflow-gate-summary --all-targets -- -D warnings
```

`tests/summary.rs` 覆盖 fixture 汇总与全部失败关闭路径（性能判定词、锚点未命中、
身份不符、未完成项拒绝、占位路径缺根、输出目录已存在）；`tests/l1_generated.rs`
为忽略级进程内全链（编译 LFCA 属重负载，按窗口手动运行）。

## 关联

[#1268](https://github.com/illusion-tech/laneflow/issues/1268)、
[#539](https://github.com/illusion-tech/laneflow/issues/539)、
[#305](https://github.com/illusion-tech/laneflow/issues/305)、
[#1279](https://github.com/illusion-tech/laneflow/issues/1279)、
[#1271](https://github.com/illusion-tech/laneflow/issues/1271)、
[#1266 回执](https://github.com/illusion-tech/laneflow/issues/1266#issuecomment-6651487168)、
ADR 0033（摘要角色；本工具不新增摘要）、
`docs/governance/documentation-policy.md` §3（研究结论与过程数据边界）。
