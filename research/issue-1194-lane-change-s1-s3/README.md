# #237 最小换道 S1/S3 切片实现（Issue #1194）

**状态**: 实现中（实现方案已获统筹审阅通过；等值/发射验证随实现执行）
**基线**: `origin/main` @ `186e1fd5`（含已冻结合同 `lane-change-minimal-contract.md`）
**范围**: S1 过渡感知扩展 + S3 材料化/静态发射 + 能力拒绝 + A0/A1 硬等值 +
合同状态回写（Review → Accepted）

## 交付物

| 交付物          | 位置                                                                                | 内容                                                                                                                                                                                                                                                        |
| --------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 实现方案        | [plan.md](plan.md)                                                                  | S1/S3/能力拒绝设计、A0/A1 等值计划、发射实测四口径、R1–R3 引用不实现                                                                                                                                                                                        |
| S1 过渡感知扩展 | `tools/laneflow-lust-converter/src/convert/routes.rs`                               | `TransitionDeclaration`/`TransitionIndex`（校验失败关闭）；两相展开（相 1 严格=逐字节旧行为，相 2 声明转移仅严格失败时启用）；物化插入过渡边链 + 目标 lane 段；`build_routes_and_bind_population_with_transitions` 新入口（旧入口委托空声明，调用点零改动） |
| 能力拒绝        | `crates/laneflow-runtime/src/kernel/{error.rs,tables.rs}`                           | `RouteError::TransitionUnsupported`（结构化/失败关闭/注册表不变）；`compile_route` 过渡段派生规则（uncovered 非 junction 边 + 前后邻边同走廊不同 lane 链）；能力谓词 `transition_execution_supported`（实现能力，非配置）                                   |
| 契约回写        | `docs/design/lane-change-minimal-contract.md`、`docs/design/real-road-workloads.md` | 合同状态 → **Accepted**（#237 G1 冻结，2026-10-31）；§3.1 边内换道语义修订（仅声明边界转移、次选语义、旧输入零新拒绝）                                                                                                                                      |

## 裁定护栏（2026-10-31 统筹四点 + 硬护栏）

1. 派生规则不新增静态实体；`TransitionUnsupported` 属 A6 授权拒绝面；
   **硬护栏：仅声明输入可触达**——空声明/旧输入路径任何情形不产生该错误，
   A0/A1 等值以「旧输入零新拒绝」为断言。
2. 选择语义：声明转移仅在现行 lexicographic-min 失败时启用（两相展开）。
3. 夹具载体：converter 合成夹具（v5 派生）；LuST 全量发射归 S6。
4. 可选参数属 converter 内部 API；`routes.toml` 与数据规格不动。
5. routes.rs 严格规则放宽必须声明门控（空声明=原规则逐字节）；S3/S4 验收
   边界按 PR #1188 条目（S3 只验注册面，准入/恢复复核留 S4）。

### 实现差异记录（统筹 2026-10-31 复核接受）

- **裁定③夹具载体**：原字面「converter 合成夹具」→ 实现为**合成编制夹具**
  （`laneflow-compiler` `SyntheticModuleBuilder`，即 SyntheticDsl 合成来源
  原生声明）。理由：横断面叠层（B-up/TZ/B-down 双 lane 链）与 uncovered
  过渡段只有合成编制输入可表达（converter 发射为单 lane section/边，无法
  承载多 lane 走廊与过渡段身份）；裁定③的样本结构（v5 派生 B-up/TZ/B-down
  + T01）完整保留；LuST 全量发射仍归 S6。

## 等值与验证（结果如实记录，失败不修绿）

- **A0/A1 硬等值基线**（S1 核心落码后）：converter **318 通过 / 0 失败 / 49
  ignored**（含 A0 五组 + A1 十组）。
- **终态全量**：converter **323 通过 / 0 失败**（318 基线 + S1 新增五测试，
  既有测试文件零改动、旧入口零签名变更）；runtime **641 通过 / 0 失败**
  （639 基线 + S3/能力拒绝两测试）——**旧输入零新拒绝**（硬护栏）实证。
- **S3 发射预算实测**：合成样本网络（v5 派生 13 边）经 SyntheticModuleBuilder
  → Compiler → `emit_portable_candidate` → `check_post_emission_bundle`
  （`FormatLimits::HARD`，**零放宽**）→ `build_shared_network_revision` 全链
  通过；逐边长度多重集与 sample v5 钉版值逐项相等。
- **静态序列验证 + 能力拒绝**：直行链编译通过（能力门放行）；过渡链
  `[A0,I1,B0_up,T01,B1_down,I2,C0]` 返回 `TransitionUnsupported`（错误类型即
  证明静态序列合同完整通过）；拒绝纯度/可重复、注册失败原子（D5）、无过渡
  结构路线永不触发（护栏断言）。
- **常规检查**：`clippy -D warnings`（两 crate 全 targets）、`fmt --check`、
  `git diff --check` 全部通过。

## 验收对照（Issue #1194）

- [x] A0/A1 硬等值三项口径全绿（旧入口旧结论逐值不变 + 旧输入零新拒绝）。
- [x] S1 静态序列验证 + 能力拒绝断言（S3 侧注册面；准入/恢复留 S4）。
- [x] S3 发射预算实测四口径 + 失败留档纪律（合成样本全链 HARD 零放宽；
  逐边证据 = 长度多重集断言；无失败需留档）。
- [x] 合同状态转 Accepted；§3.1 契约同步。
- [ ] 常规检查与九项必需检查随 PR（本地 clippy/fmt/diff 已全绿）。

## 未执行 / 不做（引用验收形态不实现）

- **R1a/R1b/R2/R3 运行时验收（S4）、R3 持久化交叉（S5）**：引用
  `lane-change-minimal-contract.md` §7 与 sample v5 验收形态，本切片不实现。
- 不放宽生产路线行为；不改已冻结合同语义；等值失败如实报不修绿；发射失败
  逐字留档、不修绿/不提预算/不改钉版几何。
