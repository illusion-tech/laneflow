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
- `laneflow-core`、`laneflow-data` 与 `laneflow-spatial` 只由 `fixture-oracle` 在计时区外
  读取并独立投影当前夹具；
- 每种第三方候选拥有独立私有特性（feature）；
- `research-runner-full` 是正式研究执行器的封闭总特性（feature），其成员已在 #308 G2
  依赖审计中冻结；
- `xxhash-rust 0.8.18` 仅服务 XXH3/XXH64 研究候选。其 BSL-1.0 许可证通过
  `deny.toml` 中绑定精确版本的 #308 例外接受，不得扩散到生产依赖图；
- `sysinfo 0.39.6` 关闭默认特性并只启用 `system`，仅供父进程在计时区外刷新系统
  RAM、重算停止护栏；其 crates.io 校验和为
  `d2071df9448915b71c4fe6d25deaf1c22f12bd234f01540b77312bb8e41361e6`，许可证为
  MIT，不进入生产依赖图；
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
- `LF-COMP-CORRIDOR-v1` 对绑定的信号走廊、空间几何与停车夹具完成有类型模板投影，
  按 `N` 复制三百五十七项声明、六百二十七项关系和一千三百九十八个规范几何点；
  三种模块图均物化来源输入、有类型抽象语法树、高层中间表示、中层中间表示、
  已验证规范低层中间表示、诊断、暂存和输出构造八项阶段，并输出两千三百八十二条
  完整规范记录；
- `LF-COMP-JUNCTION-GRID-v1` 按四进口方向、禁止掉头的冻结公式生成十二个有向
  机动（Movement）、三十二条车道图边（LaneEdge）、三十六个机动门
  （ManeuverGate）、三十六条停止线（StopLine）和二十四个等待区（WaitingZone）；
  每单元物化一百六十六项声明、二百五十二项关系、六十四个规范几何点和四百八十二条
  完整规范记录。扩大 `N` 时，几何按宽度四千零九十六的行优先网格重新计算，既有
  工作单元的坐标不变；
- `LF-COMP-RESEARCH-CURRENT-FIXTURES-v1` 对信号走廊三件套、停车与信号基线、
  多机动门待转区三个当前固定样例执行非生产研究投影。每个文件先校验精确长度与
  SHA-256；ScenarioManifest 还逐角色核对制品名、媒体类型、摘要和长度，但不生成
  阶段记录。研究私有 JSON 投影与当前生产加载器规范化对象独立重建的模板逐项一致，
  再由独立身份和记录流预言机逐字节复核；该工作负载不进入规模发现、预算或候选排名；
- `LF-COMP-ID-v1` 从来源输入到有类型抽象语法树（typed AST）、高层中间表示
  （HIR）、中层中间表示（MIR）、已验证规范低层中间表示（canonical LIR）和输出
  构造的因果管线；空诊断与实际暂存容量按同一八项阶段分解记账；
- 单调时钟十万次差值观测，以及“计时前规模计划—单一外层计时区—停表后摘要与形状
  检查”的最小测量原语；独立精确预言机只由单独的 `oracle` 角色执行；
- 同一编译器实例内的冷实例、三次不计时预热和七次稳定容量复用基础能力：样本间释放
  全部语义值，只保留已清空的阶段容器容量，以可失败精确预留拒绝不可满足的容量请求，
  并逐阶段报告实际保留容量字节；
- 四个闭合二进制角色：`compiler-calibration-runner-v1` 只执行契约引导、编排、监控
  和结果汇总；`compiler-calibration-timing-v1` 只执行单一外层墙钟且以编译期常量
  消除逐容量请求记账；`compiler-calibration-attribution-v1` 独占受控分配、分配/
  重分配次数与字节、当前/峰值存续请求字节及保留容量归因；
  `compiler-calibration-oracle-v1` 只从受信任原始清单走独立精确构造路径。四个入口
  分别由四个可执行文件
  承载，角色描述测试会拒绝职责或记账模式混淆；
- `LF-COMP-ID-v1` 七个独立新进程的冷实例试运行基础：父进程签发与进程号、地址和
  运行标识符无关的编译器实例标识符，子进程只回传实例标识、墙钟时间和语义摘要等最小
  结果。七个子进程现在都必须解析为 `timing` 角色且显式报告未启用逐分配记账；父进程
  核对七个身份互异、语义摘要一致，并以精确整数计算中位数与绝对中位差（MAD）；
- 父进程启动前停止护栏预检：每个样本启动前刷新系统物理内存，从受信任清单重算主
  记录数、有类型序号上界和单缓冲区下界；首级只使用清单下界，后继严格二倍级别使用
  `5/4` 安全因子的受检 `u128` 精确上取整预测。预测达到本机阈值、可用物理内存低于
  四分之一或有类型序号越界时，不启动子进程；
