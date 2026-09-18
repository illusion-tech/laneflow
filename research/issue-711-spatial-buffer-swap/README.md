# Spatial 缓冲所有权交换 A/B 测量

关联 [#711](https://github.com/illusion-tech/laneflow/issues/711)。本目录是独立
workspace 的研究程序，用于对照同一测量程序在两份真实生产源码上的完整
`SpatialSession::extract_pose_batch` 路径表现：

- **before（A）**：生产基线 `b99a282ee399e15687301265dd8b27e8f7457d27`，成功提交
  仍为 `records.clear()` + `extend_from_slice` 复制。
- **after（B）**：仅修改成功提交尾部的缓冲所有权交换（`std::mem::swap` +
  接管后 `clear` 保留容量）。

两份源码使用字节相同的测量程序（`src/main.rs`、`Cargo.toml`、`Cargo.lock` 由
SHA-256 记录在各 `environment.json` 的 `sources`/`manifest`/`lockfile`）。A 侧
工作树只允许本目录未跟踪存在（`allowUntracked`），跟踪文件必须保持基线原样；
B 侧在测量源码提交后取证，只允许 evidence 输出未跟踪。

## 测量对象与口径

- 主指标：完整 `extract_pose_batch` 调用（scratch 准备、全部输入采样、frame
  检查、成功提交）。swap 微内核不在本程序测量范围。
- 输入：受检 LFCA fixture `lfca-full-spatial`（9 边、2 个 canonical frame、
  1 个显式泊位）构建的共享根；输入为 frame 0 内三条边上的 `PoseInput` 列表，
  规模口径为**记录数**（0 / 1 / 1_000 / 10_000 / 100_000），不是车辆数。
- 停车探针对同一静态泊位重复采样并单独标注，不冒充大量真实停车车辆。
- 场景：steady（各规模稳定成功路径）、cold（全新配对首次调用）、grow
  （1k→10k→100k 连续增长）、shrink（100k 暖机后回 1k）、alternate（两个 output
  交替）、fresh_output（每次换入全新 output）、fail_last（末条失败）、retry
  （失败后重试）、parking、retained_build（全新配对连续三次调用到稳态，区域
  累计 allocated bytes 即两侧 records backing 的建立字节）。
- 计时：每 case 暖机 4 次、7 个样本；steady/shrink/fail_last/parking 每样本
  32 次调用，alternate/fresh_output/retry 每样本 32 次调用（成对计入），
  cold/grow 每样本 1 次且不暖机。先对每样本除以调用次数，再取每进程 7 样本
  中位数，最后取三进程中位数。
- 分配计数构建（`--features allocation`，`stats_alloc`）与正常墙钟构建分开
  运行；墙钟 CSV 的分配四列为占位零。
- fixture 构建、输入准备、输出快照、完整对拍、摘要打印与文件写入不进入热
  路径计时；每个 case 在计时外验证记录数、身份顺序、header、token，并输出
  完整批次的 SHA-256 oracle 摘要（header + 记录身份 + 位模式级 pose 分量）。
- 测量循环对输入与输出使用 `black_box`；它是尽力而为的优化屏障，不构成
  “测量绝对可靠”的保证。
- 运行：固定单个 logical processor（≥17 核时固定第 17 个，否则最高位），
  平衡电源方案，未锁频。三组 A/B 墙钟进程按 A₁B₁B₂A₂A₃B₃ 交错；分配构建
  每变体一次。详细环境见各 `evidence/*/run*/environment.json`。

## 结果

取证后填写：见 [results.csv](results.csv) 与本节汇总；原始记录在
[evidence](evidence/)。历史 #681 数值产生自不同机器、提交与固定输入局部测量，
不与本结果相减。

## 复现

在仓库根使用 pwsh 7、仓库工具链（rust-toolchain 固定 1.98.0）与已缓存依赖：

```powershell
cargo clippy --locked --offline --manifest-path research/issue-711-spatial-buffer-swap/Cargo.toml --all-targets --all-features -- -D warnings
cargo fmt --manifest-path research/issue-711-spatial-buffer-swap/Cargo.toml -- --check
cargo run --locked --offline --release --manifest-path research/issue-711-spatial-buffer-swap/Cargo.toml -- --smoke
# A 侧需另建基线工作树并复制本目录（未跟踪）后：
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/run.ps1 -Output research/issue-711-spatial-buffer-swap/evidence/before/run1 -AllowUntracked 'research/issue-711-spatial-buffer-swap/'
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/run.ps1 -Output research/issue-711-spatial-buffer-swap/evidence/after/run1 -AllowUntracked 'research/issue-711-spatial-buffer-swap/evidence'
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/analyze.ps1
cargo test --locked -p laneflow-spatial -p laneflow-bevy --tests
```

run.ps1 在构建前、后及采集后要求 HEAD 不变且跟踪文件干净；evidence 目录必须
是新目录；binary 保留在取证目录但不入库，其 SHA-256 记录于 environment.json。
测量源码提交后再取证；证据另行提交，不 amend/rebase 测量源码提交。
