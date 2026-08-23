# 参考资料

本目录保存可复用的辅助规范，不承载高层决策，也不替代设计文档。

适合放在这里的内容：术语表、命名约定、校验矩阵、机器可读编制种子。不适合：
里程碑收口审阅、逐切片验证流水账、性能轮次原始 JSON、smoke 截图。那些当时证据
只保留在 git 历史与对应 GitHub Issue / PR。

## 当前文档

- `glossary.md`：双语术语单一事实源；中文术语和中文定义权威。
- `commit-convention.md`：Conventional Commits、`Refs` / `Closes`、必要时的
  `BREAKING CHANGE:`；PR 默认经 Merge Queue 最终 Rebase。
- `rust-code-style.md`：`rustfmt` 无法表达的仓库级可读性约定。
- `validation-matrix.md`：切片类型到最小验证要求。
- `mcpls.md`：可选 Rust 语义开发工具的安装、信任与回退边界。
- `v0.10-compiler-production-baseline.md` 与配对 JSON：#292 在 P100 推荐参考机型
  上运行真实生产 `Compiler::compile` 的现行预算基线；不是 #308 研究替身。
- `road-editing-source-semantic-seed-v1.json`：#296 不可变 benchmark 语义种子；
  只供 test/research generator 读取，不是 production JSON。
- `road-editing-source-reference-machine-v1.json`：#296 道路编辑来源校准参考机。
- `road-editing-source-workload-definition-v1.json`：#296
  `LF-ROAD-EDITING-P100-v1` 的机器可读定义。

#308 研究执行器与 R0 巨大 Evidence 不在当前工作区。G4 精确证据提交为
[`de4cd460a96415cafbd811141568b81f74d73534`](https://github.com/illusion-tech/laneflow/tree/de4cd460a96415cafbd811141568b81f74d73534)，
交付 PR 为 [#310](https://github.com/illusion-tech/laneflow/pull/310)。
