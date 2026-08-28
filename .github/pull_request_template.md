# PR 检查清单

## 范围

- 关联 Issue：Closes #
- 切片类型：
  - [ ] docs-only（仅文档）
  - [ ] governance（治理）
  - [ ] core-runtime（Traffic Runtime 运行时）
  - [ ] data-spec（数据格式）
  - [ ] adapter（引擎适配）
  - [ ] authoring-tool（编辑工具）
  - [ ] example（示例）
  - [ ] cross-layer（跨层高风险）
- 本次变更：
- 本次明确不做：

## 影响

- Traffic Runtime API：`无` / 说明：
- 数据格式：`无` / 说明：
- Adapter API：`无` / 说明：
- 依赖 / 许可证：`无` / 说明：
- 破坏性变更：`否` / `是`，说明：

<!-- 未触及数值域则删除本节。
## 数值权威

- 域：标量（`f32` / `f64` / 确切整数宽度 / 字节数组），范围与转换边界：，依据：
-->

## 设计依据

- 相关文档 / ADR：
- 是否需要新增 ADR 或更新 design？`否` / `是`，说明：

## 验证

列出实际运行的命令或手工检查。未运行的相关检查请说明原因。

- 构建 / 测试：
- 文档：
- 其他：

## 完成

- [ ] commit 使用 Conventional 标题、`Refs: #<issue>`；破坏性变更同时有标题 `!` 和 `BREAKING CHANGE:`
- [ ] 未解决的 review conversation 已处理（由 GitHub 原生规则拦截）
- [ ] 合入走 Merge Queue，不使用 `--admin`
