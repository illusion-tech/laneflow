# JSON 浮点解析精度缺口生产路径审计 — 审计结论（#1155 交付物 ① 定稿）

- 状态：**审计结论修订版 R2**（维护者 2026-10-29 两轮审定：R1 撤回 integrity_soak「影响面为零」
  改逐谓词待验证、双态测试证明力降级；R2 补持续时间链丢位实证并分列两项待验证、LFCA 格式依据
  更正为 LaneFlow 封闭节目录格式位编码、两处翻转论证收紧到输入域 + 解析器行为、「均不含浮点」
  限定为列举路径、写侧依赖更正为 `zmij`）。
  路径清单见同目录 [`path-inventory-draft.md`](path-inventory-draft.md)，初稿已过 lead 初审；
  本文件汇总结论与证据，供 lead 转维护者定稿。
- 工作树：`E:/projects/worktrees/float-roundtrip-audit/laneflow`，分支 `research/1155-float-roundtrip-audit`；
  审计基线 `47be29a5`（origin/main），测试提交 `777ad953`（叠 #1153 助手分支 `5a59a759`）。
- 派工勘误（lead 2026-10-29 确认）：必读清单中 `docs/reference/snapshot-format.md` 系派工笔误、仓库无此文件，
  **以 `docs/design/traffic-runtime-snapshot.md` 为准**。E 的 `json_leaf_tokens`/`json_leaf_mismatch` 位于
  PR #1153（分支 `research/1140-shared-baseline-hard-gates`），本切片合并该分支复用；C 的
  `compare-anchor-bits.ps1` 为本机证据工具不入库，入库交付物用 Rust `cfg(test)` 形态。

## 1. 结论（影响面判定）

1. **生产持久化 / 摘要 / 验收判定路径未发现实际丢位，不提级。** 依据逐路径证据（路径清单 §1 A/B 表）：
   - LFRS 快照与确定性状态摘要为整数规范字节 + FlatBuffers，无浮点文本、无 JSON
     （`laneflow-runtime` 生产依赖树无 serde_json；快照一维数值全整数 mm/µm/`u32` mm/s）；
   - LFCA / LFSM / LFSD / LFCP 规范制品为 **LaneFlow 自有封闭节目录格式**（非 FlatBuffers、
     非 JSON；`docs/design/portable-canonical-artifact.md:50-53`），浮点 f32/f64 以 **IEEE 754
     小端位编码** 写入——生产读写 `crates/laneflow-format/src/writer.rs:1116-1117`
     （`FieldWriteValue::F32/F64 => cursor.u32/u64(value.to_bits())`）与 `object.rs:495-500`
     （`from_bits` 解码），`table.rs:720-743` 拒 NaN/inf/−0.0；不经十进制文本/JSON
     （FlatBuffers 仅为 LFRS 快照容器与未发布 Road Editing 编制输入，同文档 :53；「u64 位三元组」
     是研究 JSON `*_f64_bits` 惯例、非 LFCA 编码形态——原稿此节误写，维护者复审更正）；
   - **列举的**生产 JSON 判定路径不含浮点解析值（仅限以下已审计路径，不作全称断言）：
     stub 授权门控只取字符串身份 + net_digest；caller.json / TickRecord / TickTiming / RunResult
     全 `derive Eq` 无浮点字段；events / commands / 观测摘要 JSON 全整数与字符串；
     ConversionReport / provenance 无浮点字段。（含浮点的判定路径见 §1.2。）
