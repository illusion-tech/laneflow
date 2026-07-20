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
- `rust-code-style.md`：补充 `rustfmt` 无法表达的 Rust 仓库级可读性约定，当前重点规定数字字面量分组边界与例外。
- `validation-matrix.md`：切片类型到最小验证要求的矩阵，用于 `G3` 合并闸口判断。
- `v0.2-closure-review.md`：v0.2 Lane Graph + Route 收口时核验的契约、验证证据、发现项处置和非阻断风险基线。
- `v0.3-closure-review.md`：v0.3 Vehicle Following 收口时核验的设计、实现、数据契约、性能、安全与治理基线。
- `v0.3-vehicle-following-validation.md`：v0.3 Vehicle Following 的确定性、不变量、生命周期、10k 性能和 100k 扩展性验证基线。
- `v0.4-signals-validation.md`：v0.4 Signals 的 loader-to-Core、确定性、SignalStop、10k matched workload 与 100k 扩展性验证基线。
- `v0.4-closure-review.md`：v0.4 Signals 收口时核验的设计、实现、current 0.4 数据契约、性能、安全、治理与剩余风险基线。
- `v0.5-lifecycle-substrate-validation.md`：#106 lifecycle、overflow-safe route distance、command-spatial、allocation/retained-memory 与同机 base/candidate 性能验证基线。
- `v0.5-static-parking-validation.md`：#107 static Parking、current 0.5 schema/loader/fixtures、foreign-graph rebind、10k all-vacant 0-allocation 与同机 matched 性能验证基线。
- `v0.5-parking-runtime-validation.md`：#108 Parking authority/snapshot、同步 commands、Parked/despawn lifecycle、transitional guard、local-query oracle、10k/100k、allocation 与同机 matched 性能验证基线。
- `v0.5-parking-activation-validation.md`：#109 ParkingStop/arrival、unified reducer、parking-aware traversal、route-completion release、事件总序、Reserved ratio、allocation 与 matched step 性能证据。
- `v0.5-parking-validation.md`：#110 schema/loader/Core 端到端示例、D11 组合矩阵、10k/100k、allocation/retained-memory、pathological scaling 修复与 CPU profile 验证基线。
- `v0.5-closure-review.md`：v0.5 Parking 收口时核验的治理、设计、实现、current 0.5 数据契约、性能、安全、发现项与剩余边界基线。
- `v0.5-lifecycle-substrate-validation.json`：#106 验证基线的 machine-readable 原始 round、倍率、环境与依赖审计摘要。
- `v0.6-numeric-validation.md`：#122、#140/#141、#125–#127 与 #144 的数值盘点、误差、产品范围、路线布局、内存、性能和 no-go 生产裁决基线。
- `v0.6-numeric-performance-evidence.json`：#127 多轮数值候选性能、无效污染轮次、来源提交与配对摘要的机器可读证据。
- `v0.6-numeric-production-migration-evidence.json`：#144 完整生产候选、五轮 14 项稳态矩阵、来源节点与 no-go 裁决的机器可读证据。
- `v0.6-numeric-closure-review.md`：v0.6 数值切片收口时核验的治理、当前生产事实、目标/当前分离、机器证据、性能裁决与剩余边界基线。
- `v0.6-spatial-validation.md`：#123 Spatial 设计研究及 ADR 0015 修订后的有界 canonical `f32` 验证基线；不替代 #138 独立收口。
- `v0.6-spatial-performance-evidence.json`：#137 production `f32` 对同构 `f64` oracle、10k/100k p95、零分配、retained memory、Criterion 与 lookup/sampling 分解的机器可读证据。
- `v0.6-spatial-closure-review.md`：v0.6 Spatial 切片收口时核验的治理、权威分层、数据制品、生产实现、正确性、资源、性能、安全、发布与剩余边界基线。
- `v0.6-closure-review.md`：v0.6 Numeric & Spatial Foundation 整体收口时汇总的双切片治理、最终生产契约、性能裁决、安全发布状态与 v0.7 进入边界基线。
