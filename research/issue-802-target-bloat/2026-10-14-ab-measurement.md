# issue #802 A/B 实测：dev 档调试信息降级 + 关闭 incremental

本文件是 #802 验收项「量化前后对比（体积、冷/增量构建时长、诊断能力）」的实测记录。

> **作废声明**：本仓此前记录的「冷构建 19m38.9s」是在**大批量删除 16 个 worktree 的
> target 与构建并发进行时**测得的，磁盘 IO 争用把构建放大了约 3.5 倍。本轮为重启后
> 无其它负载环境下的完整重测，此前所有时长数据以本文件为准。

---

## 1. 测量方法

| 项 | 取值 |
|---|---|
| 工作树 | `E:/projects/worktrees/802-target-profile/laneflow`（分支 `codex/802-target-profile`） |
| 工具链 | `cargo 1.98.0 (797e8a9bc 2026-08-05)` / `rustc 1.98.0 (88d9e12ae 2026-08-18)`，Windows MSVC |
| 基线配置 | `git stash` 掉本次改动后的 `origin/main` 配置（`[profile.dev]` 无 `debug` / `incremental` 设置） |
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

> 增量重建的 7m05.6s → 2m33.1s 是本次最反直觉的一项：**关掉 incremental 后增量重建
> 反而快了 4 分半**。原因是 `incremental = true` 需要额外读写增量缓存，且基线
> `debug = 2` 下 codegen 与链接都显著更重；两者叠加后，重编 9 个下游 crate +
> 重链 151 个测试二进制的总耗时反而更高。Cargo Book 所述「关闭 incremental 会让增量
> 重编变慢」的取舍在本仓实测不成立。

### 2.2 产物构成

**基线（16.0 GiB）**

| 组成 | 体积 | 个数 | 占比 |
|---|---:|---:|---:|
| `.pdb` | 8.3 GiB | 159 | 51.8% |
| `debug/incremental` | 5.8 GiB | — | 36.3% |
| `.o` | 2.5 GiB | 11 757 | 15.4% |
| `.bin`（`dep-graph.bin` 等） | 2.0 GiB | 459 | 12.6% |
| `.bc` | 1.3 GiB | 4 604 | 7.8% |
| `.rlib` | 1.1 GiB | 159 | 7.0% |
| `.exe` | 690.0 MiB | 151 | 4.2% |

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

### 2.3 诊断能力（#802 验收项）

| 能力 | 基线 | 改造后 | 结论 |
|---|---|---|---|
| panic 栈 | `panicked at …\_tmp_panic_probe.rs:3:5`，帧带 `.\tests\_tmp_panic_probe.rs:3` | 同左，完全一致 | ✅ 无退化 |
| dhat 栈（`ftbl`） | 9 帧，8 帧带 `file:line` | 24 帧，**23 帧带 `file:line`** | ✅ 无退化 |

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

## 4. 外推与遗留

- 单工作树 16.0 → 3.3 GiB（−79%）。本机 16 个带 `target` 的工作树合计 202.3 GiB，
  按同比例外推约 **42 GiB**；但各树的 `incremental/` 会随使用时长继续累积（759 那棵
  单树 incremental 就达 34.8 GiB），关闭后这部分不再增长，长期收益大于静态外推。
- 未在本轮覆盖：kache（内容寻址共享缓存）、`CARGO_BUILD_BUILD_DIR` 分离中间产物、
  定期清扫。三者仍列为 #802 之后的 P1/P2 项。
- 环境坑（记录备查）：MSVC `link.exe` 会因等不到 `mspdbsrv.exe` 而无限期挂起，
  表现为「构建卡住、CPU 与磁盘均为零」。构建前先常驻
  `mspdbsrv.exe -start -spawn -shutdowntime -1` 可规避；本轮重测即采用该方式，
  未再复现。
