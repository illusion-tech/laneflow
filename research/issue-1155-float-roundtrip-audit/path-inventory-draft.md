# JSON 浮点解析精度缺口生产路径审计 — 路径清单初稿（#1155 交付物 ①）

- 状态：**初稿，待 lead 审**。本文件只含路径清单；位级往返测试、耗时对照与修复实施（②③④）另行交付。
- 工作树：`E:/projects/worktrees/float-roundtrip-audit/laneflow`，分支 `research/1155-float-roundtrip-audit`，基线 `47be29a5`（origin/main）。
- 缺陷口径（严格照 Issue #1155）：serde_json 1.0.154 默认解析对 17 位十进制浮点误舍入 1 ULP
  （`"10.220051765441895"` → bits …313，`float_roundtrip` 下正确为 …312）。**尚未证明 Runtime 快照受影响**，
  快照 LFRS/FlatBuffers 与 LFCA 位编码路径按背景口径仅审计确认、不预列故障面。
- 比较语义（沿 C 路三类）：解析域折叠（同解析器双侧比较）→ 对漏报敏感；「内存值 vs serde 解析值」形态 →
  误报/漏报均可能；文本 token 域严格。文本→f64 正确舍入参照为 std `f64::from_str`
  （仓库现有先例：lust-converter `src/sumo/decimal.rs` `ExactDecimal::to_f64`，逐字文本→一次正确舍入）。

## 0. 实际 feature 实况（证据）

- `Cargo.lock` 锁定 serde_json **1.0.154**（与 issue 复现版本一致）。
- `cargo tree --locked -e features -i serde_json`（本树实测）：`float_roundtrip` **仅**由
  `laneflow-post-p3-research`（`research/issue-762-post-p3-hotspots`，**workspace 成员**，根 `Cargo.toml:15`）请求；
  其余全部消费方（`xtask`、`laneflow-lust-converter`、`laneflow-urban-harness`、`laneflow-static-network`（dev））
  均为 **default（无 float_roundtrip）**。
- **Feature 合并陷阱（实证，对应 issue ② 的"单包验证"要求）**：issue-762 是 workspace 成员 →
  `cargo test --workspace` 等全工作区构建把 `float_roundtrip` 统一进共享 serde_json（resolver v2 只隔离
  build/normal 与 target 边，同一构建图内仍统一），测试面"看起来正确"；单包构建
  （如 `cargo build -p laneflow-urban-harness`）则**不含** float_roundtrip。生产二进制按单包/消费方构建，
  现状为默认解析。
- `laneflow-runtime` 生产依赖树**无 serde_json**（cargo tree 仅经 dev-dependency `dhat` 间接出现）。

## 1. 路径清单

### A. 生产运行时 · 持久化 · 摘要 · 判定（结论：无影响）

| 路径 | 读/写 | 浮点字段 | 实际 feature | 去向 | 证据与判定 |
|---|---|---|---|---|---|
| LFRS 快照 `encode_lfrs` / `restore_lfrs` | 二进制 FlatBuffers（**非 JSON**） | 无：一维数值全整数 mm/µm/`u32` mm/s | 不适用（无 serde_json） | 持久化（存档） | `docs/design/traffic-runtime-snapshot.md` §3"无浮点字段"、§4 容器（size-prefixed FlatBuffers、`LFRS`）；cargo tree 无 serde_json → **无影响** |
| 确定性状态摘要 `deterministic_state_digest` | 二进制规范前像（**非 JSON**） | 无：全整数小端 + `StableId128` | 不适用 | 验收判定（检查点对拍三元组） | 同上 §6 前像布局（u8/u16/u32/u64/16 字节 ID），BLAKE3-256 → **无影响** |
| LFCA / LFSM / LFSD / LFCP 规范制品（及 LFCA 构建的共享静态路网） | 二进制（LaneFlow 自有封闭节目录格式，**非 FlatBuffers、非 JSON**） | 浮点 f32/f64 以 **IEEE 754 小端位编码** 写入（`to_bits()` u32/u64 LE） | 不适用 | 静态数据 / 发布链 | 格式规范 `docs/design/portable-canonical-artifact.md:50-53`（FlatBuffers 仅为 LFRS 快照容器与未发布 Road Editing 编制输入，:53）；生产读写 `laneflow-format/src/writer.rs:1116-1117`（`FieldWriteValue::F32/F64 => cursor.u32/u64(value.to_bits())`）、`object.rs:495-500`（`from_bits` 解码）、`table.rs:720-743`（拒 NaN/inf/−0.0）；各 crate `Cargo.toml` 无 serde_json；不经十进制文本 → **无影响**。（原稿「FlatBuffers/u64 三元组」系误写，维护者复审更正：u64 三元组是研究 JSON `*_f64_bits` 惯例、非 LFCA 编码形态） |
| `laneflow-static-network` `page_injection_tests.rs` | JSON 读写 | `serde_json::Value`（状态/计数，无浮点语义） | default（dev-dependency） | 仅测试 | `crates/laneflow-static-network/Cargo.toml:24-29` serde_json 在 `[dev-dependencies]`；`src/lib.rs:28-29` `#[cfg(test)] mod page_injection_tests` → **生产无 JSON** |

