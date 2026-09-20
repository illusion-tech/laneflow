# 借用来源与 Adapter 缓冲复用 A/B 测量

关联 [#712](https://github.com/illusion-tech/laneflow/issues/712)。独立 workspace
研究程序，对照同一测量程序在两份生产源码上的来源读取与完整 Adapter 提取：

- **before（A）**：`b52f9ec4b792a158aed45f0ca4f536379073684f`（main，含 #719），
  `committed_pose_sources` 仍为按值 Vec；以 `legacy-source` feature 构建。
- **after（B）**：#712 栈（来源判定统一 + 借用迭代器 + Adapter 双候选缓冲 +
  容量/测后校验），默认 feature 构建。第四轮取证的提交脉络与 environment.json
  记录一致：`9eac374d`（稳态断言仅约束借用侧的修复）→ `298daed8`（移除旧
  证据目录，即各轮 environment.json 记录的取证 baseline / 取证时的 HEAD）→
  取证六轮 → `72ff8e92`（证据入库，取证后提交）。测量程序内容以各轮
  environment.json 的 src SHA 为准（汇总器强制 A/B 全等）；生产代码即
  712-1/712-2 两层提交。更早版本（`dc5a3685`/`1c02d477`/`63fbcd5a` 等）为
  历史取证轮次，见 git 记录，不作为当前有效结论来源。

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

## 结果（2026-09-19 第四轮取证；完整数值见 results.csv，环境见 summary-table.md）

稳态分配（allocation 构建，**逐样本断言**：B 侧 adapter_full/alternate 每个 32 次调用窗口均 0 分配/0 重分配，断言在程序内、暖机修复后不再有首窗重暖成本；A 侧按值 Vec 的每窗分配是被测基线现象，不套用该断言）：全部数据集稳态 1 → 0 次分配/调用；fresh_output 3 → 2；cold 4 → 3。**验收核心成立。**

数据集形态测前显式断言（同第三轮）：all_active 100% Active；mixed_parking 半 Active + 128 显式 Parked + 其余 virtual；high_completed 90% Completed + 10% Active（presentable=1000/10000）；sparse 100 Active + 其余 virtual。

oracle（v2 全字段）与测后校验：批次摘要含车辆序列、记录位模式、修订/frame/token、完整上下文与长度；来源摘要含有序完整序列。adapter 每计时样本结束后与 token 3 参考全字段一致；alternate 结束后 A、B 分别校验；fresh_output 计时外重放逐次校验。15 组 oracle A/B 逐键一致。

墙钟判读（口径不变）：分配消除与重分配消失证据成立；本轮 source_full 100 k 全 Active -17.5%、10 k -69.8%；adapter/alternate 各档涨跌互现（-18.5% ~ +12.1%），中位数差值不作稳定可重复幅度结论。

**容量实值轨迹**（`session.rs` capacity_tests，--nocapture 观测；元素 16 B PoseInput / 8 B VehicleHandle）：

| 步骤                | presentable | out.vehicles len/cap | pose 候选 len/cap | 车辆候选 len/cap |
| ------------------- | ----------: | -------------------- | ----------------- | ---------------- |
| 小批 4              |           4 | 4/4                  | 4/4               | 0/0              |
| 大批 68             |          68 | 68/128               | 68/128            | 0/4              |
| despawn 64 后小批 4 |           4 | 4/4                  | 4/128             | 0/128            |

缩量经合法生命周期（despawn）完成；大批 128 backing 缩量后轮换到 Session 车辆候选一侧（out 接回小 backing），断言按两侧最大值证明未释放、不误判为主动缩容；内容=保留的 4 辆、记录 ID 从 0 重新连续、旧尾部不可见。双 output 交替用完整快照（车辆+批次+上下文）双向比较；全新 output 首调用接住暖 backing、Session 接回空 backing 后下一批重建（#711 轮换合同）；失败保持旧输出内容与 backing、候选容量可继续用于重试。

retained 口径（Adapter 可观测范围）：输入候选 capacity×16 B + 车辆候选 capacity×8 B + 各存活 output 车辆 capacity×8 B，按观察点当时所有者统计、转移不重复计；不含 Spatial（#711 证据覆盖）、Runtime 或 allocator 元数据，不是进程 RSS。

## #718 组合观察（预集成，版本固定）

#718 在取证时未合并。组合验证基于 **#718 head `6bc440e8268f8d5f74ec7db07290fe4ac826a443`**
与本栈 `63fbcd5a` 的临时预集成提交 `e619371e`（throwaway，不入栈）。
`run-combination.ps1` 完整重现（失败即终止：每条 Git/Cargo 命令检查退出码；
过滤执行的测试断言实际通过数 ≥ 预期；`-SelfCheck` 验证损坏补丁被拒绝且失败
传播为非零退出；测试过滤器按目标拆分，不跨目标误过滤）：建 worktree → 固定
双 head 合并 → 应用 `combination-observation.patch` → 运行下列测试。最近一次
复现头 `b84f388f`（栈 `e3ae83a6` 前身 + 6bc440e8 + 补丁）。#718 再前进时按
增量影响重跑适用测试（不重跑不含 #718 的基础 A/B）。

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
