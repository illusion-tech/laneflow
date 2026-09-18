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

取证于 2026-09-18（AMD Ryzen 9 9955HX、Windows 29661、平衡电源方案、单
logical processor 固定、Rust 1.98.0 MSVC release、opt-level 3）。每变体三个
独立正常进程（A₁B₁B₂A₂A₃B₃ 交错）+ 一次分配计数进程；汇总取每样本除以调用
次数 → 每进程 7 样本中位数 → 三进程中位数。完整数值见
[results.csv](results.csv)，三轮中位数与全部原始样本在 [evidence](evidence/)。

主要稳定场景（µs/调用，三进程中位数；三轮中位数见 evidence）：

| 场景                       | before |  after | 判读                         |
| -------------------------- | -----: | -----: | ---------------------------- |
| steady / 1 记录            |    116 |    106 | 噪声内                       |
| steady / 1_000 记录        |    119 |    118 | 噪声内                       |
| steady / 10_000 记录       |  1_749 |  1_259 | 见双峰说明，不构成稳定结论   |
| steady / 100_000 记录      | 17_864 | 18_498 | 见双峰说明，不构成稳定结论   |
| shrink → 1_000 记录        |    119 |    119 | 噪声内                       |
| alternate / 10_000 记录    |  1_229 |  1_186 | 约 -3%，三轮完全分离，可重复 |
| fresh_output / 10_000 记录 |  1_254 |  1_243 | 噪声内                       |
| fail_last / 10_000 记录    |  1_194 |  1_185 | 无差异（失败路径未改动）     |
| fail_last / 100_000 记录   | 11_978 | 11_925 | 无差异（失败路径未改动）     |
| retry / 100_000 记录       | 12_045 | 12_014 | 无差异                       |
| parking / 100_000 记录     | 29_063 | 28_470 | 停车采样主导，差异不归因提交 |

双峰说明：steady 10_000 / 100_000 的每进程 7 样本在本机呈双峰（快/慢模式约
1.5 倍差，推测为平衡电源方案下的频率波动）。快模式成本两侧相等：10_000 记录
before/after 各自最低簇均约 1_190 µs（快样本各 10/21 个）；100_000 记录两侧最低
样本分别 12_046 / 11_902 µs。中位数差值由双峰落点决定，不作为 A/B 收敛结论；
原始样本见 evidence，未删改任何不利样本。

分配计数（allocation 构建，每次完整调用）：steady、shrink、parking、retry 稳态
两侧均 0 次分配；grow、fresh_output、fail_last 两侧各 1–2 次（scratch 扩容或
错误包装），次数相同。冷启动路径 before 为 2 次/调用（scratch 与
`output.records` 首次各自扩容），after 为 1 次（output 直接接住 scratch
backing）。retained_build（全新配对连续三次调用到稳态）两侧每调用分配同为
2/3 次；区域累计 allocated bytes 即两侧 records backing 建立字节，两侧相同——
交换不改变双缓冲总保留量，只省去每批复制与冷启动的第二次 backing 扩容。

正确性：全部场景 oracle SHA-256 摘要 before/after 逐 case 完全一致（14 组，
analyze.ps1 强制校验）；每个进程在计时外断言记录数、身份顺序、header、token，
失败场景断言完整旧输出保持。

结论分类：**复制消除成立**（生产实现只交换所有权，指针级测试证明无逐元素
复制路径；分配计数与 oracle 全等支持）；**完整路径墙钟以差异接近噪声为主**
——本构建 `CanonicalPoseRecord` 为 40 B，成功提交逻辑复制量 10_000×40 B =
400 KB、100_000×40 B = 4 MB，按常见 memcpy 带宽量级估计仅占完整调用的约
1–2%，而采样插值占绝对主导，本机环境噪声（双峰 ±30%）高于该收益；
交替 output 场景呈现约 3% 的三轮完全分离改善，量级与移除每批复制一致；
未观察到可归因于交换的退化（fail_last/retry 与 100_000 档快模式两侧相等）。
不能用本结果宣称 Spatial 整体等比例加速；#712 的来源分配消除仍是独立成本。

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
