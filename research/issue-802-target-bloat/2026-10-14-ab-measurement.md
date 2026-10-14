# issue #802 A/B 实测：dev 档调试信息降级 + 关闭 incremental

本文件是 #802 验收项「量化前后对比（体积、冷/增量构建时长、诊断能力）」的实测记录。

> **作废声明**：本仓此前记录的「冷构建 19m38.9s」是在**大批量删除 16 个 worktree 的
> target 与构建并发进行时**测得的，磁盘 IO 争用把构建放大了约 3.5 倍。本轮为重启后
> 无其它负载环境下的完整重测，此前所有时长数据以本文件为准。

**冻结身份**

| 侧 | 身份 |
|---|---|
| 基线源码 | 提交 `d975a6811a6b243c91a19133d8b938352d06885a`（`origin/main`） |
| 改造后源码 | 提交 `e364921d7cf1d42fa8f043837a7e1bb126b8a374` |
| 改造后 `Cargo.toml` | blob `662dcfe9a1185789d2e096cb56526340df7bcaea` |
| 改造后 `CONTRIBUTING.md` | blob `ec912af83ba7988c1782ba06711d3ea8cc78af81` |
| 本报告（测量时版本） | blob `285047b9643cd598e6117a5788092895a74f4263` |

基线侧即 `git checkout d975a681 -- Cargo.toml CONTRIBUTING.md` 后的工作树；改造侧的
补丁内容由上述 blob 摘要固定，`origin/main` 前进不影响复现。

---

## 1. 测量方法

| 项 | 取值 |
|---|---|
| 工作树 | `<worktree>`（分支 `codex/802-target-profile`） |
| 工具链 | `cargo 1.98.0 (797e8a9bc 2026-08-05)` / `rustc 1.98.0 (88d9e12ae 2026-08-18)`，Windows MSVC |
| 基线配置 | 基线提交 `d975a6811a6b243c91a19133d8b938352d06885a` 的 `origin/main` 配置（`[profile.dev]` 无 `debug` / `incremental` 设置） |
| 改造后配置 | `[profile.dev] debug = "line-tables-only"` + `incremental = false` + `[profile.dev.package."*"] debug = false` |
| 冷构建 | 每次测量前完整删除 `target/`，再跑 `cargo +1.98.0 test --workspace --locked --no-run` |
| 增量构建 | 冷构建产物保留，`touch crates/laneflow-compiler/src/lib.rs`（只改 mtime，不改内容）后重跑同一命令 |
| 环境 | 重启后无其它负载；测量前先常驻 `mspdbsrv.exe`，规避 MSVC `link.exe` 等 PDB 服务不来的死锁 |
| 诊断能力 | 临时探针：panic 栈（`RUST_BACKTRACE=1`）+ dhat 栈（`ftbl` 帧是否带 `file:line`） |

---

## 2. 结果

### 2.1 核心指标

| 指标 | 基线 | 改造后 | 变化 |
|---|---:|---:|---:|
| **target 体积** | **16.0 GiB** | **3.3 GiB** | **−79.4%** |
| **冷构建时长** | 5m34.0s | 4m45.9s | −14.4% |
| **增量重建时长**（改 1 个文件） | 7m05.6s | **2m33.1s** | **−64.1%** |
| 全量 `cargo test --workspace` | — | 5m48.1s，2106 passed / 0 failed | — |

> ⚠️ 上表的 7m05.6s → 2m33.1s 是**两套 profile 的整体差异**，不能算作
> `incremental` 单独的贡献（基线同时是 `debug = 2`）。§2.3 的同调试设置对照显示，
> 单独关闭 `incremental` 会让单文件增量重建**变慢约 45 秒**。

### 2.2 产物构成

**基线（16.0 GiB）—— 顶层构成，占比之和为 100%**

| 组成 | 体积 | 个数 | 占比 |
|---|---:|---:|---:|
| `.pdb` | 8.3 GiB | 159 | 51.8% |
| `debug/incremental`（目录合计） | 5.8 GiB | — | 36.3% |
| `.rlib` | 1.1 GiB | 159 | 7.0% |
| `.exe` | 690.0 MiB | 151 | 4.2% |
| 其余（`.d` / `.rmeta` / 元数据等） | 约 0.1 GiB | — | 0.7% |

其中 `debug/incremental` 的 5.8 GiB **按下表再分解**（子项不再与父项并列计占比）：

| `debug/incremental` 子项 | 体积 | 个数 | 占父项 |
|---|---:|---:|---:|
| `.o` | 2.5 GiB | 11 757 | 43.1% |
| `.bin`（`dep-graph.bin` 等） | 2.0 GiB | 459 | 34.5% |
| `.bc` | 1.3 GiB | 4 604 | 22.4% |

