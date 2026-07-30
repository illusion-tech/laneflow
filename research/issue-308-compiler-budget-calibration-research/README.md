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

## 已实现切片

当前已建立：

- 受信任契约引导（trusted contract bootstrap）和命令行 `verify-contract` 冒烟入口；
- 对 `LF-COMP-ID-v1` 所需生成器清单子契约的类型化读取与逐字段拒绝；
- 由清单驱动的 SplitMix64、从末项到首项的 Fisher–Yates 置换、模块种子序号和
  BLAKE3-128 命名空间派生；
- 三种模块图配置档在 `N = 1`、`N = 2` 时的全部展开模块、置换后导入、跨模块引用、
  模块种子序号与命名空间已知向量；
- `LF-COMP-ID-v1` 在三种模块图、`short-unique-v1` 和 `N = 1` 下的二十二种身份
  声明、十条所有者关系、完整规范记录流与 SHA-256 语义摘要已知向量；
- `LF-COMP-ID-v1` 从来源输入到有类型抽象语法树（typed AST）、高层中间表示
  （HIR）、中层中间表示（MIR）、已验证规范低层中间表示（canonical LIR）和输出
  构造的因果管线；空诊断与实际暂存容量按同一八项阶段分解记账；
- 单调时钟十万次差值观测，以及“计时前规模计划—单一外层计时区—停表后摘要与精确
  预言机”的最小测量原语；
- 同一编译器实例内的冷实例、三次不计时预热和七次稳定容量复用基础能力：样本间释放
  全部语义值，只保留已清空的阶段容器容量，以可失败精确预留拒绝不可满足的容量请求，
  并逐阶段报告实际保留容量字节；
- `LF-COMP-ID-v1` 七个独立新进程的冷实例试运行基础：父进程签发与进程号、地址和
  运行标识符无关的编译器实例标识符，子进程只回传实例标识、墙钟时间和语义摘要等最小
  结果，父进程核对七个身份互异、语义摘要一致，并以精确整数计算中位数与绝对中位差
  （MAD）。

代码中的 v1 常量只用于证明清单字段与已接受契约精确一致并在漂移时失败，不构成第二
事实源；研究语义仍以已验证的工作负载清单为权威。

已知向量均绑定工作负载清单摘要，不是生产制品或正式性能证据：

- `known-vectors/module-graphs-v1.json`：精确长度 `6545` 字节，SHA-256
  `abe175a0982c6483619fb65738011c97e7871faf247531f4a46cffb136da41f5`；
- `known-vectors/identity-records-v1.json`：精确长度 `109533` 字节，SHA-256
  `b78d429e586a231ba20e9710b198834e9df7e3d5b12976635fc7da30149f27f1`。

`LF-COMP-ID-v1` 的独立精确研究预言机已经覆盖三种模块图在 `N = 1`、`N = 2`
下的六个用例，并逐项核对身份声明、所有者关系、来源字节、字符串字节、来源位置、
MIR 语义记录、canonical LIR、最终规范记录流、语义摘要和八项阶段公式结果。阶段
生产路径物化逐模块来源位置、记录种类、符号序号、解析目标和连续载荷缓冲区；MIR
由 HIR 解析结果构造，canonical LIR 再独立完成所有者序号分配与规范排序。同长度错误
引用和同长度字符串替换均有拒绝测试。其余工作负载的完整记录流、当前固定样例研究
投影、正式新进程轮次编排、停止护栏、证据写出器和正式
`run --protocol compiler-calibration-v1` 入口仍须按设计文档继续实现；在完整正确性
与 pilot 证据落盘前，不得宣称任何正式预算数字。

现有单次测量原语只证明外层计时边界能够隔离 SHA-256、完整输出比较和独立预言机；
稳定容量实例尚未建立正式轮次或机器可读证据。七进程试运行没有执行停止护栏，也不
写入编译器校准证据 v1（Compiler Calibration Evidence v1）；其内部子进程 JSON 只
用于父子进程通信，不是正式证据封套。上述原语单独输出的墙钟值和保留容量没有预算
资格，试运行中即使满足时钟分辨率、离散程度与摘要一致性条件，也不得据此选择参考规模
`B`。

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  verify-contract
```

重新生成两份仓库内已知向量：

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  write-known-vectors
```

写入身份向量前，命令会先要求生产者与独立预言机的六个用例完全一致；任一用例不一致
时不会写入。

只需检查标准输出时，可分别使用 `print-module-graph-known-vectors` 与
`print-identity-known-vectors`。

执行 `LF-COMP-ID-v1` 的生产者/独立预言机六用例交叉验证（同时核对阶段精确内容与
公式结果）：

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  verify-identity-oracle
```

执行七个独立新进程的冷实例试运行（示例只使用 `N = 1` 验证编排，不产生正式预算
证据）：

```powershell
cargo +1.96.0 run --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  smoke-identity-fresh-process-pilot local-smoke wide-star-v1 1
```