2. **「浮点解析→判定」形态集中于 urban-harness integrity_soak**（worker 把
   `active_seconds`/`paused_seconds`（f64）写入 summary.json，`supervisor.rs:295-296` 解析回判定；
   `tests.rs` 另有阈值断言）。**未发现实际丢位或判定翻转，相关浮点判定路径仍需验证**。
   逐条谓词、容差与边界余量论证如下（漂移量级：1 ULP 满足 ULP(x) ≤ x·2⁻⁵²，1–100 s 量级值
   约 ≤ 1.4×10⁻¹⁴ s；「余量未证」= 静态审计无法证明运行值距判定边界的距离大于漂移量，
   一律记**待验证**，不得记零影响）：

   | 判定谓词（supervisor.rs / tests.rs 行号） | 容差 | 边界余量论证 | 判定 |
   |---|---|---|---|
   | `(active − covered×0.016).abs() ≤ 0.016`（:317，虚拟钟） | 有（±0.016 s） | 翻转需原值距边界 ≤ 漂移量（~10⁻¹⁴ s 量级）；运行值分布未证 | **待验证** |
   | `paused ≤ elapsed().as_secs_f64()`（:318，虚拟钟） | **无** | 翻转需 paused 距实时墙钟 ≤ 漂移量；余量由摘要落盘→判定间开销承担，未证 | **待验证** |
   | `active + paused ≤ elapsed().as_secs_f64()`（:320，真实钟） | **无** | 两解析侧 f64 各 ±1 ULP 求和后入比较；同上未证 | **待验证** |
   | `active < spec.config.seconds` 判失败（:333） | **无** | 翻转需 measured active 距阈值 ≤ 漂移量；worker 以「active ≥ 请求」为停机条件，超调余量由循环粒度决定，未证 | **待验证** |
   | `!active.is_finite()`（:332）/ `!paused.is_finite()`（:334） | — | 基于输入域 + 解析器行为证明（非一般性质）：本路径 f64 输入恒为 `Duration::as_secs_f64()` 输出（量级 ≤ u64::MAX 秒 ≈ 1.85×10¹⁹，远离溢出阈值），token 为 zmij 有限十进制字面量，解析结果受字面量量级约束必有限 | **论证成立（前提：输入域不变量 + 解析器量级行为）** |
   | `paused < 0.0` 判失败（:335） | **无** | 基于 token 域 + 解析器符号行为（非一般性质）：非负 Duration 的 zmij 字面量无负号 token，解析按量级舍入不引入符号 → 结果 ≥ +0.0 | **论证成立（同前提）** |
   | `tests.rs:223-224`（`paused ≥ 0.4`、`active ≥ 4.0`）、`tests.rs:327`（`wall ≥ 30.0`） | **无** | 与 :333 同形态：阈值超调余量由运行时机决定，未证 | **待验证** |

   判定输入域注记：`elapsed()` 为判定时实时测量（不经 JSON），漂移仅作用于 JSON 解析侧的
   `active`/`paused`；`covered`/`requested_seconds` 为整数域（精确）。维护者口径：即使有容差
   也须证明距判定边界的余量——故容差行同样记「待验证」。上表两项「论证成立」**不是一般性质**：
   其成立依赖①输入域不变量（字段值仅来自 `Duration::as_secs_f64` 的非负有限输出、summary.json
   由 harness 自写，威胁模型「不防有意伪造」见 `report/timing.rs:74`）与②解析器行为（非负字面量
   不引入符号、量级受限字面量不溢出）；输入域或解析器变化须重证。验证手段（修复阶段或独立
   切片）：对上述谓词注入 ±1 ULP 边界值做判定翻转测试，或对运行值实测边界距离分布。
3. **丢位实证分列如下，各有限定**：
   - **既有事故记录（限定：已记录的实际丢位事故仅在研究导出工具）**：#1140 重锚
     parse→re-serialize ±1 ULP（18/53 分量 + 18/84 路径），与 serde_json 1.0.154 默认解析
     1 ULP 误舍入机理一致（本审计实测复证，见 §2），非生产路径，已文本拼接硬门钉版
     （`shared_baseline_anchor_text_splice_is_byte_faithful`）。历史研究结论有效（lead 已裁定）。
   - **合成输入实证（维护者 2026-10-29 复现，`target/issue-1155-review/probe.rs`，本审计已独立
     复证）**：soak 同款 `Duration → as_secs_f64 → JSON → Value` 链默认解析丢位——
     `Duration::new(1, 774_304_400)` 写出 `1.7743044000000001`（bits `4610669575807328744`）
     解析回 `4610669575807328743`（−1 ULP）；10,000 抽样 **1,444 个丢位**（±1 ULP 双向，另见
     `1.8793370999999999` +1 ULP、`1.8715220000000001` −1 ULP）；启用 `float_roundtrip` 后
     **0/10,000**。三例已入位级回归语料（`float_roundtrip.rs`），双形态 6/6 复证钉版数值。
   - **仍待验证（分列，不并入上两项结论）**：① 真实运行产物是否丢位（soak/生产实际 JSON 数据
     是否出现丢位 token）→ **待验证**；② 验收判定是否翻转（阈值谓词在实际运行值上是否被 1 ULP
     翻转）→ **待验证**（虚拟时钟 100 抽样 0 丢位/0 翻转、1,444 丢位样本未见阈值翻转打印，均系
     小样本观察，不作论证）。
