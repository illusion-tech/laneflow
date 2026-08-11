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

- `data-versioning.md`
- `adapter-compatibility-matrix.md`
- `example-scenario-checklist.md`

## 当前文档

- `glossary.md`：LaneFlow 双语术语 SSOT；中文术语和中文定义是权威事实，英文只作
  辅助理解，当前完整覆盖 #291 编译器时代静态路网、Identity、制品/镜像、信任、
  Traffic Runtime、城市模拟游戏上层、执行规划、路网修订、快照/回放、Routing、
  Spatial 与 Adapter 术语。
- `compiler-calibration-workloads-v1.json`：#308 已完成研究的机器可读工作负载清单；
  冻结模块图、字符串/来源位置、identity 字段绑定、研究记录布局、精确计数、夹具
  摘要、失败变体与候选注册表，不是生产编译器 API 或产品容量声明。
- `compiler-calibration-evidence-v1.schema.json`：#308 已完成研究的证据 JSON Schema；
  冻结原始执行制品绑定、来源、环境、覆盖计数和紧凑派生结果封套；逐次运行只保存在
  被绑定的原始 JSON，Rust 验证器从原始事实独立重算整份 Evidence。
- `compiler-calibration-contract-v1.json`：#308 已完成研究的非自指契约描述符；从
  Evidence v1 Schema 外部绑定证据 Schema 与工作负载清单的路径、版本、长度和
  SHA-256，正式验证必须先校验该描述符。
- `road-editing-source-semantic-seed-v1.json`：#296 的不可变 benchmark 语义种子；只供
  test/research generator 读取，不是 production source format，也不形成 JSON 兼容承诺。
- `road-editing-source-reference-machine-v1.json`：#296 道路编辑来源校准参考机声明；以
  hardwareId 与硬件身份 SHA-256 绑定正式校准运行的物理机器，其余字段为声明时快照。
- `road-editing-source-workload-definition-v1.json`：#296 纠偏后
  `LF-ROAD-EDITING-P100-v1` 的 G1 机器可读定义；以带摘要的旧研究 fixture 仅作 test-only
  语义种子，冻结五模块/1,715 稳定声明、35 条 alignment 与 160 条 junction-internal
  curve、完整映射/optional/string/width 规则、九档组合、单模块候选替换生命周期和精确
  测量协议。它不是 production JSON 兼容层或测量结果；writer/fixture digest 与 exact
  commit 只由后继 G2 evidence 填写。
- `v0.10-compiler-pilot-budget.md`：#308 在 R0 研究机上得到的九个基础规模冷实例临时
  性能预算、来源、计算规则、正确性核对与禁止外推边界；只用于早期基础规模发现与
  正式校准候选输入，不是 #292 G1 冻结的首轮实现预算。
- `v0.10-compiler-pilot-budget.json`：上述临时预算的机器可读精确整数、七样本原始值、
  运行/预言机引用、语义摘要与来源检查点绑定；不是 Compiler Calibration Evidence v1。
- `v0.10-compiler-budget-calibration-raw.json`：#308 R0 两批正式执行的逐次运行权威事实，
  包含有效、无效、失败与受护栏记录；紧凑 Evidence 通过精确长度和 SHA-256 绑定它。
- `v0.10-compiler-budget-calibration-environment.json`：上述 R0 的操作者环境声明。
- `v0.10-compiler-budget-calibration-evidence.json`：从绑定原始执行结果独立重算的紧凑
  Evidence v1，包含基础规模、正式阶梯分析、重复性包络、预算与候选分类。
- `v0.10-compiler-budget-calibration-report.md`：上述 R0 的中文权威结论、预算摘要、候选
  判断、环境边界、P100 同机硬件角色更新与精确复现命令；硬件角色更新不改变原始证据
  或追溯形成 Product Pass。
- `v0.10-compiler-foundation-validation.md`：#292 G2 生产纵向切片的实现、依赖方向、
  迁移等价、本地验证与性能门槛适用性结论；记录 G1 修订、生产基线和进入 Delivery PR
  审阅前的剩余边界。
- `v0.10-official-module-admission-validation.md`：#315 官方前端共同受检模块接入 G2
  的编译器私有共同 Typed AST、逐文档描述符、v1/v2 配置档版本边界、原子准入、来源映射、同机配对性能
  与进入 Delivery PR 前的本地验证事实。
- `v0.10-compiler-production-baseline.md`：#292 在 P100 推荐参考机型上运行真实生产
  `Compiler::compile` 的工作负载、计时边界、五级紧凑结果和使用边界。
- `v0.10-compiler-production-baseline.json`：上述生产 R0 的机器可读紧凑证据；只保存
  五级汇总，不保存逐样本 raw 或研究进程编排记录。