### B. 生产工具 JSON 读写路径

| 路径 | 读/写 | 浮点字段 | 实际 feature | 去向 | 证据与判定 |
|---|---|---|---|---|---|
| lust-converter `ConversionReport` / `SemanticProvenanceManifest` / `BuildProvenanceRecord`（`output/mod.rs` `json_bytes`） | 写（`to_vec_pretty`） | **无 f32/f64** | default | 持久化（转换产物）+ 验收输入 | grep `output/report.rs`、`output/provenance.rs` 零浮点字段；读回仅 `#[cfg(test)]`（`report.rs:283`）→ **无影响** |
| lust-converter `routes.toml` / `manifest.toml` | 写（`toml::to_string_pretty`） | f64：`LaneEdge.length/speed_limit`、`VehicleProfile.*` | **TOML 域（非 serde_json）** | 持久化（转换产物） | `topology.rs:158`、`pipeline.rs:1345`；属 toml crate 解析域，**不在本切片 JSON 缺口面**；列为边界观察项（TOML 浮点审计如有需要另立切片） |
| lust-converter `StubWeldRecord` 诊断清单 | 写（JSON） | f64：`entry_end_m/weld_target_m/displacement_m/span_m/shape_len_m/locality_m/shared_traversals` | default | 诊断记录（不进判定） | `junction.rs:1016-1042`；manifest 生成为 `#[cfg(test)]`（`junction.rs:944-958`）；写侧 serde 序列化为最短往返表示，无解析 → **无影响** |
| lust-converter `evidence/lust-stub-weld-candidates.json`（pinned） | **生产读**（`approved_stub_set`） | 文件含 f64，但解析后**仅取字符串身份三元组 + net_digest** | default | **验收判定**（`BlockedOutOfDomain` 授权门控） | `junction.rs:1150-1177`：`ApprovedManifestView` 提取只用 `stub_lane_id/entry_lane_id/exit_lane_id` 与 `net_digest`，浮点解析后丢弃 → **判定谓词不含浮点值，无影响** |
| lust-converter emit/s1 证据文档（`source_length_m`、`source_length_delta_m` 等） | 写 | f64 | default | 研究/测试证据 | `output/emit/s1.rs:370`；读回仅测试（同解析器自洽，见 §2 比较语义）→ 不进生产判定 |
| urban-harness `caller.json`（`CallerState`，`urban-save-caller-v1`）+ `Domain::CallerState` 摘要 | 写 + 读（续跑恢复）+ 摘要比对 | **无浮点**（全结构 derive `Eq`） | default | **持久化 + 摘要**（save/resume） | `save.rs:40-75`；`VehicleState` 为整数 mm/µm（`crates/laneflow-runtime/src/kernel/vehicle.rs:18-31`，ADR 0028）→ **无影响** |
| urban-harness `tick.jsonl` / `timings.jsonl` / `result.json` / endpoint | 写 + 读（比较/校验） | **无浮点**（`TickRecord`/`TickTiming`/`RunResult` derive `Eq`） | default | 持久化 + 验收判定 | `runner.rs:27-54`、`report/timing.rs:16-22`、`report.rs:66-116` → **无影响** |
| urban-harness events/commands JSONL 与 `Domain::TickEvents`/`TickCommands` 摘要 | 写（`to_vec` 进摘要）+ sustained 读回重算摘要 | 无浮点（`json!` 字面量：tick/due/sequence/boundary/id/枚举字符串） | default | **摘要 / 验收判定** | `runner.rs:1097,1120,1289,1572`、`observe.rs:892,1041`、`sustained.rs:618,655`（parse→re-serialize 重算）；输入输出均无浮点 token → **无影响** |
| urban-harness 观测摘要（`urban-observation-v4`，`BlockHash::json`） | 写（`serde_json::to_writer` 入摘要字节） | 无浮点（passage/range/id/整数字段） | default | **摘要**（续跑判据） | `observe.rs:153-177`；仅写侧、无解析 → **无影响** |
| urban-harness `diagnostics.json`（`elapsed_seconds` 等） | 写 + 读 | f64：`elapsed_seconds` | default | 记录/展示 | `report.rs:842`；timing verify 的判定字段全为 `u64` ns（`timing.rs:63-69` `Envelope`）→ **判定无影响**，f64 仅展示 |
| urban-harness integrity_soak 控制/记录 JSON | **写 + 读 + 判定** | f64：`active/paused/wall_seconds`、`paused_interval_seconds` | default | soak 墙钟会计判定（容差 0.016 s 量级） | `control.rs:114-132,264-277`，读回 `control.rs:26,324,402`；判定 `supervisor.rs:295-333`（`as_f64` + 容差比较）→ **「浮点解析→判定」形态集中处**；未发现实际丢位或判定翻转，各判定谓词边界余量**待验证**（逐谓词表见 [audit-conclusion.md](audit-conclusion.md) §1.2，含无容差比较与 `active ≥ requested_seconds` 形态） |
| urban-harness presentation 采样（f32 位姿预览） | 写（trial JSONL `presentation`） | f32 位置数组 | default | 调试可视化 / 测试 | `presentation.rs:478-480`、`transitions.rs:397-403`；不进持久化/判定 → 无影响（测试比较若为解析域则折叠自洽，见 §2） |
| xtask（cargo metadata 解析、`commit_models`/`commit_harnesses` 目录、measurement receipts） | 读 + 写 | **无 f32/f64 字段**（grep 零匹配） | default | 开发工装 / CI | `xtask/src` grep `: (f32|f64)` 零匹配；内容为字符串/整数/日期 → **无影响** |