4. **Feature 统一性实证（「单包构建验证」必要性核心证据）**：`float_roundtrip` 当前仅由
   `research/issue-762-post-p3-hotspots`（**workspace 成员**）请求 → workspace 测试构建经 feature 统一
   使全workspace共享 `float_roundtrip`（**掩盖缺口**），单包生产构建没有（**暴露缺口**）。
   `cargo tree --locked -e features -i serde_json` 与双形态测试二进制（`b5b263…` vs `32f58c…`）为实证。
5. **边界观察**：`routes.toml` / `manifest.toml` 的 f64 字段走 toml crate 序列化（非 serde_json 域），
   不在本切片缺口面；若需 TOML 浮点审计另立切片。`ExactDecimal::to_f64`（`sumo/decimal.rs`）以
   `f64::from_str` 一次正确舍入，是仓库内文本域正确语义先例。

## 2. 位级往返测试与已知反例回归（#1155 交付物 ②，已入库 `777ad953`）

测试模块：`tools/laneflow-lust-converter/src/output/emit/s1/recovery_coverage/dependency_recovery/calibration/float_roundtrip.rs`，
比较语义全部走文本 token 域（复用 #1153 `json_leaf_tokens`/`json_leaf_mismatch`，数字叶 `f64::from_str`
正确舍入 → `to_bits`）；serde 解析域位值只作被测对象。

**已知反例实测钉版**（serde_json 1.0.154，单包构建 = 生产 default 形态）：

| 文本 token | 正确舍入 bits（`f64::from_str`） | 默认解析 bits | 默认再序列化 token |
|---|---|---|---|
| `10.220051765441895` | `4621942995720077312` | `4621942995720077313`（+1 ULP） | `10.220051765441896`（丢源 token） |
| `10.220051765441896` | `4621942995720077313` | `4621942995720077313`（正确） | `10.220051765441896` |
| `0.009999999999999787` | `4576918229304087552` | `4576918229304087553`（+1 ULP） | `0.009999999999999789`（丢源 token） |
| `0.009999999999999789` | `4576918229304087553` | `4576918229304087553`（正确） | `0.009999999999999789` |
| `1.7743044000000001` | `4610669575807328744` | `4610669575807328743`（−1 ULP） | ≠ 源 token（丢位漂移） |
| `1.8793370999999999` | `4611142601035910460` | `4611142601035910461`（+1 ULP） | ≠ 源 token（丢位漂移） |
| `1.8715220000000001` | `4611107404954462598` | `4611107404954462597`（−1 ULP） | ≠ 源 token（丢位漂移） |

- 两组反例对（…895/…896、…787/…789）各自恰为相邻 1 ULP；写侧序列化文本 token 逐字保真、位值往返恒等
  （写侧无损，缺口纯在解析侧）。
- 后三行为 soak 同款时长链丢位实证样本（维护者探针 1,444/10,000 中的代表例，±1 ULP 双向），
  已入 `float_roundtrip.rs` 语料并双形态复证。
- **默认解析下两组反例对各自折叠到同一 bits**（…895/…896 均 → …313；…787/…789 均 → …553）——
  解析域折叠对漏报形态的直接实证；文本 token 域恒可区分。
- 有损再序列化（#1140 事故形态）在文本 token 域可检出，行为随构建形态钉死。

**双形态验证**（6 项测试 × 2 形态，均 6/6 绿）：

```text
cargo +1.99.1 test --locked -p laneflow-lust-converter --lib float_roundtrip
cargo +1.99.1 test --locked -p laneflow-lust-converter -p laneflow-post-p3-research --lib float_roundtrip
```