**改造后（3.3 GiB）**

| 组成 | 体积 | 个数 | 占比 |
|---|---:|---:|---:|
| `.pdb` | 2.1 GiB | 159 | 62.8% |
| `.exe` | 657.3 MiB | 151 | 19.2% |
| `.rlib` | 435.2 MiB | 159 | 12.7% |
| `.rmeta` | 160.9 MiB | 159 | 4.7% |
| `debug/incremental` | **0** | — | — |
| `.o` / `.bc` / `.bin` | **0** | — | — |

消失的 `.o`（11 757 个）、`.bc`（4 604 个）、`.bin`（459 个）共约 5.8 GiB，
正是 incremental 缓存与小文件元数据开销所在。附带收益：产物文件数下降一个数量级，
`cargo clean` / 清扫工具不再需要处理十几万个随机小文件（这也是此前删除 target 极慢的
根因——瓶颈是文件数不是字节数）。

### 2.3 incremental 开/关的受控对照（隔离 §2.1 的归因）

两侧同为改造后的调试设置（`debug = "line-tables-only"` + 依赖 `debug = false`），
只翻转 `incremental`（用 `CARGO_INCREMENTAL=1` 覆盖 profile），同机、独立 target 目录：

| 指标 | `incremental = false`（本 PR） | `incremental = true` | 差异 |
|---|---:|---:|---:|
| 冷构建 | **4m45.9s** | 5m37.9s | 关闭快 52.0s |
| 单文件增量重建 | 2m33.1s | **1m47.9s** | **关闭慢 45.2s** |
| target 体积 | **3.3 GiB** | 9.00 GiB | 关闭省 5.7 GiB |
| 产物文件数 | **4 091** | 31 899 | 关闭少 87% |

结论：

- Cargo Book 所述取舍**在本仓成立**——关闭 `incremental` 确实让增量重编变慢（+45s）。
  §2.1 里 7m06 → 2m33 的大幅改善主要来自调试信息降级（基线 `debug = 2` 的 codegen
  与链接成本），而非关闭增量编译。
- 本 PR 仍选择关闭，理由是**权衡后划算**：每工作树省 5.7 GiB、冷构建快 52s，代价是
  每次单文件增量重建多 45s。在 16 个工作树、磁盘已告警的采集机上，5.7 GiB × 树数
  的收益远大于 45s；且 `incremental/` 是 Cargo 永不回收的目录，长期只增不减。
- 若后续引入 kache 等内容寻址共享缓存，可重新评估打开 `incremental`
  （kache 会强制 `CARGO_INCREMENTAL=0`，届时本取舍自动失效）。

### 2.4 诊断能力（#802 验收项）

| 能力 | 基线 | 改造后 | 结论 |
|---|---|---|---|
| panic 栈 | `panicked at …\_tmp_panic_probe.rs:3:5`，帧带 `.\tests\_tmp_panic_probe.rs:3` | 同左，完全一致 | ✅ 无退化 |
| dhat 栈（`ftbl`） | 9 帧，8 帧带 `file:line` | 24 帧，**23 帧带 `file:line`** | ✅ 无退化 |

### 2.5 依赖帧：受控对照（补充 §2.4 的边界）

§2.3 只覆盖了工作区帧。用**同一份探针**（`tests/_tmp_dep_frames.rs`，经 `dhat` 全局
分配器分配并抓取 `std::backtrace`）在两套配置下各跑一次，按帧来源分类统计
（原始 JSON 见 §4 制品）：

| 帧来源 | 基线 | 改造后 | 结论 |
|---|---|---|---|
| 工作区 crate | 9 / 9 带 `file:line` | 9 / 9 带 `file:line` | ✅ 无退化 |
| **本地编译的依赖**（`dhat-0.3.3`） | `dhat::impl$7::alloc (dhat-0.3.3\src\lib.rs:1176:0)` | `<dhat::Alloc as GlobalAlloc>::alloc (???:0:0)` | ⚠️ **丢失 `file:line`，符号名仍在** |
| sysroot（`std` / `alloc` / `core`） | 带 `file:line` | 带 `file:line` | ✅ 不受影响（预编译，`*` 覆盖不到） |

结论修正：诊断无退化**仅对工作区 crate 与 sysroot 成立**。故障或热点位于本地编译
的依赖内部时，回溯帧保留符号名但退化为 `???`，失去文件名与行号。这是
`[profile.dev.package."*"] debug = false` 的直接后果；需要完整依赖调试信息时，
在 `debugging` 档显式加 `[profile.debugging.package."*"] debug = true`
（见 §3.1 的继承实测）。

