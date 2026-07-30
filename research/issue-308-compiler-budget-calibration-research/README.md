# #308 编译器预算校准非生产研究

本目录只实现 #308 已通过 G1/G2 的编译器资源上限与性能预算校准研究。它是 Rust
工作区成员，以便接受统一格式、测试、Clippy、文档和依赖审计；它不是生产编译器、
公共应用程序接口（API）、产品性能承诺或真实城市规模模型。

权威研究契约为：

- `docs/design/compiler-budget-calibration.md`；
- `docs/reference/compiler-calibration-contract-v1.json`；
- `docs/reference/compiler-calibration-workloads-v1.json`；
- `docs/reference/compiler-calibration-evidence-v1.schema.json`。

研究执行器必须先以受信任来源提交外部登记的精确字节长度与 SHA-256 校验契约描述符，
再按描述符校验工作负载清单和证据 Schema；校验完成前不得解析证据或开始测量。

## 依赖边界

- `publish = false`，`default = []`；
- 生产 crate 不得依赖本包；
- `laneflow-data` 与 `laneflow-spatial` 只由 `fixture-oracle` 在计时区外读取当前夹具；
- 每种第三方候选拥有独立私有特性（feature）；
- `research-runner-full` 是正式研究执行器的封闭总特性（feature），其成员已在 #308 G2
  依赖审计中冻结；
- `xxhash-rust 0.8.18` 仅服务 XXH3/XXH64 研究候选。其 BSL-1.0 许可证通过
  `deny.toml` 中绑定精确版本的 #308 例外接受，不得扩散到生产依赖图；
- 具体依赖版本、来源与校验和由 `Cargo.lock` 绑定，正式证据必须记录同一锁文件的
  SHA-256。

G2 冻结的候选依赖如下；“特性”只列本研究包直接启用的上游特性（feature）：

| 候选依赖      | 精确版本 | 特性            | crates.io 校验和                                                   | 许可证              |
| ------------- | -------- | --------------- | ------------------------------------------------------------------ | ------------------- |
| `hashbrown`   | 0.17.1   | `inline-more`   | `ed5909b6e89a2db4456e54cd5f673791d7eca6732202bbf2a9cc504fe2f9b84a` | MIT OR Apache-2.0   |
| `indexmap`    | 2.14.0   | `std`           | `d466e9454f08e4a911e14806c24e16fba1b4c121d1ea474396f396069cf949d9` | Apache-2.0 OR MIT   |
| `xxhash-rust` | 0.8.18   | `xxh3`、`xxh64` | `aee1b19627c7c60102ab80d3a9cbe18de90bfe03bfa6c3715447681f0e8c8af6` | BSL-1.0（精确例外） |

## 当前实现切片

当前切片只建立受信任契约引导（trusted contract bootstrap）和命令行
`verify-contract` 冒烟入口。代表性编译管线、精确研究预言机、测量隔离、证据写出器
和正式 `run --protocol compiler-calibration-v1` 入口仍须按设计文档继续实现；在完整
正确性与 pilot 证据落盘前，不得宣称任何正式预算数字。

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  verify-contract
```
