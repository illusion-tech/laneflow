# JSON 浮点解析精度缺口生产路径审计 — 审计结论（#1155 交付物 ① 定稿）

- 状态：**审计结论定稿**（路径清单见同目录 [`path-inventory-draft.md`](path-inventory-draft.md)，初稿已过 lead 初审；
  本文件汇总结论与证据，供 lead 转维护者）。
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
   - LFCA / 共享静态路网 / wire 浮点为二进制位编码（u64 三元组），不经十进制文本；
   - 生产 JSON 判定谓词均不含浮点解析值：stub 授权门控只取字符串身份 + net_digest；
     caller.json / TickRecord / TickTiming / RunResult 全 `derive Eq` 无浮点字段；
     events / commands / 观测摘要 JSON 全整数与字符串；ConversionReport / provenance 无浮点字段。
2. **唯一「浮点解析→判定」形态**为 urban-harness integrity_soak 墙钟会计（f64 秒，容差 0.016 s 量级，
   `supervisor.rs` 判定）。1 ULP（±2⁻⁵² 量级）不可能翻转容差 → 影响面为零，列低风险观察项。
3. **实际丢位史证仅出现在研究导出工具**（#1140 重锚 parse→re-serialize ±1 ULP，18/53 分量），
   与 serde_json 1.0.154 默认解析 1 ULP 误舍入机理一致（本审计实测复证，见 §2），非生产路径，
   且已有文本拼接硬门钉版（`shared_baseline_anchor_text_splice_is_byte_faithful`）。历史研究结论有效（lead 已裁定）。
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

- 两组反例对（…895/…896、…787/…789）各自恰为相邻 1 ULP；写侧序列化文本 token 逐字保真、位值往返恒等
  （写侧无损，缺口纯在解析侧）。
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
测试对两形态自适应双态钉死，任何新静默漂移（依赖版本漂移等）都会失败。
**修复合入后，单包构建运行本模块即为「feature 生效」的固定检查面**（自动转入正确舍入分支），
直接落实防测试构建 feature 合并掩盖的要求。

整包回归：`cargo +1.99.1 test --locked -p laneflow-lust-converter --lib` → 287 通过 / 0 失败 / 38 ignored。

## 3. 与交付物 ③④ 的边界（本文件不含）

- **③ 固定输入解析耗时对照**（上游注明 `float_roundtrip` 有解析成本）：待做，方法为 release、固定输入、
  双形态中位对照；结论随成本数据单独报。
- **④ 修复实施（六路收口后）**：Cargo.toml 启用 serde_json `float_roundtrip`；应不变产物逐字节核验、
  修正项逐字段解释、**禁止批量刷新基线**；单包正常构建 + 本模块双形态检查面验证 feature 生效；
  全量回归 + 钉版复验 + 差异说明。
- **E 助手合同**：E 的文本域助手继续承担原文字节比较职责，不受本修复影响（issue 范围 3）。

## 4. 证据索引

- feature 实况：`cargo tree --locked -e features -i serde_json`（本树实测，输出存档于审计记录）；
- 路径逐条证据（文件:行号）：[`path-inventory-draft.md`](path-inventory-draft.md) §1 表格；
- 位级证据与测试：`calibration/float_roundtrip.rs`（本提交）；反例对实测上表；
- 事故史证：`research/issue-1140-shared-baseline/README.md` 差异记录（lead 独立复验）；
- 比较语义口径：`research/issue-1139-signal-diag-2/README.md` 第 4 条；`sumo/decimal.rs` `ExactDecimal::to_f64`。