- `commit-convention.md`：提交信息规范，以 Conventional Commits 标题为基础，用 `Gate`、`Slice`、`Impact`、`Validation` 等字段记录 LaneFlow 治理状态；并说明 PR 默认使用 Rebase and merge。
- `rust-code-style.md`：补充 `rustfmt` 无法表达的 Rust 仓库级可读性约定，当前重点规定数字字面量分组边界与例外。
- `validation-matrix.md`：切片类型到最小验证要求的矩阵，用于 `G3` 合并闸口判断。
- `v0.2-closure-review.md`：v0.2 Lane Graph + Route 收口时核验的契约、验证证据、发现项处置和非阻断风险基线。
- `v0.3-closure-review.md`：v0.3 Vehicle Following 收口时核验的设计、实现、数据契约、性能、安全与治理基线。
- `v0.3-vehicle-following-validation.md`：v0.3 Vehicle Following 的确定性、不变量、生命周期、一万性能和十万扩展性验证基线。
- `v0.4-signals-validation.md`：v0.4 Signals 的 loader-to-Core、确定性、SignalStop、一万 matched workload 与十万扩展性验证基线。
- `v0.4-closure-review.md`：v0.4 Signals 收口时核验的设计、实现、current 0.4 数据契约、性能、安全、治理与剩余风险基线。
- `v0.5-lifecycle-substrate-validation.md`：#106 lifecycle、overflow-safe route distance、command-spatial、allocation/retained-memory 与同机 base/candidate 性能验证基线。
- `v0.5-static-parking-validation.md`：#107 static Parking、current 0.5 schema/loader/fixtures、foreign-graph rebind、一万 all-vacant 0-allocation 与同机 matched 性能验证基线。
- `v0.5-parking-runtime-validation.md`：#108 Parking authority/snapshot、同步 commands、Parked/despawn lifecycle、transitional guard、local-query oracle、一万/十万、allocation 与同机 matched 性能验证基线。
- `v0.5-parking-activation-validation.md`：#109 ParkingStop/arrival、unified reducer、parking-aware traversal、route-completion release、事件总序、Reserved ratio、allocation 与 matched step 性能证据。
- `v0.5-parking-validation.md`：#110 schema/loader/Core 端到端示例、D11 组合矩阵、一万/十万、allocation/retained-memory、pathological scaling 修复与 CPU profile 验证基线。
- `v0.5-closure-review.md`：v0.5 Parking 收口时核验的治理、设计、实现、current 0.5 数据契约、性能、安全、发现项与剩余边界基线。
- `v0.5-lifecycle-substrate-validation.json`：#106 验证基线的 machine-readable 原始 round、倍率、环境与依赖审计摘要。
- `v0.6-numeric-validation.md`：#122、#140/#141、#125–#127 与 #144 的数值盘点、误差、产品范围、路线布局、内存、性能和 no-go 生产裁决基线。
- `v0.6-numeric-performance-evidence.json`：#127 多轮数值候选性能、无效污染轮次、来源提交与配对摘要的机器可读证据。
- `v0.6-numeric-production-migration-evidence.json`：#144 完整生产候选、五轮 14 项稳态矩阵、来源节点与 no-go 裁决的机器可读证据。
- `v0.6-numeric-closure-review.md`：v0.6 数值切片收口时核验的治理、当前生产事实、目标/当前分离、机器证据、性能裁决与剩余边界基线。
- `v0.6-spatial-validation.md`：#123 Spatial 设计研究及 ADR 0015 修订后的有界 canonical `f32` 验证基线；不替代 #138 独立收口。
- `v0.6-spatial-performance-evidence.json`：#137 production `f32` 对同构 `f64` oracle、一万/十万 p95、零分配、retained memory、Criterion 与 lookup/sampling 分解的机器可读证据。
- `v0.6-spatial-closure-review.md`：v0.6 Spatial 切片收口时核验的治理、权威分层、数据制品、生产实现、正确性、资源、性能、安全、发布与剩余边界基线。
- `v0.6-closure-review.md`：v0.6 Numeric & Spatial Foundation 整体收口时汇总的双切片治理、最终生产契约、性能裁决、安全发布状态与 v0.7 进入边界基线。
- `v0.7-bevy-validation.md`：#171 campus headless E2E、分帧确定性、一万/十万零分配、固定机 p95、benchmark 与 CI 验证基线。
- `v0.7-bevy-performance-evidence.json`：#171 固定 Windows 性能机环境、source commit、逐轮 `PostUpdate` p95/median、allocation 与适用边界的机器可读证据。
- `v0.7-bevy-debug-gizmos-validation.md`：#172 可选 debug Gizmos 的 validated-batch、预算/过滤、依赖图、headless tests、MSRV、dependency policy 与本机可视 smoke 证据。
- `v0.7-bevy-native-example-validation.md`：#173 campus native reference example 的真实制品加载、feature/依赖边界、dedicated compile、运行时控制与本机窗口 smoke 证据。
- `v0.7-bevy-native-example-smoke.jpg`：#173 本机运行 `native_reference`、启用 debug Gizmos 后保存的窗口内渲染截图。
- `v0.7-bevy-closure-review.md`：#174 对 v0.7 Bevy Reference Adapter 治理、生产契约、正确性、性能、可视验证、依赖安全与兼容边界的最终独立收口基线。
- `v0.8-signalized-corridor-closure-review.md`：#195 对 v0.8 直行信号化走廊治理、制品、道路限速、双信号控制、50–200 人口、确定性回流、native 证据、安全与兼容边界的最终独立收口基线。
- `v0.9-protected-turning-native-validation.md`：#190 protected-left / straight / protected-right native example 的 targeted headless 与 Windows 本机 smoke 证据。
- `v0.9-protected-turning-native-smoke.png`：#190 本机运行 `signalized_corridor` 默认 100 vehicles / seed 0 并通过 `F12` 保存的窗口内渲染截图。
- `v0.9-cross-layer-validation.md`：#191 的 50/100/200 人口、stress seeds、回流与生命周期、chunking replay、灯具一致性、speed×signal、制品长度绑定与性能/有界性统一 cross-layer 证据。
- `v0.9-cross-layer-native-smoke-50.png` / `v0.9-cross-layer-native-smoke-100.png` / `v0.9-cross-layer-native-smoke-200.png`：#191 本机运行 50/100/200 vehicles / seed 0 并通过 `F12` 保存的窗口内渲染截图。
- `v0.9-signalized-corridor-closure-review.md`：#192 对 v0.9 受保护转向信号化走廊治理、显式 Junction/Movement/ManeuverPath 静态身份、制品、限速、双 12-phase 信号控制、50–200 人口、确定性回流、统一 cross-layer 证据、安全与兼容边界的最终独立收口基线。