**唯一损失**：调试器看不到局部变量与类型（变量级调试信息缺失）。兜底方式：

```text
cargo build --profile debugging
```

`[profile.debugging] inherits = "dev"` 且 `debug = true` / `incremental = true`，
产物落在 `target/debugging`，首次使用需完整重建一次。这一条已写入 `CONTRIBUTING.md` §8.1。

---

## 3. 验收三件套（改造后配置）

| 检查 | 命令 | 结果 |
|---|---|---|
| metadata | `cargo +1.98.0 metadata --locked --format-version 1` | ✅ EXIT=0，1.9s |
| 依赖策略 | `cargo deny --locked --all-features check advisories bans licenses sources` | ✅ `advisories ok, bans ok, licenses ok, sources ok` |
| workspace 测试 | `cargo +1.98.0 test --workspace --locked` | ✅ 5m48.1s，2106 passed / 0 failed，EXIT=0 |

---

## 3.1 profile 继承语义验证（`debugging` 档的实际边界）

Cargo Book 未写明 `inherits` 是否继承 `package` 覆盖，故用最小工程实测：
两个 crate，`app`（工作区根包）依赖 `dep`（path 依赖，非工作区成员），
配置与本 PR 相同，取 `cargo build -v` 中每个 crate 的 `-C debuginfo=` 实参。

| profile | `app`（工作区 crate） | `dep`（依赖） |
|---|---|---|
| `dev` | `-C debuginfo=line-tables-only` | 无 `debuginfo` 实参（= 0） |
| `debugging` | `-C debuginfo=2` | 无 `debuginfo` 实参（= 0） |

结论：`[profile.dev.package."*"] debug = false` **会被 `inherits` 继承**，
`debugging` 档恢复的是工作区 crate 的变量级调试，依赖仍无调试信息。
已写入 `CONTRIBUTING.md` §8.1 与根 `Cargo.toml` 注释；需要依赖调试信息时
临时加 `[profile.debugging.package."*"] debug = true`。

---

## 4. 原始证据制品

完整原始字节（逐文件清单、构建日志、dhat 帧 JSON、panic 回溯、探针源码、冻结记录）
不进入 Git 源码树，另存为可下载制品：

- 下载地址：https://github.com/illusion-tech/laneflow/releases/download/research-evidence-802-dev-profile-d975a681/ab-802-dev-profile-d975a681.tar.gz
- 文件：`ab-802-dev-profile-d975a681.tar.gz`，**966 475 字节**，SHA-256
  `d674e87307fde91f36d862f2d2768fb0462d77d1a8c389c0f7a6e84a2486cbfa`
- 本地证据根：`<evidence-root>/issue-802/d975a681/`（checkout 外，只新增不覆盖）
- 复核方法：下载后先核对长度与 SHA-256，再 `tar -xzf ab-802-dev-profile-d975a681.tar.gz`
  解包；解包后用 `MANIFEST.tsv` 的逐文件 SHA-256 复核，
  并用 `measurement-freeze.json` 核对冻结提交、工具链与命令。
- 归档清单（22 项）含：`raw/*-file-inventory.tsv`（全量文件清单，基线 19 244 项 /
  改造后 4 091 项 / incremental 开 31 899 项）、`raw/*-summary.txt`（扩展名聚合）、
  `raw/0*-*.log`（构建与验收日志）、`raw/*-dep-frames.json`（依赖帧 dhat JSON）、
  `raw/*-panic.txt`（panic 回溯对照）、`probes/`（两份探针源码与三个统计脚本）。

---

## 5. 外推与遗留

- 单工作树 16.0 → 3.3 GiB（−79%）。本机 16 个带 `target` 的工作树合计 202.3 GiB，
  按同比例外推约 **42 GiB**；但各树的 `incremental/` 会随使用时长继续累积（759 那棵
  单树 incremental 就达 34.8 GiB），关闭后这部分不再增长，长期收益大于静态外推。
- 未在本轮覆盖：kache（内容寻址共享缓存）、`CARGO_BUILD_BUILD_DIR` 分离中间产物、
  定期清扫。三者仍列为 #802 之后的 P1/P2 项。
- 环境坑（记录备查）：MSVC `link.exe` 会因等不到 `mspdbsrv.exe` 而无限期挂起，
  表现为「构建卡住、CPU 与磁盘均为零」。构建前先常驻
  `mspdbsrv.exe -start -spawn -shutdowntime -1` 可规避；本轮重测即采用该方式，
  未再复现。
