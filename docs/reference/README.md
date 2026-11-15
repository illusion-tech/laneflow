# 参考资料

本目录保存可复用的辅助规范，不承载高层决策，也不替代设计文档。

适合放在这里的内容：术语表、命名约定、校验矩阵、机器可读编制种子。不适合：
里程碑最终审阅、逐任务验证流水账、性能轮次原始 JSON、smoke 截图。那些当时证据
只保留在 git 历史与对应 GitHub Issue / PR。

## 当前文档

- [`glossary.md`][glossary]：双语术语唯一事实来源；以中文术语和中文定义为准。
- [`capabilities.md`][capabilities]：当前实现、使用条件与已知限制的索引。它连接专题合同和少量实现证据，是现行事实的阅读入口，不替代设计，也不记录计划进度。
- [`commit-convention.md`][commit-convention]：Conventional Commits、`Refs` / `Closes`、必要时的
  `BREAKING CHANGE:`，以及 AI 署名的 `Harness:` / `Model:`；PR 默认经 Merge Queue
  最终 Rebase。
- [`rust-code-style.md`][rust-code-style]：`rustfmt` 无法表达的仓库级可读性约定。
- [`validation-matrix.md`][validation-matrix]：任务类别到最小验证要求。
- [`documentation-checks.md`][documentation-checks]：离线文档结构检查的命令、范围、锚点规则与能力边界。它只检查路径和锚点，不判断文档语义。
- [`mcpls.md`][mcpls]：可选 Rust 语义开发工具的安装、信任与回退边界。
- [`road-editing-source-semantic-seed-v1.json`][semantic-seed-json]：
  #296 不可变 benchmark 语义种子；只供 test/research generator 读取，不是 production JSON。
- [`road-editing-source-reference-machine-v1.json`][reference-machine-json]：
  #296 道路编辑来源校准参考机。

#308 研究执行器与 R0 巨大 Evidence 不在当前工作区。G4 精确证据提交为
[`de4cd460a96415cafbd811141568b81f74d73534`][calibration-tree]，
交付 PR 为 [#310][pull-310]。

三组 v0.10 历史测量基线（compiler production baseline、attestation production
baseline、direct-digest optimization baseline 的 Markdown 与 JSON）已退役；仍有效的
聚合结论与适用限制原位保留在现行责任文档（编译器见
`../design/compiler-foundation.md` §11.5，认证与摘要成本见
`../design/traffic-runtime-state-integrity.md` §8）。追溯原件使用固定提交
[3f80b65f][retired-baselines-tree] 下的 `docs/reference/` 副本。

[glossary]: glossary.md
[capabilities]: capabilities.md
[commit-convention]: commit-convention.md
[rust-code-style]: rust-code-style.md
[validation-matrix]: validation-matrix.md
[documentation-checks]: documentation-checks.md
[mcpls]: mcpls.md
[semantic-seed-json]: road-editing-source-semantic-seed-v1.json
[reference-machine-json]: road-editing-source-reference-machine-v1.json
[calibration-tree]: https://github.com/illusion-tech/laneflow/tree/de4cd460a96415cafbd811141568b81f74d73534
[retired-baselines-tree]: https://github.com/illusion-tech/laneflow/tree/3f80b65f3fee19cd6085fc0d8be8f58b7a14d551/docs/reference
[pull-310]: https://github.com/illusion-tech/laneflow/pull/310