- 归因子进程受控分配硬上限：十九个具名阶段缓冲区的每次容量增长都在
  `try_reserve_exact` 前以原子账本预占请求字节；越界时不执行该容量请求，正常返回
  `guard/allocation-hard-ceiling` 结构化结果。研究测试先从同一基线取得公共
  `peakLiveRequestedBytes`，再证明界内（at-bound）成功且加一（plus-one）在越界前失败。
  正式限制资格会在九个自然身份的校准/压力规模执行全部二十三个限制维度，并为存续字节
  上限保留两个独立 `attribution` 进程副本；timing 路径不执行该原子账本，只保留受检
  容量规划与可失败精确预留；
- 父子进程启动握手与父进程持续监控：父进程在放行管线前取得子进程私有字节初始快照，
  随后在两次轮询之间休眠一毫秒，并检查私有字节、完整子进程墙钟和系统可用物理内存。
  达到阈值或监控缺样时终止子进程，并返回结构化无效停止结果；Evidence v1 写出器会把
  这些结果与正常完成结果一并投影。正常完成的每个样本在试运行 JSON 中保存最后/峰值私有字节、观察
  次数、退出码和实际父进程墙钟。当前私有字节提供程序只在 Windows 上绑定
  `sysinfo 0.39.6` 后端的 `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage`；其他平台明确
  拒绝执行，不把通用虚拟内存冒充私有内存。回归测试以受控观察值在精确私有字节阈值
  触发父进程终止路径，并验证子进程已经等待回收；
- 与 Evidence v1 同形的运行结果与终止状态协议：正常完成、启动前拒绝、子进程内受检
  护栏、异常退出和父监控终止分别编码为五种 `exitKind`；未启动、正常退出码、POSIX
  信号和无法映射为非负退出码的 Windows 原生状态分别保留独立终止观察。Windows 原生
  状态保存无符号原始值，POSIX 构建保存信号号和原始 wait status，不能压扁成普通退出
  码。监控缺样、非零退出和父进程强制终止返回带状态、失效原因、值/原因观察、最后
  监控快照与诊断的结构化停止记录；受检护栏返回 `guarded`，其他异常只返回
  `invalid`，不再只抛出丢失上下文的字符串错误。
- `run --protocol compiler-calibration-v1 --output <path>` 正式研究入口：入口在任何
  测量前拒绝 debug 二进制、未启用 `research-runner-full` 的构建和
  脏工作树；三个可扩展工作负载与三种模块图配置档分别从 `N = 1` 开始严格二倍探测。
  每个候选级别使用七个独立的新进程冷实例样本，精确重算中位数/MAD、时钟量子阈值、
  语义摘要一致性和护栏状态；无效尝试完整保留并以新尝试身份重试，不能进入贡献集合。
  找到 `B` 后执行至少五级正式阶梯；每个新级别先取得一次插桩预检和一次独立预言机
  结果，再按两批、每批五轮运行非插桩时延与插桩归因子进程。每个子进程取得一次冷实例、
  三次不计时预热和七次稳定容量复用，随后从完整有效原始值重算中位数、MAD、相邻级别
  比值、成本拐点和校准/压力规模。检查点保留已完成状态，成功结束时发布正式执行
  检查点；该制品仍不是编译器校准证据 v1，也不包含预算建议或产品服务等级协议
  （SLA）。正式阶梯遇到无效轮次时保存事实并停止，不在单次研究运行内自动重试；排除
  环境干扰后使用新输出路径重跑完整协议。若受测子进程先于下一二倍级别预检触发运行中
  停止护栏，执行器会持久化统一作废后的尝试，把该自然身份记录为未找到可靠 `B`，再
  继续其他身份；它不会重试超限规模，也不会把冷实例运行伪装成
  `terminalGuardRunId` 所要求的实际 `guard-preflight` 运行。
- 全部限制维度的界内/加一资格、三十二次重复失败、恢复成功、全新实例预言机、残留存续
  字节与保留容量检查，以及走廊重复所有者语义失败资格；
- 私有容器与哈希候选矩阵：从冻结注册表形成依赖安全快照，先执行完整管线正确性资格和
  恒定哈希输入资格，再按三个规模角色、三个键域、两批平衡顺序执行新进程墙钟比较；
  进程私有字节只作为原始诊断，不冒充缺少同分层重复性包络的分类指标；
