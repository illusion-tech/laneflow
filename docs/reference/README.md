# 参考资料

本目录用于保存 LaneFlow 的长期参考资料、模板、术语和通用约定。

`docs/reference/` 不承载高层决策，也不替代设计文档。它用于沉淀可复用的辅助规范。

## 适合放在这里的内容

- 术语表
- 状态字典
- 命名约定
- Issue 和 PR 模板说明
- 校验矩阵
- 数据格式版本约定
- 示例场景验收清单

## 建议后续补充

- `glossary.md`
- `data-versioning.md`
- `adapter-compatibility-matrix.md`
- `example-scenario-checklist.md`

## 当前文档

- `commit-convention.md`：提交信息规范，以 Conventional Commits 标题为基础，用 `Gate`、`Slice`、`Impact`、`Validation` 等字段记录 LaneFlow 治理状态；并说明 PR 默认使用 Rebase and merge。
- `validation-matrix.md`：切片类型到最小验证要求的矩阵，用于 `G3` 合并闸口判断。
- `v0.2-closure-review.md`：v0.2 Lane Graph + Route 收口时核验的契约、验证证据、发现项处置和非阻断风险基线。
