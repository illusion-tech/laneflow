# #876 测量二进制去除本机路径：方案、实现与验证

**日期**：2026-10-13；Refs #876，关联 #707（父测量线）、#861（长窗验收暴露的发布冲突）。

## 1. 结论

正式测量二进制改用 `cargo +1.98.0 run --locked -p xtask -- build-measurement` 构建：
冻结的 `cargo +1.98.0 build -p laneflow-urban-harness --release --locked` 附加三条
固定顺序的 `--remap-path-prefix`（工作树→`<worktree>`、Cargo 主目录→`<cargo-home>`、
rustup 工具链→`<rustup-toolchain>`）与 `CARGO_INCREMENTAL=0`。构建后固定执行
`check-binary-paths` 扫描，命中任何本机根（用户目录、Cargo 主目录、
rustup 主目录/工具链、工作树，以及调用方追加的证据根）即失败。测量载荷升级为
`urban-performance-measurements-v5`：provenance 新增 `build_rustflags`（来源端
占位符归一化后的实际构建 flag）与 `build_profile` 两个固定协议字段，普通 cargo
构建的二进制不再通过正式测量来源校验，旧口径载荷不与新口径混合聚合。

同一提交两种构建各跑 10k 短窗：交通文件逐字节一致，耗时差异在运行间波动内；
新构建的二进制扫描不到任何本机路径，并以正式 performance 窗口端到端跑通 v5
来源校验。该二进制已原样上传（[Research Evidence
prerelease](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-876-pathless-binary-c5e811a9)，
索引见 [evidence/measurement-binary.toml](evidence/measurement-binary.toml)，
制品经重新下载核对 SHA-256），执行身份与公开制品一致。

## 2. 方案调查

- Cargo `trim-paths`：在固定工具链实测未稳定——Cargo 1.98.0 拒绝
  `[profile.release] trim-paths = true`（"feature `trim-paths` is required"），
  不能作为映射入口。
- `--remap-path-prefix`：稳定可用。`CARGO_ENCODED_RUSTFLAGS` 在 Cargo 的 flag
  来源中优先级最高（覆盖 config 的 `build.rustflags` 与用户环境 `RUSTFLAGS`），
  构建入口设置后即锁定实际 flag 集合。
- 基线盘点（同提交普通 release 构建，`CARGO_INCREMENTAL=0`）：二进制内嵌两类
  本机路径——rustup 工具链 sysroot（86 处，std panic 位置经 rust-src 组件落到
  本机 `lib/rustlib/src/rust/...`）与 Cargo registry 源码目录（155 处）；工作树
  路径未出现（Cargo 对工作区成员传相对路径），仍作第三类固定映射以防御其他
  构建布局。remap 后三类命中全部归零（§4.1）。
- 构建期捕获通道：wire 审计（`xtask check-wire-audit`）禁止 harness 携带
  build.rs（build.rs 可生成文本扫描不可见的 Rust 源码，fail closed）。改用
  编译期环境变量：实测 Cargo 把进程环境传给 rustc，`option_env!` 可读
  `CARGO_ENCODED_RUSTFLAGS` 等变量；但直接嵌入该变量会把 remap 来源端（本机
  路径）写进二进制，故构建入口按同一映射表另写一份归一化记录
  `LANEFLOW_MEASUREMENT_RUSTFLAGS` 供嵌入。profile 无法经环境变量读取
  （`PROFILE`/`OPT_LEVEL`/`DEBUG` 只对 build.rs 可见），由
  `cfg!(debug_assertions)` 派生；opt-level 等细节由锁定的 workspace release
  profile 与冻结命令承载。
- `measurements.toml` 的 `invocation` 字段仍记录运行侧实际命令行（含本机路径）：
  它属于文本运行记录，发布时沿用既有占位符副本做法；本 Issue 的"原样发布"
  对象是二进制本身（改字节即改变登记的 SHA-256，无法事后脱敏）。

## 3. 实现

- `report.rs`：provenance 新增 `build_rustflags`（编译期
  `option_env!("LANEFLOW_MEASUREMENT_RUSTFLAGS")` 嵌入的归一化 flag 记录）与
  `build_profile`（`cfg!(debug_assertions)` 派生）两个固定协议字段，
  与 `build_parameters`（改为 xtask 构建入口）一起逐项校验；三轮聚合的
  provenance 全等比较自动覆盖新字段。
- `xtask/src/measurement.rs` 的 `build-measurement`：正式构建唯一入口；同一张
  映射表写出实际 `CARGO_ENCODED_RUSTFLAGS` 与归一化记录（来源端替换为占位符，
  永不携带本机路径）；每次正式构建使用全新 target 目录
  `target/release-measurement/`（先清空，杜绝构建脚本缓存携带未声明输入）。
  同时钉住 release profile 各键
  （`CARGO_PROFILE_RELEASE_*`）与工具链内 rustc（`CARGO_BUILD_RUSTC`），清除
  `RUSTC`/wrapper 类变量与本机原生工具链变量（`CC`/`CFLAGS`/`AR` 等的
  plain/`HOST_`/`TARGET_`/目标三元组限定形式），审计采集机 Cargo 配置
  （用户级与祖先目录，递归跟随 `include`；拒绝 profile/paths/source/patch/env
  表与 build.rustc*/rustflags、target.*.linker/runner/rustflags），并要求
  `cargo metadata` 的全部依赖源码落在工作树
  或 Cargo 主目录内（目录替换等逃逸 fail closed）。构建后正向断言二进制嵌入了
  期望的归一化串（rustc dep-info 本已跟踪 `option_env!` 触发重编，此为纵深
  防御），再固定
  执行扫描；回执（占位符、字节数、SHA-256）写入 `--receipt`（默认 target 下，
  不回写源码树）。
