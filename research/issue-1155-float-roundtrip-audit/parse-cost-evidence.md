# #1155 ③ 固定输入解析耗时对照（同机描述性基线）

- 日期：2026-10-29；机器：AMD Ryzen 9 9955HX（LF-P100-REF-01 同机）；rustc 1.99.1；release；
  serde_json 1.0.154。数值只描述本机，不是产品阈值（沿 §8 口径）。
- 取证入口：`calibration/float_roundtrip.rs::fixed_input_parse_cost_evidence`（`#[ignore]` 手动取证）。
- 固定输入：确定性 50,000 个浮点 token 的 JSON 数组文本，905,087 bytes——两形态字节完全一致
  （写侧 `zmij` 序列化不受 feature 影响，实测 bytes 相同）；解析目标 `Vec<f64>`。
- 方法：每轮 220 次完整解析，弃前 20 预热，取 200 样本中位与最小值；两种构建形态各自测两轮。
  形态 A = 单包（default，无 `float_roundtrip`，生产语义）；形态 B = `-p laneflow-lust-converter
  -p laneflow-post-p3-research`（feature 统一，`float_roundtrip` 生效）。

| 形态 | 轮次 | 中位耗时 | 中位 ns/token | 最小值 |
|---|---|---|---|---|
| A default（`float_roundtrip=false`） | 1 | `708.9 µs` | 14 | `652.3 µs` |
| A default | 2 | `662.4 µs` | 13 | `642.1 µs` |
| B float_roundtrip（=true） | 1 | `835.5 µs` | 16 | `810 µs` |
| B float_roundtrip | 2 | `902.4 µs` | 18 | `817.9 µs` |

## 对照结论（描述性）

- `float_roundtrip` 生效后解析耗时约为默认的 **1.2–1.4×**（中位对照：×1.18 / ×1.36；最小值对照：
  ×1.24 / ×1.27），即本浮点密集输入上 **+18% ~ +36%**（中位）/ **+24% ~ +27%**（最小值）——
  与上游注明该 feature 有浮点解析成本一致。
- 轮间波动（default 中位 708.9→662.4 µs；unified 835.5→902.4 µs）显示同机会有 ~±10% 噪声，
  单轮对照不足为凭，故按两轮区间报告；正式基线（若需）应固定环境多轮取分布。
- 成本归属：解析侧唯一受影响路径（本审计 §2 实证写侧无损）；E 的文本域助手（`json_leaf_*`）
  比较职责与解析器无关，不受修复影响（issue 范围 3）。

## 复现

```text
cargo +1.99.1 test --release --locked -p laneflow-lust-converter --lib fixed_input_parse_cost_evidence -- --ignored --nocapture
cargo +1.99.1 test --release --locked -p laneflow-lust-converter -p laneflow-post-p3-research --lib fixed_input_parse_cost_evidence -- --ignored --nocapture
```
