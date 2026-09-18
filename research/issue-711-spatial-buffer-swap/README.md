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
- 场景与单位：
  - steady（各规模稳定成功路径）、cold（全新配对首次调用）、shrink（100k
    暖机后回 1k）、alternate（两个 output 交替）、fail_last（末条失败）、
    retry（失败后重试）、parking、retained_build（全新配对连续三次调用到
    稳态，区域累计 allocated bytes 即两侧 records backing 建立字节）——
    单位均为 **µs/调用**。
  - fresh_output：每次调用使用**全新 output 的“创建—提取—释放”完整生命
    周期**，三者都计入该指标；完整结果对拍只在暖机与每样本结束时的计时
    区间外执行。单位 µs/调用（生命周期口径）。
  - grow：一次计时区间内连续完成 1k→10k→100k 三次完整调用，单位为
    **µs/增长序列**（3 次调用整段），不除以调用次数，也不代表单批成本。
- 计时：每 case 暖机 4 次、7 个样本；steady/shrink/fail_last/parking/
  alternate/fresh_output/retry 每样本 32 次调用（retry 为 16 组失败+重试），
  cold/grow 每样本 1 次且不暖机，retained_build 每样本 3 次调用。先按单位
  归一，再取每进程 7 样本中位数，最后取三进程中位数。
- 分配计数构建（`--features allocation`，`stats_alloc`）与正常墙钟构建分开
  运行；墙钟 CSV 的分配四列为占位零。分配计数同样按单位归一（retry 为
  失败与成功混合调用的平均值）。
- fixture 构建、输入准备、输出快照、完整对拍、摘要打印与文件写入不进入热
  路径计时；每个 case 在计时外验证记录数、身份顺序、header、token，并输出
  完整批次的 SHA-256 oracle 摘要（header + 记录身份 + 位模式级 pose 分量）。
- 测量循环对输入与输出使用 `black_box`；它是尽力而为的优化屏障，不构成
  “测量绝对可靠”的保证。
- 运行：固定单个 logical processor（≥17 核时固定第 17 个，否则最高位），
  未锁频。电源方案分阶段记录：full 阶段 powercfg 取证失败、未记录（各轮
  environment.json 中为空）；fresh 阶段六轮均记录为“平衡”（GUID
  `381b4222-f694-41f0-9685-ff5bb260df2e`）。记录到“平衡”不等于证明测量期间
  CPU 频率稳定，噪声与未锁频限制仍然保留。三组 A/B 墙钟进程按
  A₁B₁B₂A₂A₃B₃ 交错；分配构建每变体一次。详细环境见各
  `evidence/*/**/environment.json`；汇总器对同阶段各轮及 A/B 的工具链
  （完整 `rustc -Vv`，含 host 与 LLVM）、cargo、OS、CPU、逻辑核数与电源方案
  记录做一致性强制校验。

## 取证轮次

- **full 阶段**（`evidence/{before,after}/run1..3`）：2026-09-18 首轮全矩阵
  取证。A/B 测量程序 `main.rs` SHA-256
  `12B0BEFF…C5BB3`；B 侧生产基线为 `e6c524ce`。该阶段 fresh_output 的计时
  区间误含完整结果比较（每迭代 10_000 条全量 `assert_eq!`），该场景数值
  只作存在性校验、不进汇总；其余场景的比较均在计时区间外，不受影响。
- **fresh 阶段**（`evidence/{before,after}/fresh/run1..3`）：审阅修复后将
  fresh_output 的完整比较移出计时区间，用 `--only fresh_output` 定点重采
  （A₁B₁B₂A₂A₃B₃ 交错）。A/B 测量程序 `main.rs` SHA-256
  `ACBABC8E…E923E`；B 侧生产基线为 `3e8c4c03`（生产代码与 full 阶段字节
  相同，仅测量程序与脚本修复）。汇总中的 fresh_output 一律取自本阶段。

## 结果

数值由 `analyze.ps1` 从 evidence 自动汇总（含样本唯一性、逐轮 oracle 完整
性、分配样本、A/B 程序来源一致性校验；拒绝测试见 `test-analyze.ps1`，8/8
通过）。完整表见 [results.csv](results.csv) 与自动生成的
[summary-table.md](summary-table.md)；三轮中位数与全部原始样本在
[evidence](evidence/)。环境：AMD Ryzen 9 9955HX、Windows 29661、单 logical
processor 固定、**rustc/cargo 1.98.1（environment.json 取证记录；CI 门禁另用
1.98.0）**、release opt-level 3。

主要稳定场景（µs/调用，三进程中位数）：

| 场景                         |   before |    after | 判读                         |
| ---------------------------- | -------: | -------: | ---------------------------- |
| steady / 1 记录              |    0.116 |    0.106 | 噪声内                       |
| steady / 1_000 记录          |  118.872 |  118.419 | 噪声内                       |
| steady / 10_000 记录         |  1_748.6 |  1_259.3 | 见双峰说明，不构成稳定结论   |
| steady / 100_000 记录        | 17_863.7 | 18_498.1 | 见双峰说明，不构成稳定结论   |
| grow（每增长序列，3 次调用） | 14_374.1 | 13_611.1 | 序列口径；差异在噪声量级     |
| shrink → 1_000 记录          |  119.453 |  118.525 | 噪声内                       |
| alternate / 10_000 记录      |  1_229.4 |  1_186.1 | 约 -3.5%，三轮完全分离       |
| fresh_output / 10_000 记录   |  1_221.9 |  1_205.8 | 重采后噪声内                 |
| fail_last / 10_000 记录      |  1_193.8 |  1_185.4 | 无差异（失败路径未改动）     |
| fail_last / 100_000 记录     | 11_978.3 | 11_924.9 | 无差异（失败路径未改动）     |
| retry / 100_000 记录         | 12_044.6 | 12_013.5 | 无差异                       |
| parking / 100_000 记录       | 29_063.2 | 28_470.2 | 停车采样主导，差异不归因提交 |

