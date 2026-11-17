+++
schema_version = 1
id = "docs-reference-index"
kind = "index"
status = "active"
revised_date = "2026-11-17"
summary = "提供术语、能力、开发约定与文档规范的索引。"
+++

# 参考资料

本目录保存可复用的辅助规范与仍约束实现的机器可读输入，不替代高层决策和具体设计。
里程碑审阅、逐任务验证流水账与原始测量留在 Git 历史及 Issue/PR。

## 机器可读输入

- [道路编辑来源语义种子][semantic-seed]仅供测试和研究生成器读取，不是生产 JSON 入口。
- [校准参考机][reference-machine]说明道路编辑来源的参考条件。

三组 v0.10 历史测量基线已退役，有效聚合结论与限制见编译器基础及运行时状态完整性
设计。历史原件见[固定提交][retired-baselines]；#308 研究执行器的当时证据见
[固定证据提交][calibration-tree]与[交付 PR][pull-310]。

## 规范文档

[semantic-seed]: road-editing-source-semantic-seed-v1.json
[reference-machine]: road-editing-source-reference-machine-v1.json
[retired-baselines]: https://github.com/illusion-tech/laneflow/tree/3f80b65f3fee19cd6085fc0d8be8f58b7a14d551/docs/reference
[calibration-tree]: https://github.com/illusion-tech/laneflow/tree/de4cd460a96415cafbd811141568b81f74d73534
[pull-310]: https://github.com/illusion-tech/laneflow/pull/310

<!-- generated-doc-index:start -->

- [当前能力与限制][doc-reference-capabilities] — 持续维护；无生效取代。提供当前实现能力、条件与限制的阅读入口。
- [提交规范][doc-reference-commit-convention] — 持续维护；无生效取代。规定提交标题、关联议题和工具归属格式。
- [文档元数据与发布模型][doc-reference-document-metadata] — 持续维护；无生效取代。定义正式文档的 TOML 元数据、有效性关系和发布数据模型。
- [离线文档结构检查][doc-reference-documentation-checks] — 持续维护；无生效取代。定义文档结构、元数据、关系与索引的离线检查。
- [LaneFlow 双语术语表][doc-reference-glossary] — 持续维护；无生效取代。统一中文规范术语、英文辅助名与废弃旧称。
- [mcpls 可选开发工具][doc-reference-mcpls] — 持续维护；无生效取代。说明可选 Rust 语义开发工具及其信任边界。
- [Rust 代码风格][doc-reference-rust-code-style] — 持续维护；无生效取代。规定 Rust 可读性、注释与契约文档约定。
- [验证矩阵][doc-reference-validation-matrix] — 持续维护；无生效取代。规定各任务类别的最低验证要求。

[doc-reference-capabilities]: capabilities.md
[doc-reference-commit-convention]: commit-convention.md
[doc-reference-document-metadata]: document-metadata.md
[doc-reference-documentation-checks]: documentation-checks.md
[doc-reference-glossary]: glossary.md
[doc-reference-mcpls]: mcpls.md
[doc-reference-rust-code-style]: rust-code-style.md
[doc-reference-validation-matrix]: validation-matrix.md

<!-- generated-doc-index:end -->