### C. 研究与测试 fixture 域（非生产；反例与比较语义来源）

| 路径 | 事实 |
|---|---|
| research JSON 基线（`issue-1103/1105/1107/1111/1113/1115*/results.json` 等） | 含 17 位十进制 `canonical_arc_m: 10.220051765441895` —— issue 反例对 …895/…896 的实证出处 |
| `research/issue-1140-shared-baseline` 重锚事故 | 导出 parse→re-serialize **已实际丢位**：源 `10.220051765441895`（bits …312）写成 `…896`（bits …313）、`source_length_delta_m` …787→…789，受影响 18/53 分量 + 18/84 路径；与 serde_json 默认解析 1 ULP 机理一致（非生产路径）。已由文本拼接修复 + 硬门测试 `shared_baseline_anchor_text_splice_is_byte_faithful` 钉版（lust-converter `shared_baseline.rs:61,738-751`：断言含 `…895`、不含 `…896`）。lead 已裁定历史研究结论有效 |
| lust-converter 测试读研究基线（`include_str` + serde parse + 比较） | 同解析器双侧比较 = 解析域折叠（对漏报敏感）；补位级核对须文本 token 域（`research/issue-1139-signal-diag-2/README.md` 第 4 条口径：`f64::from_str` 正确舍入或文本位级，勿用「内存值 vs serde 解析值」形态） |
| `research/issue-762-post-p3-hotspots` | float_roundtrip 已启用的唯一样本（研究 crate，workspace 成员） |
| population 迁移 JSON（`convert/population/migration.rs:29`） | `departSeconds` 以 `to_string()` **文本**写出 —— 文本域保真先例 |

## 2. 初步影响面判定（终稿随"审计结论"里程碑）

1. **生产持久化/验收路径已实际丢位：未发现** → 不触发提级条件。LFRS/确定性摘要/LFCA/wire 无 JSON、
   无浮点文本；生产 JSON 判定谓词（stub 授权门控身份匹配、timing verify、caller/checkpoint 摘要）
   不含浮点解析值。
2. 「浮点解析→判定」形态集中于 integrity_soak 墙钟会计（含无容差比较与 `active ≥ requested_seconds`
   阈值形态）：未发现实际丢位或判定翻转，浮点判定路径**仍需验证**（逐谓词边界余量见
   [audit-conclusion.md](audit-conclusion.md) §1.2，缺论证项一律记待验证），不作零影响结论。
3. 实际丢位史证仅出现在研究导出工具（issue-1140 重锚），非生产路径，且已有文本拼接硬门钉版。
4. **Feature 合并陷阱确认**：workspace 测试构建统一 float_roundtrip、单包生产构建无 → 修复验收必须
   单包正常构建验证 feature 生效（issue ② 要求，防测试构建合并掩盖）。

## 3. 待 lead 确认 / 补料

1. 必读清单中 `docs/reference/snapshot-format.md` **不存在**（本 worktree 与主检出均无此文件）；
   快照容器/字段事实以 `docs/design/traffic-runtime-snapshot.md` 为准（schema 在私有 wire package，
   `schemas/runtime-snapshot/v7` 不在本仓）。请确认口径或补正确路径。
2. E 的 `json_leaf_mismatch` 助手与 C 的 `compare-anchor-bits.ps1` **不在 origin/main 树**
   （仅 issue-1139 README 提及）。请提供所在路径，或按 README 描述语义执行
   （`f64::from_str` 正确舍入 / 文本 token 域位级比较）。