双峰说明：steady 10_000 / 100_000 的每进程 7 样本在本机呈双峰（快/慢模式约
1.5 倍差；full 阶段电源方案未记录、fresh 阶段为“平衡”，无论哪种情形频率波动只是推测之一，未锁频限制保留）。快模式成本两侧相等：
10_000 记录 before/after 各自最低簇均约 1_190 µs（快样本各 10/21 个）；
100_000 记录两侧最低样本分别 12_046 / 11_902 µs。中位数差值由双峰落点决定，
不作为 A/B 收敛结论；原始样本见 evidence，未删改任何不利样本。同理，+3.55%
不足以证明交换导致稳定退化。

分配计数（allocation 构建，按各场景单位归一）：steady、shrink、parking 稳态
两侧均 0 次分配/调用；grow 为 2 次/增长序列；fresh_output、fail_last 两侧各
1 次/调用（scratch 扩容或错误包装），次数相同。retry 每样本 16 次失败调用
（错误包装各 1 次分配）加 16 次成功重试（0 次），平均 0.5 次/调用——这是
失败与成功混合后的均值，不是成功重试自身必定分配，也不是零分配场景。
冷启动路径 before 为 2 次/调用（scratch 与 `output.records` 首次各自扩容），
after 为 1 次（output 直接接住 scratch backing）。retained_build 两侧每调用
分配同为 2/3 次；区域累计 allocated bytes 即两侧 records backing 建立字节，
两侧相同——交换不改变双缓冲总保留量，改变的是首次调用的分配时序并省去每批
提交复制。

正确性：全部场景 oracle SHA-256 摘要 before/after 逐 case 完全一致（14 组，
full 与 fresh 阶段分别强制校验）；每个进程在计时外断言记录数、身份顺序、
header、token，失败场景断言完整旧输出保持。

结论分类：**复制消除成立**（生产实现只交换所有权，指针级测试证明无逐元素
提交复制路径；分配计数与 oracle 全等支持）。**完整路径墙钟以差异接近噪声
为主**——本构建 `CanonicalPoseRecord` 为 40 B，成功提交逻辑复制量
10_000×40 B = 400 KB、100_000×40 B = 4 MB；按常见 memcpy 带宽的数量级**粗略
估计**（非实测阶段占比），它只占完整调用的约 1–2%，采样插值占绝对主导，
而本机环境噪声（双峰 ±30%）高于该量级。alternate 场景三轮完全分离的约
-3.5% 与移除每批复制的量级一致，但属间接归因，不宣称已精确证明来源。
未观察到可归因于交换的退化（fail_last/retry 无差异；100_000 档快模式两侧
相等）。不能用本结果宣称 Spatial 整体等比例加速；#712 的来源分配消除仍是
独立成本。

## 复现

在仓库根使用 pwsh 7、已缓存依赖（本地默认工具链 1.98.1；CI 门禁另查
1.98.0）：

```powershell
cargo clippy --locked --offline --manifest-path research/issue-711-spatial-buffer-swap/Cargo.toml --all-targets --all-features -- -D warnings
cargo fmt --manifest-path research/issue-711-spatial-buffer-swap/Cargo.toml -- --check
cargo run --locked --offline --release --manifest-path research/issue-711-spatial-buffer-swap/Cargo.toml -- --smoke
# A 侧需另建基线工作树并复制本目录（未跟踪）后：
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/run.ps1 -Output research/issue-711-spatial-buffer-swap/evidence/before/run1 -AllowUntracked 'research/issue-711-spatial-buffer-swap/'
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/run.ps1 -Output research/issue-711-spatial-buffer-swap/evidence/after/run1 -AllowUntracked 'research/issue-711-spatial-buffer-swap/evidence'
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/run.ps1 -Output research/issue-711-spatial-buffer-swap/evidence/after/fresh/run1 -AllowUntracked 'research/issue-711-spatial-buffer-swap/evidence' -CaseFilter fresh_output
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/analyze.ps1
pwsh -NoProfile -File research/issue-711-spatial-buffer-swap/test-analyze.ps1
cargo test --locked -p laneflow-spatial -p laneflow-bevy --tests
```

run.ps1 在构建前、后及采集后要求 HEAD 不变且跟踪文件干净；evidence 目录必须
是新目录；binary 保留在取证目录但不入库（`.gitignore` 排除），其 SHA-256 记录
于 environment.json。测量源码提交后再取证；证据另行提交，不 amend/rebase
测量源码提交。analyze.ps1 拒绝不完整证据（缺 oracle、样本重复/缺失、分配样
本缺失、来源不一致、墙钟分配列非零），拒绝用例见 test-analyze.ps1。
