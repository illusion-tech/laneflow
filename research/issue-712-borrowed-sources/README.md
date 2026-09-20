# 借用来源与 Adapter 缓冲复用 A/B 测量

关联 [#712](https://github.com/illusion-tech/laneflow/issues/712)。独立 workspace
研究程序，对照同一测量程序在两份生产源码上的来源读取与完整 Adapter 提取：

- **before（A）**：`b52f9ec4b792a158aed45f0ca4f536379073684f`（main，含 #719），
  `committed_pose_sources` 仍为按值 Vec；以 `legacy-source` feature 构建。
- **after（B）**：#712 栈（来源判定统一 + 借用迭代器 + Adapter 双候选缓冲 +
  容量/测后校验），默认 feature 构建。各轮 environment.json 的 `baseline` 是
  取证时栈头（测后校验版取证时为 `63fbcd5a`）；测量程序内容对应
  `research/issue-712-borrowed-sources`（src SHA 见 environment.json 并由
  汇总器强制 A/B 全等），生产代码即 712-1/712-2 两层提交。早期版本
  （`dc5a3685`/`1c02d477` 等）为历史取证，见 git 记录。

两侧 `src/main.rs` 字节相同（SHA-256 记录于 environment.json 并由汇总器强制
A/B 全等）；`legacy-source` 是研究程序内的**薄版本适配层**——只切换
`for_each_source` 的消费写法（A：`as_slice()` 循环；B：直接消费迭代器），
`adapter_full` 与全部验证代码两侧逐字相同。生产 crate 不保留任何新旧开关。
A/B 都不包含 #718（基线两侧一致，不做跨成分相减）。

## 方法

- 合成修订：512 条互相平行的 2 000 m 车道（#681 布局，几何折叠在 canonical
  界内）+ 虚拟池设施 + 128 个显式泊位；进程内编译并经完整 emission check。
- 数据集形态（记录数口径）：全 Active（10 k / 100 k）、混合停车（10 k /
  100 k：半数 Active + 128 显式 Parked + 其余 virtual Parked）、高 Completed
  （10 k，推进至 ≥90% 完成）、稀疏可表现（10 k：100 Active + 其余 virtual
  Parked）。数据集准备不进入计时。
- 指标：`source_full`（完整消费全部来源，含成员判定；不是只构造迭代器）、
  `adapter_full`（`extract_committed_pose_batch` 完整调用：配对、单遍候选
  构建、Spatial 采样、成功提交）、`transform_convert`（提取后的位置转换，
  产品链路补充观察）、生命周期（cold：全新 Session+output 首次提取；
  fresh_output：每次换新 output；alternate：双 output 交替）。
- 计时：暖机 4、每样本 32 次调用、7 样本中位数 → 三进程中位数；墙钟与
  分配计数（stats_alloc）分开构建；A₁B₁B₂A₂A₃B₃ 交错取证；单 logical
  processor 固定；未锁频。环境由 environment.json 汇总，汇总器对同变体各轮
  与 A/B 的工具链（完整 `rustc -Vv`）、cargo、OS、CPU、逻辑核数、电源方案
  做一致性强制校验。
- 正确性：每数据集输出完整提取结果的 SHA-256 oracle（车辆句柄 + 位模式级
  记录 + 上下文）；汇总器逐日志强制完整 15 键 oracle 集合、样本编号唯一、
  A/B 程序身份与 feature 方向（A=legacy-source，B=默认）；拒绝测试
  `test-analyze.ps1` 6/6 通过。

## 结果（2026-09-19 重采；完整数值见 results.csv，环境见 summary-table.md；各轮原始样本在 evidence，未删不利样本）

稳态分配（每 32 次调用中位数，allocation 构建）：全部数据集 source_full / adapter_full / alternate **1 → 0 次分配/调用**；fresh_output 3 → 2；cold 4 → 3。**验收核心成立：暖机后来源不再逐帧分配，完整 Adapter 提取零新增分配。**

数据集形态测前显式断言：all_active=100% Active；mixed_parking=半数 Active + 128 显式 Parked + 其余 virtual Parked；**high_completed=90% Completed + 10% Active（分批在车道末端生成并逐拍完成，presentable=1000/10000，不再是上一版只有约 1% Completed 的错误形态）**；sparse=100 Active + 其余 virtual Parked。

oracle 完整性（v2 摘要）：来源摘要编入有序完整 `(VehicleHandle, PoseSource)` 序列（句柄 + 判别 + 全部字段 + 长度）；批次摘要编入车辆句柄序列、全部记录位模式、批次 header（修订 / canonical frame / placement token）、完整消费上下文（world id + world generation）与两个序列长度。字段敏感性有直接实证：同数据集下 adapter（token 3）与 alternate（token 8）摘要不同（上一版 v1 摘要二者相同，即复核指出的缺口）。15 组 oracle A/B 逐键一致。

墙钟判读（收紧表述）：本轮各独立轮次波动明显（本机已知双峰特征；电源方案为平衡已记录但未锁频）。source_full 100 k 全 Active 中位数 -47.5%、10 k -26.6%、混合 100 k -10.1%；adapter_full 与 transform_convert 各档涨跌互现（-5.6% ~ +29%）。**结论：分配消除与重分配消失（32 次分配 + 384 次重分配 → 0/0）证据成立；本机该轮观察到来源完整消费的改善，但独立轮次波动明显，中位数差值不作为稳定可重复的加速幅度**。上一下一轮采集（README 历史版本记录的 -6%~-7.6% adapter 档）同样按此口径理解。冷启动 -18.5%、fresh_output 大档 -33.7%/小档 -2.4%，方向与少一次/两次分配一致。

retained 与容量：分配事件数不是存活容量；同 Session 涨缩、不同容量 output 轮换与失败重试的容量轨迹专项证据**尚未交付**（遗留项，见 PR 未运行清单）。Spatial records backing 的轮换与容量行为由 #711 的 B 系列测试与证据覆盖；本切片新增的 Adapter 候选缓冲（输入 `PoseInput`、车辆 `VehicleHandle`、输出车辆）归 Session/调用方所有，逐批分配消失、retained 总量按所有者另行统计。

## #718 组合观察（预集成，版本固定）

#718 在取证时未合并。组合验证基于 **#718 head `6bc440e8268f8d5f74ec7db07290fe4ac826a443`**
与本栈 `63fbcd5a` 的临时预集成提交 `e619371e`（throwaway，不入栈）。
`run-combination.ps1` 完整重现：建 worktree → 固定双 head 合并 → 应用
`combination-observation.patch` → 运行下列测试。#718 再前进时按增量影响
重跑适用测试（不重跑不含 #718 的基础 A/B）。

补丁新增 Runtime 库内直接观察（`pose_source_observation_tests`，用 #718 的
`cfg(test)` 钩子，不经外部集成测试冒用私有入口）：

- **真实分发后成功读取**：`force_motion_dispatch` 强制 P5 走真实分发（工作集
  非空），`last_motion_dispatch_stats` 断言本次调度 `dispatched_chunks >= 2`
  （该统计每次调度覆盖，非历史累计）；随后完整来源序列与 worker=1 融合参照
  逐步一致。
- **本拍工作区已产生结果后失败**：`drop_motion_slot_at` 在 P5 暂存已产生后
  注入完成前沿缺口，协调器检出并整拍失败；来源序列与已提交状态摘要等于
  调用前。
- **失败后重试**：同初态另一世界直接成功；解除注入后的重试来源与摘要与
  直接成功完全一致（不是各 worker 臂互相相等）。
- Adapter 层完整批次观察（`pose_extraction_commit` 的混 frame 失败原子性/
  重试对拍与 `capacity_tests` 的交替快照）在组合树上通过；
  `preview_at_threshold_equivalence`（超融合阈值）与
  `pose_source_execution_equivalence`（worker 1–16）作为分发与等价背景通过。

以上即组合四项的直接见证；早期"叠加其上"式表述作废。

## 复现

```powershell
cargo clippy --locked --offline --manifest-path research/issue-712-borrowed-sources/Cargo.toml --all-targets -- -D warnings
cargo clippy --locked --offline --manifest-path research/issue-712-borrowed-sources/Cargo.toml --all-targets --features allocation -- -D warnings
cargo fmt --manifest-path research/issue-712-borrowed-sources/Cargo.toml -- --check
cargo run --locked --offline --release --manifest-path research/issue-712-borrowed-sources/Cargo.toml -- --smoke
# A 侧在基线工作树复制本目录后（features 必须 legacy-source）：
pwsh -NoProfile -File research/issue-712-borrowed-sources/run.ps1 -Output research/issue-712-borrowed-sources/evidence/before/run1 -AllowUntracked 'research/issue-712-borrowed-sources/' -Features legacy-source
pwsh -NoProfile -File research/issue-712-borrowed-sources/run.ps1 -Output research/issue-712-borrowed-sources/evidence/after/run1 -AllowUntracked 'research/issue-712-borrowed-sources/evidence'
pwsh -NoProfile -File research/issue-712-borrowed-sources/analyze.ps1
pwsh -NoProfile -File research/issue-712-borrowed-sources/test-analyze.ps1
pwsh -NoProfile -File research/issue-712-borrowed-sources/run-combination.ps1
cargo test --locked -p laneflow-runtime -p laneflow-bevy --tests
```

run.ps1 要求 HEAD 稳定、跟踪文件干净、evidence 目录为新目录；binary 不入库
（SHA-256 记录于 environment.json）。测量源码提交后再取证；证据另行提交。
