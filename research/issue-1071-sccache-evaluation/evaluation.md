# 内容寻址编译缓存（sccache 类）评估记录（#1071）

> 本记录为 [#1071](https://github.com/illusion-tech/laneflow/issues/1071) 的评估产出；实施准入决议、ADR 判断与失败语义选择由维护者在该 Issue 裁定，本记录只给建议与证据。

评估环境：本机专用工作树（分支 `research/1071-sccache-eval`，基线 `232198ae`）；Windows 11 本机；Rust 1.99.1（`x86_64-pc-windows-msvc`）。被评估工具：sccache **v0.19.0**（2026-10-24 发布），本机实验用 `sccache-v0.19.0-x86_64-pc-windows-msvc.zip`，SHA-256 `28be8c61f6b2c89b9eb2869f6af7f78e6f992e78a8abae0ce855fcc129cbeac1`，与 GitHub 发布 API 的官方 digest 一致。CI 落地对应 Linux 资产 `sccache-v0.19.0-x86_64-unknown-linux-musl.tar.gz`（官方 digest `aa73c7270c997894038c0d8f0d297865f57569e1687883322bc431713d4bb9dd`）。

## 1. 正确性论证（纸面）

sccache 作为编译器包装器（`RUSTC_WRAPPER=sccache`，cargo ≥1.40 原生支持），缓存命中条件与 mtime 无关：

- **Rust 缓存键**（官方 `docs/Caching.md`）：对每个被编译文件计算 blake3 摘要，并混合 rustc 可执行文件路径、host triple、sysroot 路径、sysroot `lib/` 全部共享库摘要、解析后的 rustc 参数（含 `--` 之后的尾参）。因此：
  - 源码内容变化 → 摘要变化 → 必然重编译（**内容寻址，天然绕开 #1061 发现 2 的 mtime 墙**）；
  - 工具链升级（rustc 路径 / sysroot 库摘要）→ 键变化 → 自动失效，不会跨版本复用；
  - 编译参数（含 `-F unsafe_code` 类尾参）逐字参与哈希 → 不同 lint 形态、不同特征形态分别命中，不会串用。
- **失配 / 错误行为**（官方 README）：默认 sccache 与其本地服务通信失败时**构建直接失败**（失败关闭，不产出陈旧制品）；`SCCACHE_IGNORE_SERVER_IO_ERROR=1` 可改为降级直连真实 rustc（仍然正确，只是不缓存）。v0.19.0 将远端读错误从"静默按 miss 处理"改为显式 cache error（发布说明 #2928），错误不再伪装成未命中。
- **等价性要点**：miss 时 sccache 以原参数调用真实 rustc 并存储产物；hit 时返回的是一次"同源码摘要 + 同参数 + 同编译器身份"调用的产物。包装器不修改参数、不修改产物语义。
- **已知边界**（官方 README "Known Caveats"）：`bin` / `dylib` / `cdylib` / `proc-macro` 等需链接的 crate 不可缓存（测试二进制、最终链接留在热运行地板里）；增量编译产物不可缓存——CI 侧 rust-cache 已注入 `CARGO_INCREMENTAL: 0`，满足前提。

## 2. 信任面与实验 1（实测：hermetic 子编译与 job 级包装器完全隔离）

**机制**：`xtask/src/wire_audit.rs` 的 `HERMETIC_SCRUB_ENV_EXACT`（:176-187）精确剔除 `RUSTC` / `RUSTC_WRAPPER` / `RUSTC_WORKSPACE_WRAPPER` / `CARGO_BUILD_RUSTC*` / `RUSTFLAGS*` / `RUSTC_BOOTSTRAP` 共 10 键，外加模式剔除 `CARGO_TARGET_*_RUSTFLAGS`（:190-193）；`hermetic_cargo_command`（:1240-1259）以仓库外 cwd + 逐键 `env_remove` 构造命令。

**实验**：透传包装器 `wrap-log.cmd`（记录 `%*` 后原样转发），全程以 `RUSTC_WRAPPER` 挂载：

| 阶段       | 命令                                                                                                 | 结果                                                            | WRAP 日志行数                    |
| ---------- | ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- | -------------------------------- |
| D 正对照   | `cargo +1.99.1 build --locked -p xtask` + `test --no-run`（wrapper 在场）                            | 构建成功                                                        | **52**（包装器确实拦截普通构建） |
| A 主体     | `target\debug\xtask.exe check-wire-audit`（wrapper 在场，`CARGO`=1.99.1、`RUSTUP_TOOLCHAIN=1.99.1`） | **exit=0 审计全绿**，369s（含 hermetic 双配置编译与三路金丝雀） | **0**                            |
| B 单测     | `xtask-724e65963a3e9f0b.exe wire_audit`（wrapper 在场）                                              | **28 passed / 0 failed**                                        | **0**                            |
| C 反面对照 | `cargo +1.99.1 check` 独立 scratch crate（wrapper 在场）                                             | 构建成功                                                        | **6**（同场条件下日志确实工作）  |

**结论**：xtask 运行后的 hermetic 子编译面（默认+全特性双配置 `-F` 编译、三路注入金丝雀、hermetic metadata）在 job 级 `RUSTC_WRAPPER` 在场时**零经过包装器**；隔离来自 `HERMETIC_SCRUB_ENV_EXACT` 而非巧合（C 组证明）。金丝雀三路注入语义（manifest [lints] / `RUSTFLAGS=-A` / cwd `.cargo/config` rustflags 须编译失败且指向 unsafe）不受包装器影响——它们根本不在包装器路径上。

**结论边界（审阅意见修正）**：上述隔离**只覆盖 hermetic 子编译**。审计器自身的引导构建（`ci.yml` 的仓库外 cwd `cargo build -p xtask`）继承 job 环境，`RUSTC_WRAPPER` 在场时**会**经过包装器（D 组 52 次拦截即包含此类构建），`HERMETIC_SCRUB_ENV_EXACT` 要等 xtask 启动后才作用于其子进程。因此「sccache 无法触碰门禁审计」仅对 hermetic 子编译成立；引导构建面的处置列为实施前提（§10 前提 6）：推荐 `wire-audit` job 整体不挂 `RUSTC_WRAPPER`（该 job 无缓存收益——hermetic 子编译被剔除、引导构建缓存无意义），或为引导构建单独完成信任论证并留档。

**待实施确认项**：`SCCACHE_*` 环境键未列入剔除名单；因包装器路径不被触发，它们对 hermetic 编译惰性（实验 A/B 的零拦截即证），建议落地 PR 在文档中显式论证而不必扩名单。

## 3. 实验 2：收益实测（同目标开/关对照）

对象：`cargo +1.99.1 build --locked -p laneflow-runtime --tests`（runtime + dev-deps 闭包），三个干净 `CARGO_TARGET_DIR` 依次：

| 轮次 | 形态                                  | 墙钟                             | sccache 统计                                            |
| ---- | ------------------------------------- | -------------------------------- | ------------------------------------------------------- |
| TD1  | 直连冷构建                            | **70.8s**                        | —                                                       |
| TD2  | sccache 冷构建（缓存空）              | **71.6s**（开销 +1.1%）          | requests 164 / executed 93 / hits 0 / misses 91         |
| TD3  | sccache 热构建（target 清空、缓存热） | **26.5s**（相对直连 **-62.6%**） | 累计 hits 87（全部 Rust）/ misses 95；本轮约 87/91 命中 |

解读：冷运行几乎零开销（可接受"白开一轮"）；热运行编译墙钟降到约 37%，剩余 26.5s 是链接、bin/测试二进制 codegen 与 cargo 簿记——正是 README 声明的不可缓存面。口径声明：本机为 Windows 高核数开发机，绝对秒数不能对标 GitHub 4 核 runner，**比例（×0.37）仅作预估输入**，落地单必须做同提交开/关对照复测。

## 4. 缓存投毒威胁模型

后端采用 GHA 原生缓存（见 §5）时，作用域与写权限继承 actions/cache 的官方规则（GitHub 官方文档 "Restrictions for accessing a cache" / "Cache access for low-trust workflow triggers"）：

- **作用域**：run 只能读当前分支或默认分支（main）的缓存；PR run 额外可读 base 分支；PR run 创建的缓存落在 merge ref（`refs/pull/.../merge`）作用域，**只能被同一 PR 的重跑读取，不能被 base 分支或其他 PR 读取**；子分支 / 兄弟分支 / 不同 tag 互不可读。
- **写权限**：只有 `push` / `workflow_dispatch` / `repository_dispatch` / `delete` / `registry_package` / `page_build` / `schedule` 能创建或覆盖默认分支作用域的缓存；解析到默认分支的其他低信任触发器（`pull_request_target`、`issue_comment`、`workflow_run` 等）**只读**；`pull_request` 只写 merge ref 作用域。**投毒 main 可读缓存的唯一途径是把代码送进 main**（push / merge queue 落地）——此时攻击者已具备更大权限；PR 不能向 main 作用域写。
- **残余风险**（官方 "Best practices" 明言）："Cache contents are not signed or verified"——缓存内容无签名。有 PR 打开权的人可读 base 缓存（本项目缓存不含秘密）；被攻陷的可信 run（如 main 上被投毒的依赖）可留下持久对象码。缓解：
  1. 缓存只存编译产物且键含编译器身份（工具链升级自动失效）；
  2. PR 侧设 `SCCACHE_GHA_RW_MODE=READ_ONLY`（可选收紧），写只在 main push；
  3. `SCCACHE_GHA_VERSION` 一键整体换代清空；`SCCACHE_RECACHE` 重算单键；
  4. 把 `sccache --show-stats` 纳入 CI 日志，命中率突变（≈0 或骤降）可作异常信号；
  5. 10 GB 仓库配额与 7 天未访问淘汰、200 uploads/min 限额均需纳入运维观察（与 rust-cache 现有 ~2 GB 共享配额）。
- **merge_group**：queue ref 自身作用域写入互不可复用，但可读默认分支缓存 → 入队验证直接受益于 main 积累；此点仅有文档依据，**未实测**（见 §11）。

## 5. 后端对比

| 方案                                                                                                                                                                       | 评价                                                                                                                                                                                       |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **GHA 原生后端（推荐）**：`SCCACHE_GHA_ENABLED=on` + `ACTIONS_RESULTS_URL` / `ACTIONS_RUNTIME_TOKEN`，`SCCACHE_GHA_CACHE_TO`/`_FROM` 前缀键，可与本地盘组成多级 `disk,gha` | 无外部服务、无自备凭据；直接继承 GitHub 的作用域隔离与低信任写限制（§4）；对象级条目避免 #1061 发现 1 的"整键精确命中跳过保存"问题；限流时"构建继续、仅不落盘"（GHA.md），写失败不拖垮构建 |
| 本地盘目录 + actions/cache 手动存取                                                                                                                                        | 粗粒度整目录 blob，重新引入键管理与"精确命中跳过保存"坑（#1061 发现 1）；体积大、恢复慢；不推荐                                                                                            |
| S3 / Redis / GCS 等服务型                                                                                                                                                  | 引入网络面、凭据面与第三方服务信任，需另行安全评估；无必要，否决                                                                                                                           |

## 6. 收益预估（映射到 #1061 基线）

基线 = PR #1058 push 3（Rust checks 11m57s）；编译降幅按实验 2 的 ×0.37 保守预估：

| 步骤                         | 基线                             | sccache 热缓存预估 | 说明                                     |
| ---------------------------- | -------------------------------- | ------------------ | ---------------------------------------- |
| Run workspace tests          | 6m45s（编译 4m09s + 测试 2m36s） | ~4m08s             | 编译 249s→~92s，加不缓存的测试 156s      |
| Run execution evidence tests | 2m12s（≈全编译）                 | ~50s               | 特征形态独立命中                         |
| Check wire toolchain audit   | 1m54s                            | **1m54s 不变**     | 实验 1：wrapper 被 hermetic 剔除，零命中 |
| clippy 20s + 14s             | 不变                             | 不变               | 走 clippy-driver，不按获益计             |
| fmt / doctests / 架构 / 示例 | ~12s                             | 不变               |                                          |
| **Rust checks 合计**         | **~12m**                         | **~7m58s（热）**   | 冷缓存轮 ≈ 基线 +1%                      |

merge_group 入队（现 14m27s 全量）同样受益（读 main 缓存）；PR 二次 push、docs-only push 收益最大。首次部署与缓存淘汰后的首轮无收益，属预期。注：#1072 已把 wire 审计拆出 `Rust checks`（实测基线 10m0s），同一绝对节省量（约 4m）对应拆分后约 6m-6m30s。

## 7. 准入合规

- **安装**：对齐 `ci.yml:158-176` / `:418-435` 现行实践——官方 release asset + 仓库固定 SHA-256（发布 API 亦提供官方 digest 与 `.sha256` 旁文件可交叉核对）；升级单独审查 release notes / URL / 校验和（`dependency-security.md` §5.1）。
- **许可证**：sccache 为 Apache-2.0，落在 `dependency-security.md:78-79` 允许列表（Apache-2.0 / MIT）内；不新增允许项则无 ADR 0002 义务。
- **RustSec / cargo-deny 归属**：工具是 workspace 外独立二进制，不新增 crate 依赖面；按 `dependency-security.md` 第 8 节"工具基线"归档（对照 cargo-deny / cargo-nextest 条目）。
- **`SCCACHE_*` 与 `HERMETIC_SCRUB_ENV_EXACT`**：包装器被剔除 → `SCCACHE_*` 对 hermetic 编译惰性（实验 A/B 零拦截佐证）；建议落地 PR 在 `wire_audit.rs` 注释或治理文档补一句论证，不必扩剔除名单。
- **失败语义选择**：默认"服务不可达即构建失败"（失败关闭）与 `SCCACHE_IGNORE_SERVER_IO_ERROR=1`（降级直连）都不产生陈旧制品；建议门禁取默认失败关闭，理由与仓库"失败不被报告为成功"的口径一致。

## 8. 回滚

删除 workflow 安装步骤与 `RUSTC_WRAPPER` 环境即回直连编译；缓存是惰性内容寻址数据，无迁移；回滚后验证 = 对比同类 run 步时长回到基线并确认全部 required checks 全绿（清单以 `docs/governance/github-workflow.md` §7 为准）；可选 `SCCACHE_GHA_VERSION` 换代 + 删除 GHA 缓存条目彻底清场。回滚不留信任残余（无常驻服务、无凭据落仓）。

## 9. ADR 触发判断

**建议：不强制触发 ADR，但须留档信任面论证；两种情形升级为 ADR**：

- 不触发依据：变更仅作用于 CI 编译路径，产物由同一 rustc 以同一参数生成（§1 等价性），操作可整体回滚（§8），不改变任何产品契约、数据格式或 Runtime 行为；工具链信任面的收缩（hermetic 隔离）已实测（§2）。
- 触发条件：(a) sccache 未来参与**发布制品构建**（发布物的生成路径被缓存介入）；(b) 启用**分布式编译（dist）**（产物来源引入远端执行信任）。二者均属高影响难回退，须先 ADR。
- 最终判断归维护者签署。

## 10. 实施准入建议

**建议：准入通过（附前提条件）**。理由：正确性论证成立（内容寻址 + 编译器身份入键 + 失败关闭语义）；hermetic 子编译信任面已被实验 1 排除，引导构建面按前提 6 处置后排除；投毒面被 GitHub 作用域与低信任写限制结构性压制，残余风险有明确缓解；收益实测显著（编译 -62.6%，映射到 CI 约 12m → ~8m；#1072 后 ~10m 基线对应约 6m-6m30s）。

前提条件（进入实施单前必须闭合）：
1. 维护者签署本记录（含 §9 ADR 判断与 §7 失败语义选择）；
2. 实施单要求同提交开/关对照复测（Linux runner 实测比例替换本机 ×0.37）；
3. 落地形态按 §5 GHA 原生后端（可加 `disk` L0 多级），PR 侧评估 `SCCACHE_GHA_RW_MODE=READ_ONLY`；
4. `SCCACHE_*` 惰性论证与工具基线条目写入 `dependency-security.md`；stats 纳入 CI 日志；
5. 配额预算评估（与 rust-cache 现有条目共享 10 GB）；
6. 引导构建面处置：`wire-audit` job 不设置 `RUSTC_WRAPPER`（推荐，该 job 无缓存收益），或对审计器引导构建完成单独信任论证并留档（§2 结论边界）。

最终决议归维护者。

## 11. 未做 / 做不了的实验（如实列出）

1. **merge_group ref 的缓存可读性**：仅文档推断（"当前分支或默认分支"应覆盖 queue ref 读 main），未实测；落地单在首次入队时核对。
2. **GHA 后端实测**：本实验用本地盘缓存；`SCCACHE_GHA_*` 的实际写读、限流与 `SCCACHE_GHA_VERSION` 换代行为未在真实 workflow 中验证。
3. **Linux / GitHub runner 对照**：本机为 Windows 高核数机器，比例不外推为承诺；实施单复测。
4. **clippy 缓存行为**：按"不获益"保守估计，未实测 sccache 包裹 clippy-driver 的行为。
5. **分布式编译（dist）**：明确不在评估范围（若启用触发 ADR，见 §9）。
6. **fork PR 读 base 缓存的滥用面**：文档确认可读（无秘密可泄），未做渗透测试。

## 12. 证据清单

| #   | 位置 / 命令                                                                                                                                                                                                                                 | 说明                                                                                                                                                          |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `xtask/src/wire_audit.rs:176-193`                                                                                                                                                                                                           | hermetic env 剔除名单（10 键 + `CARGO_TARGET_*_RUSTFLAGS` 模式）                                                                                              |
| 2   | `xtask/src/wire_audit.rs:1240-1259`                                                                                                                                                                                                         | `hermetic_cargo_command` 仓库外 cwd + 逐键 `env_remove`                                                                                                       |
| 3   | `xtask/src/wire_audit.rs:225-282`                                                                                                                                                                                                           | 仓库 config `[build] rustc*` 等禁令（config 层信任面）                                                                                                        |
| 4   | 实验 1 阶段 D/A/B/C（本记录 §2 表格）                                                                                                                                                                                                       | wrapper 52/0/0/6 拦截；审计 exit=0；28 单测通过                                                                                                               |
| 5   | 实验 2 TD1/TD2/TD3（§3 表格）                                                                                                                                                                                                               | 70.8s / 71.6s / 26.5s；stats 87 hits                                                                                                                          |
| 6   | sccache 发布 API `releases/latest`                                                                                                                                                                                                          | v0.19.0 资产清单与官方 SHA-256 digest                                                                                                                         |
| 7   | [sccache docs/Caching.md](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Caching.md)                                                                                                                                                  | Rust 缓存键构成（blake3 源摘要 + 编译器身份 + 参数）                                                                                                          |
| 8   | [sccache README (v0.19.0)](https://github.com/mozilla/sccache/blob/v0.19.0/README.md)                                                                                                                                                       | wrapper 用法、失败语义（默认失败关闭 / `SCCACHE_IGNORE_SERVER_IO_ERROR`）、Rust caveats（bin/proc-macro/增量不可缓存）、`SCCACHE_BASEDIRS`、`SCCACHE_RECACHE` |
| 9   | [sccache docs/GHA.md](https://github.com/mozilla/sccache/blob/v0.19.0/docs/GHA.md)                                                                                                                                                          | GHA 后端启用、token、`SCCACHE_GHA_RW_MODE`、限流时构建继续                                                                                                    |
| 10  | [sccache docs/Configuration.md](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Configuration.md)                                                                                                                                      | `SCCACHE_*` 环境键、多级缓存 `disk,gha`、`SCCACHE_GHA_CACHE_TO/FROM`                                                                                          |
| 11  | [GitHub Docs: Dependency caching reference](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)（§Restrictions for accessing a cache / §Cache access for low-trust workflow triggers / §Best practices） | 作用域规则、PR merge-ref 缓存隔离、低信任触发器只读、"缓存内容无签名"、10 GB / 7 天 / 200 uploads 限额                                                        |
| 12  | `docs/governance/dependency-security.md:78-79`、`:100-121`                                                                                                                                                                                  | 许可证允许列表；工具钉版与升级审查                                                                                                                            |
| 13  | `.github/workflows/ci.yml:158-176`、`:418-435`                                                                                                                                                                                              | 现行工具钉版 + SHA-256 安装实践                                                                                                                               |
| 14  | #1061 实验记录评论（mtime 墙、B/C 特征形态、基线时长）                                                                                                                                                                                      | 收益预估输入                                                                                                                                                  |
