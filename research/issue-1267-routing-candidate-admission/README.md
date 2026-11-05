# #1267 Routing 候选准入正式负载集成验收证据（结论）

2026-11-05 完成。工作负载族 `LF-ROUTING-CANDIDATE-LFSYNTH-v1`，三身份
（`10000-W2-16-a`、`100000-W2-33-a`、`10000-L1-16-a`，维护者裁定）。
实现提交 `b290039aca195b09fabc0ae5b236b07ef6ee378f`（运行时工作树与提交
逐字一致）。正式登记与冻结表在
`docs/design/traffic-observation-and-routing-integration.md` §7.2；
本文件给结论、复现命令与身份摘要。

## 主要结果

- **三路径集成证据三身份全部 `complete=true`**：成功探针 27/27/30；
  拒绝探针各 13，逐条精确归类（空序列/模型失配/跨世界绑定/未知标识/
  错误实体种类/不连通/stale/路线槽与出现记录的 max+1/接收端四类）且
  断言零路线槽、零边出现记录、零观测状态序号变化；不改世界状态的
  探针前后世界快照摘要相等。检查点（含已注册长候选）→ `restore_lfrs`
  → 宿主后缀（`register_admitted_route` + spawn + 三拍 + 重开观测准入
  并注册候选）两侧 6 个对拍点 (command_cursor, tick, state digest)
  逐点相等、零分歧。
- **W2 两档规模稳态 tick 零分配**（一万/十万全 active，各 32 采样拍：
  0 allocation / 0 reallocation）。
- **L1 换道窗口内稳态拍携带分配**：37 alloc / 3 realloc、live
  +83848 B；窗口过后（steady after）归零。按描述性观察登记，不构成
  预算判定；候选优化不在本单范围。
- **候选注册账本与规模无关、随边数扩展**：1 边 adjusted live 108 B、
  2 边 276–284 B 三身份一致；L1 5 边长候选 884 B。
- **观测计量精确可解释**：full 的 batch logical = 192 + 40×边数；
  session logical = 28×边数 + batch logical；delta-zero batch logical
  恒 192 B 且 net 0；成本 payload = 24×边数；receiver 预检后零分配。
- **墙钟与 tick 间隔干扰：未运行**（负载敏感类，#1279 在冻结验收提交
  后安静窗口运行；入口已交付含干扰配对，#1279 零改码）。§7 计量义务
  待 #1279 完成才算完整。

## 证明边界

- 输入族拓扑为无环短链（W 每 cell 独立 2 边链、L1 簇路线 5 边）：
  **重复边在合法路线中不可构造**（路线编译器要求相邻连通或机动转移
  连通，无反向边/环），该 §7 矩阵义务由 `facade::routing` 合同测试在
  可控拓扑覆盖；**长边序列轴在正式输入上到顶为 5 边**，4096 边长轴
  扩展性由 §7.1 的 `LF-ROUTING-G2-LINEAR-v1` 承担。
- W 族上 1 边路线取机动完成末端边：以信号门入口边开头的单边路线按
  `ManeuverMismatch` 拒绝，属机动合同语义而非缺陷。
- 私有字段伪造类失败（绑定版本、会话修订、未来 tick/状态序号、非法
  时效窗）真实宿主构造不出，属合同测试面，未在集成面重复。
- 本机观察：G2 证据入口（`routing_g2_*_evidence`）的 fixture 构建在
  main 上已因编译器 StageScratchBytes 限额增长而失败（
  `CompileLimitExceeded`，`#[ignore]` 使 CI 不可见）——存量问题，
  与本单变更无关，待单独缺陷单处理。

## 复现

```text
# 输入：LF-SYNTH-v2 正式身份（外部缓存，锚定 8b05abce）
#   <input-root>/{10000-W2-16-a,100000-W2-33-a,10000-L1-16-a}
rustc 1.99.1 (58cf8f9c3)；release；单测试线程；本机 P100 参考机

# 三路径集成证据（每身份一个进程）
LANEFLOW_LF_SYNTH_INPUT_ROOT=<input-root> \
LANEFLOW_ROUTING_EVIDENCE_IDENTITY=<身份> \
cargo +1.99.1 test --release -p laneflow-runtime \
  --test routing_candidate_integration_evidence -- --ignored --nocapture

# 分配/retained 账本（插桩进程，与墙钟分进程；须过滤测试名——
# 同二进制里的 G2 入口在 main 上另有 #1288 的存量限额损坏）
… --test routing_budget_evidence -- --ignored --nocapture   routing_candidate_lf_synth_budget_evidence

# 墙钟 + tick 干扰入口（未插桩；正式 3-round 协议由 #1279 编排）
… --test routing_wall_clock_evidence -- --ignored --nocapture
  routing_candidate_lf_synth_wall_clock_evidence
```

## 身份摘要（实测输出行存档于本目录 `evidence/`）

| 身份 | manifest digest | network revision | workload manifest | selection | observation set | cost snapshot |
| --- | --- | --- | --- | --- | --- | --- |
| 10000-W2-16-a | 1d41fcfbf785…cf912f | 7d6f081c81d4…fdacf32 | 96069d86c545…5666e7 | 310b05d4bbd5…87c8702 | c4048b7edc3e…e8ff621 | 453092538367…3c828ff1 |
| 100000-W2-33-a | 0ddfa03e7996…327488 | a229712483b3…50d0ac68 | 5cd132dedfcd…3320f7bf | b9bf0a82c56c…5ec1d95 | ab07948592c7…60b5554c | 814d52e8f03d…9593454 |
| 10000-L1-16-a | 488e438d33ae…972998f | da3d57032ca7…c3d1925f | 62d4af7b9054…903884f | 89b24d366c13…c203b3 | c3648f298766…6aac250c | 606b8eeb12d0…2e801bb |

关联：#539（routing 验收格）、#305（消费）、#1279（墙钟与 tick 干扰，
正式）、#1282（冻结验收提交）。入口代码：`crates/laneflow-runtime/tests/`
（`routing_candidate_integration_evidence.rs`、`routing_wall_clock_evidence.rs`
的 LF-SYNTH 模式、`routing_budget_evidence.rs` 的 LF-SYNTH 模式、
`tests/support/routing_evidence.rs`）。
