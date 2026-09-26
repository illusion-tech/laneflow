# P3 合入后的剩余整拍热点

Refs #762。固定主干 `37c5e1af3c72b0f60cf2dd889bbf6713b2a77054`，只交付研究工具、
诊断证据及下一优化切片的选择。无正式 Runtime/API/数据格式/Adapter 行为变更。

已完成 12 次首轮运行与 3 次冲突准备细分，结论见 [研究结果](results.md)。
当前工具统一为 Rust workspace 包 `laneflow-post-p3-research`，无需 Python；
迁移验证与历史证据边界见 [Rust 迁移记录](migration.md)。

## 测量合同

- 原 #707 `4de40e04` 冻结包的 10k/100k `*-smoke.toml`，workers=4，每次 256 拍。
- 每规模 plain/stages/stages/plain/plain/stages；各三轮独立进程，测量串行执行。
- plain 保留主干 Runtime 原样；两臂 Harness 均在公共 `step` 计时结束后输出逐拍记录。
- stages 仅激活既有十段协调器批次墙钟，额外区分 ConflictPrepare 内 Frontier 与 P4。
  嵌套两段不得再次加入十段合计。计时涵盖等待 worker 完成；不累加 worker CPU。
- 分别报告全窗、1–64 拍、65–256 拍。这里的前缀分窗不是正式暖机或稳态声明。
- 三轮主要段排名一致且领先超过该段自身运行间均值极差，才据此指定下一原型；
  差异不可分辨就明确保留不确定性，不自动扩展为长测。
- 核验源码/二进制/输入身份、阶段次数、非负残差、两臂交通日志及结果语义一致。
  这是观测补丁的透明性检查，不给后续模型优化附加历史轨迹等价要求。
- Windows 开发机未完全隔离后台活动；正式性能预算与长期质量认证仍由 #707 承担。

首轮三次 100k 诊断均以 ConflictPrepare 为最大段；因此追加三次同输入、同 256 拍的
100k detail 诊断，细分发现、分发、消费、融合回退及排序预留。只观测协调器时钟与
已存在的 `inputs.len()`，不增加逐车时钟。结束条件是三次细分结果齐全，不追加长窗。

## Rust 工具

需要 Rust 1.98.0、Git 和系统 `tar`。以下命令均从仓库根目录执行。
先运行 `cargo build --locked -p laneflow-post-p3-research`，得到
`target/debug/laneflow-post-p3-research`（Windows 加 `.exe`）。下文简称该程序为 `TOOL`。

| 子命令    | 参数                                                        | 用途                            |
| --------- | ----------------------------------------------------------- | ------------------------------- |
| `prepare` | `<plain\|stages\|detail> <new-source> <new-index>`          | 导出固定源码并生成 SHA-256 索引 |
| `run`     | `<base\|detail> <new-output> <inputs> [parent-results]`     | 串行采集，保存进程和输入身份    |
| `analyze` | `<base\|detail> <raw> <new-results> [parent-results]`       | 校验原始证据并生成统计          |
| `verify`  | `<base\|detail> <raw> <published-results> [parent-results]` | 只读复算并比对已发布证据        |

`detail` 必须提供首轮 `parent-results`；`base` 不接受该参数。输出路径必须是新路径，
分析输出不能位于原始包内。`verify` 严格比较统计值、文件摘要及身份字段，不以 JSON
缩进或键顺序作为语义差异，不覆盖原文件。

## 复现

准备三份导出树：

- `TOOL prepare plain target/hotspot-plain target/plain-source.json`
- `TOOL prepare stages target/hotspot-stages target/stages-source.json`
- `TOOL prepare detail target/hotspot-detail target/detail-source.json`

设置环境变量 `CARGO_INCREMENTAL=0`。各模式必须使用不同的 Cargo target-dir，避免同包
同版本导出树的增量指纹误复用。例如 plain：

`cargo +1.98.0 build --release --locked --offline -p laneflow-urban-harness --manifest-path target/hotspot-plain/Cargo.toml --target-dir target/hotspot-plain-build`

stages/detail 分别使用自己的导出目录和 `target/hotspot-stages-build`、
`target/hotspot-detail-build`。将各自产出的 `release/laneflow-urban-harness` 复制为
`target/hotspot-binaries/plain`、`stages`、`detail`（Windows 均加 `.exe`）。

从已提交、干净且可达的工具源码提交启动采集；将 `INPUTS` 替换为原 #707 冻结输入目录：

- `TOOL run base target/hotspot-runs INPUTS`
- `TOOL analyze base target/hotspot-runs target/hotspot-results.json`
- `TOOL run detail target/hotspot-detail-runs INPUTS target/hotspot-results.json`
- `TOOL analyze detail target/hotspot-detail-runs target/hotspot-detail-results.json target/hotspot-results.json`

核验历史包与仓库已发布索引：

- `TOOL verify base target/hotspot-runs research/issue-762-post-p3-hotspots/evidence/results.json`
- `TOOL verify detail target/hotspot-detail-runs research/issue-762-post-p3-hotspots/evidence/detail-results.json research/issue-762-post-p3-hotspots/evidence/results.json`

工具作为 workspace 成员参与 Rust CI。局部验证使用
`cargo test --locked -p laneflow-post-p3-research` 和
`cargo clippy --locked -p laneflow-post-p3-research --all-targets -- -D warnings`。

原始运行包、二进制和导出源码保存在独立工作树的 `target/`，不纳入 Git；远程复核需要
对应外部包及冻结输入。源码索引和原始文件清单用于核对身份，不表示外部证据已随 Git 发布。