前者 = 单包 default 形态（生产语义，误舍入分支），后者 = feature 统一形态（`float_roundtrip` 分支）；
测试二进制哈希不同（`laneflow_lust_converter-b5b263…` / `-32f58c…`）证明是两个独立 feature 解析形态。

**证明力边界（维护者 2026-10-29 审定）**：双态自适应断言构成的是**双态差异实证 + 自适应对照**，
**不构成修复生效证明**——feature 缺失时误舍入分支同样通过（`float_roundtrip.rs` 的形态探测分支）。
充分性论证归修复阶段：启用 [`fix_gate_mandatory_correctly_rounded_bits`](../../../tools/laneflow-lust-converter/src/output/emit/s1/recovery_coverage/dependency_recovery/calibration/float_roundtrip.rs)
（无条件断言 …312/…552 等钉版正确位值，feature 缺失时必失败，现 `#[ignore]` 随修复 PR 启用）
并在**单包正常构建**下核验 `float_roundtrip` 生效。双态回归的既有价值不变：任何新静默漂移
（依赖版本漂移等）在两形态下同时失败。

整包回归：`cargo +1.99.1 test --locked -p laneflow-lust-converter --lib` → 287 通过 / 0 失败 / 38 ignored。

## 3. 与交付物 ③④ 的边界（本文件不含）

- **③ 固定输入解析耗时对照（已完成，见 [`parse-cost-evidence.md`](parse-cost-evidence.md)）**：
  release、同一 50,000-token/905,087-byte 固定输入、双形态各两轮（各 200 样本中位）：
  `float_roundtrip` 解析耗时约为默认 **1.2–1.4×**（中位 +18%~+36%，最小值 +24%~+27%），
  与上游注明该 feature 有解析成本一致；成本纯在解析侧（写侧无损已证），E 文本域助手不受影响。
- **④ 修复实施（六路收口后）**：Cargo.toml 启用 serde_json `float_roundtrip`；启用
  `fix_gate_mandatory_correctly_rounded_bits`（无条件正确位值断言，即修复生效的充分性证据）
  并在**单包正常构建**下核验 feature 生效（防测试构建 feature 合并掩盖）；应不变产物逐字节核验、
  修正项逐字段解释、**禁止批量刷新基线**；integrity_soak 浮点判定谓词（§1.2 表）的边界余量
  验证随修复或独立切片补做；全量回归 + 钉版复验 + 差异说明。
- **E 助手合同**：E 的文本域助手继续承担原文字节比较职责，不受本修复影响（issue 范围 3）。

## 4. 证据索引

- feature 实况：`cargo tree --locked -e features -i serde_json`（本树实测，输出存档于审计记录）；
- 路径逐条证据（文件:行号）：[`path-inventory-draft.md`](path-inventory-draft.md) §1 表格；
- 位级证据与测试：`calibration/float_roundtrip.rs`（本提交）；反例对实测上表；
- 修复生效检查面与负对照：`fix_gate_mandatory_correctly_rounded_bits`（`#[ignore]` 待修复启用）；
  负对照实测：default 形态下运行失败 `float_roundtrip must be active`（证明检查面有牙齿）；
- 解析成本对照：[`parse-cost-evidence.md`](parse-cost-evidence.md)（双形态各两轮实测）；
- 事故史证：`research/issue-1140-shared-baseline/README.md` 差异记录（lead 独立复验）；
- 合成输入丢位实证：`target/issue-1155-review/probe.rs`（维护者复现；`default.txt`/`roundtrip.txt`
  同目录存档：default 1,444/10,000 丢位、roundtrip 0/10,000）；三例语料经本模块双形态复证；
- LFCA 格式依据：`docs/design/portable-canonical-artifact.md:50-53`；生产读写
  `crates/laneflow-format/src/writer.rs:1116-1117`、`object.rs:495-500`（位编解码）、
  `table.rs:720-743`（值域校验）；
- 比较语义口径：`research/issue-1139-signal-diag-2/README.md` 第 4 条；`sumo/decimal.rs` `ExactDecimal::to_f64`。