- Rust 原生 Evidence v1 写出器与独立验证器：绑定干净源提交、契约描述符、工作负载清单、
  Schema、Cargo.lock 和三个 release 研究二进制；独立重算工作负载计数、基础规模、
  中位数/MAD、正式阶梯、拐点、增长斜率、预算建议、限制资格、失败输入摘要和候选分类。
  写出路径保留每个无效、受护栏停止和失败运行，不从成功样本反推或补造原始观察。

代码中的 v1 常量只用于证明清单字段与已接受契约精确一致并在漂移时失败，不构成第二
事实源；研究语义仍以已验证的工作负载清单为权威。

已知向量均绑定工作负载清单摘要，不是生产制品或正式性能证据：

- `known-vectors/module-graphs-v1.json`：精确长度 `6545` 字节，SHA-256
  `246a9cccd916b30dac5d951a7db60ab6cee496d8d53934327f582e522346aae0`；
- `known-vectors/identity-records-v1.json`：精确长度 `108777` 字节，SHA-256
  `619e86b2a10d5edecc87b1e790351a3a0f9e89532f8c8437705151718b76db69`；
- `known-vectors/corridor-summary-v1.json`：精确长度 `9101` 字节，SHA-256
  `7b8a66802b586ee3806d05fce7f8b842a9f1298f4fedf15cff1c5deedd1b825c`；
- `known-vectors/junction-grid-summary-v1.json`：精确长度 `8601` 字节，SHA-256
  `c9d63824f88a98a51a5e194820695f6bca9e8ad8575f90759a202ee2c8da1fe2`；
- `known-vectors/current-fixtures-summary-v1.json`：精确长度 `11755` 字节，SHA-256
  `9abc9b91027fc29c2ca292e577a29ad3501557c68bc0ad683820985556155e6f`。

`LF-COMP-ID-v1` 的独立精确研究预言机已经覆盖三种模块图在 `N = 1`、`N = 2`
下的六个用例，并逐项核对身份声明、所有者关系、来源字节、字符串字节、来源位置、
MIR 语义记录、canonical LIR、最终规范记录流、语义摘要和八项阶段公式结果。阶段
生产路径物化逐模块来源位置、记录种类、符号序号、解析目标和连续载荷缓冲区；MIR
由 HIR 解析结果构造，canonical LIR 再独立完成所有者序号分配与规范排序。同长度错误
引用和同长度字符串替换均有拒绝测试。正式基础规模新进程编排、两批五轮正式阶梯、
精确拐点分析和
  `run --protocol compiler-calibration-v1` 入口已经落地。失败清理、候选比较、
  Evidence v1 写出和独立重算路径已经实现；在两批正式 R0 原始执行与报告完成前，仍不得
  把既有冷实例临时性能预算称为正式 R0 研究预算。

`LF-COMP-CORRIDOR-v1` 的独立预言机另行实现身份字段展开、StableId128 派生、十三类
语义记录、所有者局部序号、规范排序和记录流编码，并在三种模块图的 `N = 1`、
`N = 2` 六个用例中逐条、逐字节核对生产者。另一条夹具投影路径只消费当前生产
`laneflow-data` / Scenario loader 的规范化对象，独立重建全部实体、身份引用、关系
和几何，再与研究私有 JSON 读取器的模板逐项比较；因此夹具可接受但投影错误的自洽
结果也不能通过。

`LF-COMP-JUNCTION-GRID-v1` 的独立预言机重新实现机动、内部边、机动门、停止线、
等待区、路线和网格几何公式，并故意反转关系输入顺序；三种模块图的 `N = 1`、
`N = 2` 六个用例仍须与生产者得到逐条、逐字节相同的完整规范记录流。因此规范排序
不会依赖生成器关系遍历顺序，第二工作单元的几何也实际覆盖网格坐标公式。

现有单次测量辅助函数证明外层计时边界能够隔离 SHA-256 和摘要核对。基础规模发现的
每个完整级别都会在七个 timing 新进程之后，再启动一个受相同父进程护栏约束的独立
oracle 新进程；只有完整计数、完整有类型输出和语义摘要全部相等，该级别才可能形成
候选 `B`。基础规模发现已经具备正式尝试身份、无效尝试重试、九个自然身份的严格二倍
选择和持久检查点，其墙钟只来自无逐分配记账的 timing 二进制，预言机时延不进入性能
结论。正式阶梯已经分离冷实例与稳定容量复用、时延与归因指标，并从两批五轮的有效
  原始数据选择校准规模与压力规模。重复性包络、增长斜率、失败清理、候选矩阵和完整
  Evidence v1 代码路径均已建立；执行检查点本身仍不得作为正式 R0 预算、候选排名或
  正式 Evidence 引用，必须先由写出器生成并由独立验证器通过。Rust 重算子命令可以从
  已完成的基础规模原始样本形成明确标注的冷实例