- 同模块的 `check-binary-paths`：独立扫描入口，ASCII 大小写折叠后对 UTF-8
  （两种目录分隔符）与 UTF-16LE 逐个根检查；派生根在本机必须可解析，显式
  指定的根（`--evidence-root` / `--extra-root`）不存在的路径也照扫。输出只含
  占位符、命中计数与字节偏移，可随证据公开；发布前复扫使用同一命令。

## 4. 验证

被测提交：`c5e811a96dc005eed476c1d3c1699cb0c210eea6`（本 PR 实现提交；其后的
文档提交不改 harness 构建输入）。
采集机：`amd-ryzen9-9955hx-16c-32t-64gb-hyperv` / `win11-balanced-381b4222`。
证据目录：`<worktree>/target/issue876-evidence/`（Git 忽略的本地生成数据，重建
入口即本节命令；封存制品见 §1 的 prerelease 与索引）。

### 4.1 扫描对照

| 构建      | SHA-256                                                            |  字节数 | 扫描                                                                                        |
| --------- | ------------------------------------------------------------------ | ------: | ------------------------------------------------------------------------------------------- |
| 普通（A） | `74b45e38e0f71093111985e813a6cd981c19e42d8be6791f9f4e45c770bd8d51` | 6748672 | fail：`<user-profile>`/`<rustup-home>`/`<rustup-toolchain>` 各 86 处、`<cargo-home>` 155 处 |
| 包装（B） | `a6ac4e0bbbde48e715c4f1bd5509d28dfcd28fffd21a7c20d1fd41448bd71dba` | 6748160 | pass：五类根零命中                                                                          |

### 4.2 同提交短窗逐字节一致与计时波动

10k MIXED-PEAK probe（512 暖机 + 4096 观察），workers=1，`--diagnostics`，
顺序 A1、B1、B2、A2。三臂对拍全部 `probe-match`（A1↔B1 跨二进制、A1↔A2、
B1↔B2），四份交通文件两臂逐字节相同：

| 文件               | SHA-256（A1 = B1）                                                 |
| ------------------ | ------------------------------------------------------------------ |
| resolved-plan.toml | `9699aac9a0cb905aee4abf39cb40fb75c458f311cf160bb1df4343d35729192a` |
| ticks.jsonl        | `46e1a81b56329559685818f986ad861acd522d47be0740bbcf0b18fe6d518c97` |
| commands.jsonl     | `fe29de61bfbd890200b9cc082b58e6fb99963f886fa7d0049763f4c80e034c86` |
| events.jsonl       | `920d15e4ea6ba525630d36c648be4dc0c85c45c2c795ae641080905e57b55a4b` |

观察窗耗时（ms/拍，最近秩分位；最终两个二进制，ABBA 顺序）：

| 运行       | step p50 | step p95 | 观测 p50 | 观测 p95 |
| ---------- | -------: | -------: | -------: | -------: |
| A1（普通） |    2.735 |    3.504 |    2.137 |    2.551 |
| B1（包装） |    2.759 |    3.663 |    2.119 |    2.568 |
| B2（包装） |    3.042 |    3.944 |    2.265 |    2.783 |
| A2（普通） |    3.559 |    4.835 |    2.514 |    3.229 |

step p95 四轮运行间极差 1.33 ms，同位 A−B 差值（−0.16 / +0.89 ms）方向相反、
无稳定 A−B 分离，落在运行间波动内。remap 不改变优化结果与运行行为。
短窗命令样本多为零（无命令拍占多数），不列为依据。

### 4.3 正式 performance 端到端（v5 provenance）

- 负路径：普通构建（A）运行正式 performance 计划在初始化前被
  "missing or unavailable performance provenance" 拒绝，退出码 1，不留输出目录。
- 正路径：B 跑 10k 正式 performance 一轮（30,624 暖机 + 61,248 观察，共
  91,872 拍，workers=1），`performance-round-complete`，全程 499.1 s；
  初始化前与窗口结束后来源校验均通过。计划摘要
  `045b87c845f270e5de51da0b47a9d06eb8f8396cd5ec795a73f9276bc7029ceb`，
  `measurements.toml`（3,258,140 字节，SHA-256
  `78ab391e5b679c95cb3580d01353ad0b69ab3f60374e021035d7d190b024a4f2`）记录
  `version = "urban-performance-measurements-v5"` 与 §3 两个新字段的冻结值；
  `diagnostics.json` 的二进制摘要、provenance 提交、回执 `source_commit` 与
  上传制品四者一致。
  本轮只验证协议端到端，不构成性能结论，也不替代 #707 的三轮认证。

### 4.4 发布前复扫

`check-binary-paths --evidence-root <evidence-root>` 对 B 复扫通过
（六类根零命中），作为取证流程上传前的固定步骤。

## 5. 后续

- 本 PR 的 10k 正式轮二进制已原样上传并登记（§1 链接与索引），满足 #876 的
  发布验收；后续正式或验收测量沿用同一入口：测量 worktree 干净检出目标提交，
  运行 `xtask build-measurement`，二进制连同构建回执原样上传，证据索引登记下载
  地址、字节数与 SHA-256。
- 取证脚本（证据根侧，仓库外）把"构建→扫描→上传"替换为上述入口。
