# 宿主审计测试支持

本包承载 #960 的完成性优先比较判据，供 Runtime 集成测试与 #963 城市浸泡测试共用。
它不是生产包、公共 API 或已接受的数据格式；只通过 `dev-dependencies` 接入调用方，
不持有世界、日志或检查点，也不改变 Runtime API。

本包作为 workspace 成员继承许可证与 lints，使源码、依赖与全部编译入口继续由
`check-wire-audit`、`Dependency policy` 和工作区测试审计。没有第三方依赖。
比较逻辑来自原 `crates/laneflow-runtime/tests/support/audit_replay.rs`，迁移不改变判据；
完成性、序数、分歧与独立进程回归仍在原 Runtime 集成测试中执行：

```text
cargo +1.99.1 test --locked -p laneflow-runtime --test audit_replay
cargo +1.99.1 test --locked -p laneflow-urban-harness --lib integrity_soak::short_chain_replays_in_an_independent_process
cargo +1.99.1 run --locked -p xtask -- check-wire-audit
cargo deny --locked --all-features check advisories bans licenses sources
```
