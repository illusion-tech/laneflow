# #237 最小换道 S1/S3 切片实现（Issue #1194）

**状态**: 实现中（实现方案已获统筹审阅通过；等值/发射验证随实现执行）
**基线**: `origin/main` @ `186e1fd5`（含已冻结合同 `lane-change-minimal-contract.md`）
**范围**: S1 过渡感知扩展 + S3 材料化/静态发射 + 能力拒绝 + A0/A1 硬等值 +
合同状态回写（Review → Accepted）

## 交付物

| 交付物 | 位置 | 内容 |
| --- | --- | --- |
| 实现方案 | [plan.md](plan.md) | S1/S3/能力拒绝设计、A0/A1 等值计划、发射实测四口径、R1–R3 引用不实现 |
| S1 过渡感知扩展 | `tools/laneflow-lust-converter/src/convert/routes.rs` | `TransitionDeclaration`/`TransitionIndex`（校验失败关闭）；两相展开（相 1 严格=逐字节旧行为，相 2 声明转移仅严格失败时启用）；物化插入过渡边链 + 目标 lane 段；`build_routes_and_bind_population_with_transitions` 新入口（旧入口委托空声明，调用点零改动） |
| 能力拒绝 | `crates/laneflow-runtime/src/kernel/{error.rs,tables.rs}` | `RouteError::TransitionUnsupported`（结构化/失败关闭/注册表不变）；`compile_route` 过渡段派生规则（uncovered 非 junction 边 + 前后邻边同走廊不同 lane 链）；能力谓词 `transition_execution_supported`（实现能力，非配置） |
| 契约回写 | `docs/design/lane-change-minimal-contract.md`、`docs/design/real-road-workloads.md` | 合同状态 → **Accepted**（#237 G1 冻结，2026-10-31）；§3.1 边内换道语义修订（仅声明边界转移、次选语义、旧输入零新拒绝） |

## 裁定护栏（2026-10-31 统筹四点 + 硬护栏）

1. 派生规则不新增静态实体；`TransitionUnsupported` 属 A6 授权拒绝面；
   **硬护栏：仅声明输入可触达**——空声明/旧输入路径任何情形不产生该错误，
   A0/A1 等值以「旧输入零新拒绝」为断言。
2. 选择语义：声明转移仅在现行 lexicographic-min 失败时启用（两相展开）。
3. 夹具载体：converter 合成夹具（v5 派生）；LuST 全量发射归 S6。
4. 可选参数属 converter 内部 API；`routes.toml` 与数据规格不动。
5. routes.rs 严格规则放宽必须声明门控（空声明=原规则逐字节）；S3/S4 验收
   边界按 PR #1188 条目（S3 只验注册面，准入/恢复复核留 S4）。

## 等值与验证（结果如实记录，失败不修绿）

- **A0/A1 硬等值基线**（S1 核心落码后、全部改动前）：converter 包级
  **318 通过 / 0 失败 / 49 ignored**（含 A0 五组 + A1 十组）。
- 终态复验、S1 新增五测试、S3 发射实测、能力拒绝断言：随实现执行后在此更新。

## 验收对照（Issue #1194）

- [ ] A0/A1 硬等值三项口径全绿（旧入口旧结论逐值不变 + 旧输入零新拒绝）。
- [ ] S1 静态序列验证 + 能力拒绝断言（S3 侧注册面；准入/恢复留 S4）。
- [ ] S3 发射预算实测四口径 + 失败留档纪律。
- [x] 合同状态转 Accepted；§3.1 契约同步。
- [ ] 常规检查与九项必需检查随 PR。

## 未执行 / 不做（引用验收形态不实现）

- **R1a/R1b/R2/R3 运行时验收（S4）、R3 持久化交叉（S5）**：引用
  `lane-change-minimal-contract.md` §7 与 sample v5 验收形态，本切片不实现。
- 不放宽生产路线行为；不改已冻结合同语义；等值失败如实报不修绿；发射失败
  逐字留档、不修绿/不提预算/不改钉版几何。