临时性能预算；首轮结果见
[`v0.10-compiler-pilot-budget.md`](../../docs/reference/v0.10-compiler-pilot-budget.md)
及
[`v0.10-compiler-pilot-budget.json`](../../docs/reference/v0.10-compiler-pilot-budget.json)。
attribution 的诊断墙钟仍不得进入任何时延结论。

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  verify-contract
```

重新生成五份仓库内已知向量：

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-oracle -- `
  write-known-vectors
```

写入向量前，命令会先要求身份、走廊与路口网格三个可扩展工作负载各自的生产者/
独立预言机六个用例完全一致，并要求三个当前固定样例同时通过生产加载器投影与独立
记录流复核；任一用例不一致时不会写入。

执行 `LF-COMP-ID-v1`、`LF-COMP-CORRIDOR-v1` 与
`LF-COMP-JUNCTION-GRID-v1` 的生产者/独立预言机各六个用例，以及
`LF-COMP-RESEARCH-CURRENT-FIXTURES-v1` 三个固定用例的交叉验证（同时核对
身份阶段精确内容、结构工作负载的完整记录流、当前生产加载器投影与各自公式结果）：

```powershell
cargo +1.96.0 run --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-oracle -- `
  verify-matrix
```

执行七个独立新进程的冷实例试运行（示例只使用 `N = 1` 验证编排，不产生正式预算
证据）。runner 会从同一目录解析 timing 二进制，所以先以同一 profile 和特性集合构建
全部四个角色：

```powershell
cargo +1.96.0 build --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bins

.\target\release\issue-308-compiler-budget-calibration-research.exe `
  smoke-identity-fresh-process-pilot local-smoke wide-star-v1 1
```

执行九个自然身份的基础规模试运行与正式规模阶梯。输出路径建议位于仓库外的已存在目录；正式
入口会拒绝覆盖既有输出或既有 `.checkpoints` 分代目录。运行中断后，已完成状态仍保留
在 `<output-file>.checkpoints/`，应以新输出路径重新执行，不能把两次运行拼接。单入口
会先以相同锁定工具链、release 配置和封闭总特性构建同目录 timing、attribution 与
oracle 二进制，再分别核对角色、模式、职责和记账状态，因而不依赖预先存在的角色制品：

```powershell
cargo +1.96.0 run --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  run --protocol compiler-calibration-v1 `
  --environment C:\tmp\compiler-formal-environment.json `
  --output C:\tmp\compiler-formal-execution.json
```

环境声明 JSON 必须精确包含 `vendorPerformanceMode`、`biosFirmware`、
`sleepOrSessionLockObserved` 和 `thermalOrPowerThrottlingObserved`；前两项不能为空，后两项
必须由操作者按本轮实际状态填写。命令要求调用前工作树干净；输出是
`laneflow.compiler-calibration-formal-execution-checkpoint` v1，包含基础规模、
timing、attribution、oracle、正式阶梯、限制资格、失败恢复和候选矩阵；它不是
Compiler Calibration Evidence v1。

完整检查点完成后，以同一干净提交和同一组三个 release 二进制生成 Evidence v1，再用
独立 Rust 路径重新验证已写出的精确文件：

```powershell
cargo +1.96.0 run --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  write-evidence-v1 `
  C:\tmp\compiler-formal-execution.json `
  C:\tmp\compiler-calibration-evidence-v1.json

cargo +1.96.0 run --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  verify-evidence-v1 C:\tmp\compiler-calibration-evidence-v1.json
```

基础规模的全部自然身份完成后，可以直接从最新检查点先形成冷实例临时性能预算；每个
自然身份必须有七个有效冷实例样本、统一语义摘要和成功的独立预言机。该路径按同组样本
的观测上界与最大值/中位数离散比形成透明的保守建议，不等待正式阶梯追踪拐点：

```powershell
cargo +1.96.0 run --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --bin issue-308-compiler-budget-calibration-research -- `
  recompute-pilot-budget `
  C:\tmp\compiler-formal-execution.json.checkpoints\checkpoint-XXXXXXXX.json `
  C:\tmp\compiler-pilot-budget.json `
  C:\tmp\compiler-pilot-budget.md
```

这份临时预算只覆盖冷实例；稳定容量复用、正式拐点和候选比较保持未覆盖，不能由冷实例
数据冒充。`recompute-pilot-budget` 仍只验证会影响临时预算的数据关系，不替代完整
Evidence v1；完整 R0 结论必须来自上面的正式检查点、Evidence 写出和独立验证流程。
