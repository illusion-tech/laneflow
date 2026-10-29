# 安全输入模糊测试

本包是独立、不可发布的验证 harness，不属于产品 API 或已接受数据格式。
它与根 workspace 隔离，使 libFuzzer/AddressSanitizer 不进入生产依赖图；依赖仍由
`Dependency policy` 对 `fuzz/Cargo.toml` 单独审计，Dependabot 维护其锁文件。

## 目标与实际覆盖

| 目标                       | 输入与断言                                                                                                                      |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `lfre_arbitrary_bytes`     | 任意 LFRE 字节进入受检准入，包含结构/计数/长度拒绝                                                                              |
| `lfre_writer_differential` | writer 合法种子及有界变异；未变异来源必须成功准入                                                                               |
| `portable_arbitrary_bytes` | LFCA/LFSM/LFSD/LFCP framing、registry 与字段值域；成功视图逐字段安全读取，LFCA 继续修订绑定                                     |
| `portable_bundle_mutation` | 两个合法 LFCA/LFSM/LFSD bundle 与 LFCP 变异；保留摘要和重算 chunk 摘要两种路径，检查跨对象与 base 绑定                          |
| `lfrs_restore`             | LFRS 7 任意字节和 Published/Editable 种子变异；来源错配、容量拒绝；成功恢复重新编码/恢复并比较逻辑摘要                          |
| `cutover_inputs`           | 有车辆世界的同修订切换、跨修订 LFSD/描述符变异、追赶；失败比较完整快照、根指针、世代/游标并验证可重新准备事务；合法种子必须提交 |

`seed_corpus` 通过生产 writer/受检读取/恢复构造种子；失效种子触发断言，不能
静默变成拒绝语料。`portable::tests::repaired_mutation_reaches_value_validation`
证明重算摘要后的变异可越过 chunk 和 registry 检查并到达值域拒绝。

结构变异不是任意脚本：最多 32 次操作，offset 来自有界整数，重算 chunk 时只使用
受检种子的范围，不信任变异后的 offset/count。这限制 harness 自身的分配，生产
入口的实际限制仍由生产类型拥有。随机探索不代替既有定向拒绝测试和状态迁移测试。

## 运行与回执

权威运行平台为 GitHub `Fuzz` workflow 的 Ubuntu runner，固定 `cargo-fuzz 0.13.2`。
PR/队列 smoke 每目标 300 秒；手工 `duration_seconds=1800` 是 #536 的正式有界窗口。
`run_campaign` 固定单输入 25 秒、RSS 2048 MiB、输入 262144 bytes、seed 53601，
使用 cargo-fuzz 默认 AddressSanitizer。nightly 的完整 rustc/cargo 身份进入各回执。

在仓库根目录运行（正式窗口通过 `FUZZ_MAX_TOTAL_TIME=1800` 选择）：

```text
cargo +nightly test --manifest-path fuzz/Cargo.toml --locked --lib
cargo +nightly clippy --manifest-path fuzz/Cargo.toml --all-targets --locked -- -D warnings
cargo +nightly run --manifest-path fuzz/Cargo.toml --locked --example seed_corpus -- fuzz/corpus
cargo +nightly run --manifest-path fuzz/Cargo.toml --locked --example run_campaign -- cutover_inputs
```

`fuzz/receipts/<target>/receipt.json` 记录精确源码 SHA、工具链、命令、实际进程时间、
输入初末库存、日志与二进制摘要、锁文件是否变化，以及 libFuzzer 的执行量/coverage
原文。`process_elapsed_seconds` 包含 cargo 构建时间；fuzz 实际时长以日志
`Done ... runs in ... second(s)` 为准。`complete=true` 需要子进程成功、终态统计
存在且锁文件未漂移。crash/timeout/OOM/断言失败不改记为成功，目录保留首次失败。
同目录重跑被拒绝；新的运行应使用干净 checkout 或另行归档旧目录。

工作流无论成功失败都上传语料、crash、完整日志和回执，GitHub retention 为 90 天。
正式验收须另外保存下载制品及清单，记录本地位置、大小、SHA-256；仅剩过期 URL
不能宣称原始语料长期可用。语料、日志、二进制、锁文件采用原始文件字节 SHA-256，
外部重算方是验收时的 PowerShell `Get-FileHash` 与 GitHub 下载制品的通用工具；
这是 ADR 0033 的互操作摘要，持久化在 `laneflow-security-fuzz-1` 回执中。
LFCA/LFSM/LFSD chunk 继续采用其既有 wire 字节合同，快照逻辑对照沿用
`deterministic_state_digest`，不以文件摘要替代状态比较。

依赖审计：

```text
cargo deny --manifest-path fuzz/Cargo.toml --locked check advisories bans licenses sources
```

## 验收边界

这是特定提交和有界语料的验证，不能证明任意输入下绝无缺陷。当前随机恢复场景为
Published 的小型有车路网与 Editable 的空路网来源；迁移为 Published 来源、一个
有车跨修订夹具及同修订场景。复杂 Parking/Waiting/Conflict 迁移、Editable 跨修订
组合继续由 Runtime 定向回归覆盖，未宣称已在本 fuzz corpus 穷举。
Windows 仅承担非 sanitizer 的种子回归；不把其结果标注为正式 Linux fuzz 运行。

#536 的候选提交 receipt、unsafe 审计和仓库扫描通过后，只构成 #305 的安全轴输入。
#305 汇总时仍需对齐最终验收提交，不能外推到之后的代码改动。
