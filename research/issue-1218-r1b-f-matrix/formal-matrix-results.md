# Issue #1218 正式矩阵运行记录（冻结构建）

**输入/生成器锚定提交**：`d28a8fbc8fc9918b38a814bf1a41916e71191f10`（分支
`research/1218-r1b-f-matrix`；10 身份正式输入与 manifest 的 `source_commit`/
`generator_commit` 均绑该提交）
**被测实现提交**（矩阵运行期记录）：F1/F3 = `4cb74402fe4ca9ab21eb4f803f5a1bd82ad9ba6a`（`bin4`/
`environment4.json`）；F2 = `0b49c172bcd5d6ff319c1a87e410eea7f0819298`（`bin5`/
`environment5.json`）。上述提交在 Model trailer 修正后重写，映射见 §8。
**运行性质**：正式协议运行（独立干净工作树冻结 release 构建、二进制复制到仓库外、
manifest 绑输入锚定提交、environment 绑各矩阵被测实现提交；中断/首错材料不拼接）
**边界**：结果仅为语义对照；不构成性能、恢复认证或产品认证（`Product TBD / Uncertified` 不变）。

## 1. 运行配置

| 项 | 值 |
| --- | --- |
| 构建工作树 | `git worktree add --detach` @ 4cb74402（frozen4）与 @ 0b49c172（frozen5），各 0 修改 |
| rustc / cargo | 1.99.1（58cf8f9c3 2026-10-13）/ 1.99.1（5f94df478 2026-08-27） |
| target | x86_64-pc-windows-msvc（Windows 11 专业版 Insider Preview 10.0.29695，AMD Ryzen 9 9955HX） |
| 构建命令 | `cargo build --release --locked -p laneflow-synth-harness --bin laneflow-synth-harness`；`cargo build --release --locked -p laneflow-synth-generator` |
| 测试二进制构建 | `cargo test --release --locked -p laneflow-runtime --lib --no-run --message-format=json` |
| 二进制 | `laneflow-synth-harness.exe` / `laneflow-synth-generator.exe` / `laneflow-runtime-test.exe`（SHA-256 记于 `environment4.json`/`environment5.json`，前后核对由矩阵入口执行） |
| 输入 | 10 个正式身份（10000/100000 × W1–W4/L1 × 16/33 ms），各 manifest 绑输入锚定提交 `d28a8fbc` |
| environment4/5.json | `implementation_commit` 分别绑 4cb74402 / 0b49c172、`source_clean=true`、`rustc_vv`、`cargo_version`、`target`、`build_command`、`test_build_command`、OS/CPU/采集时间 |
| 耗时校准 | 10000-W1-16 完整窗口（43,524 拍）单 worker 97.5 s |

## 2. F1 `worker-matrix`（跨 worker 确定性）

口径：10 身份 × 5 worker（1/2/4/8/16）× 3 全新进程轮 = **150 次运行 / 140 对比较**；
每对 = 回执仅排除 `workers` 后全字段相等 + `ticks.bin`/`events.bin` 逐字节（W3/W4 另
`commands.bin`/`batches.json`）；L1 行含双面检查点（T-free 64-hex 摘要 / T 触
`persistence_refused` + `t_state` 身份投影）。

**状态**：✅ **完整通过**（2026-11-03 14:32 核验）。`complete=true`，
**150/150 次运行、140/140 对比较、0 错误**；`implementation_commit=4cb74402`；
运行目录 `lf1218-formal7/F1`（构建 `bin4`/`environment4.json`）。
换道身份 L1 的双面检查点（T-free 摘要 / T 触 `persistence_refused` + `t_state`
真实身份投影）全部通过，跨 worker 回执与 `ticks.bin`/`events.bin` 逐字节相等。

## 3. F2 `worker-retry-matrix`（失败重试）

口径：6 同源正常参考 +（W3/W4 固定 `AfterTransitions` 4 身份 + L1 ×
{`AfterGrants`,`AfterTransitions`} 4「身份×断点」）× 5 worker × 3 轮 = **120 次重试 /
112 对比较**；失败零发布、retry/fresh 等价、L1 T 触时刻面 B 记录。

**状态**：✅ **完整通过**（2026-11-03 14:56 核验）。`complete=true`，
**6/6 正常参考、120/120 次重试、112/112 对比较、0 错误**；
`implementation_commit=0b49c172`；运行目录 `lf1218-formal8/F2`（构建 `bin5`/
`environment5.json`）。此前暴露的死锁、游标占位与 final outcome 缺陷均已修复，
`AfterGrants-10000-L1-16` 全部重试通过。

## 4. F3 `restore-matrix`（新进程恢复）

口径：10 参考 + 20 检查点 × 5 worker × 3 轮 = **300 次恢复**；每次对参考完整后缀
（轨迹/事件/命令）+ 末态 + 身份映射比较；L1 检查点为 T-free 切面（16/32 拍），
末态为面 B 记录（保存被结构化拒绝为期望行为）。

**状态**：✅ **完整通过**（2026-11-03 15:09 核验）。`complete=true`，
**10 参考 + 300 恢复 = 310 次、0 错误**；`implementation_commit=4cb74402`；
运行目录 `lf1218-formal7/F3`（构建 `bin4`/`environment4.json`）。恢复后缀
（轨迹/事件/命令）与末态比较全部通过，L1 检查点 T-free 切面与保存拒绝语义符合预期。

## 5. 与 dev 态切片的差异

dev 态切片（findings §7.3）为调试构建 + 主工作树（含未提交改动）+ 仅 `10000-L1-16`
身份的手工编排运行；本记录为冻结 release 构建 + 正式入口编排 + 全 10 身份全网格。
两者互不拼接；正式结论只以本文件为准。

## 6. 遗留与边界

- 首错停止/不补跑事实与中断登记见 §7；所有局部材料保留，不与正式结果拼接。
- 矩阵覆盖边界沿用方案稿 §9：L 族仅恰一横向过渡段；F2 仅 `AfterGrants`/
  `AfterTransitions` 两个注入断点单次注入；F3 检查点仅 T-free 切面（首 crossing
  前），含 T 状态保存/恢复/跨修订迁移在 S5 #1200 前一律以结构化拒绝为预期行为。
- 结果仅为语义对照，不构成性能、恢复性能或产品认证。

## 7. 中断、首错与死锁记录（2026-11-03）

> 本节记录中的提交 ID 为各次运行时的 ID；这些提交在 Model trailer 修正后已重写，
> 映射见 §8。

**第一次正式重跑（实现提交 d28a8fbc，目录 `lf1218-formal2/F1`）**：推进至 **43/150** 时
首错停止——`100000-L1-33-w1-r1`/`w2-r1` 在 `tick=1 rank=24` 触发运动学语义检查
（观测器 `motion_valid` 加速度上界硬编码 1.4 m/s²，L 族车型为 1.5 m/s²；dt=33 ms
合法 Δv=49 超过硬上限 48）。已完成 34/140 对比较全部相等；材料保留不拼接。
修复见提交 `f6629bc6` 与 findings §9。

**第二次正式重跑（f6629bc6，目录 `lf1218-formal3/F1`）**：因宿主计划关机，在 **50/150**
处主动停止（`complete=false`，0 错误、40/140 对比较通过）；材料保留。

**第三次正式重跑（f6629bc6，目录 `lf1218-formal4/F1`）**：因宿主关机被中止于 **7/150**
（0 错误）；材料保留。

**第四次正式重跑（f6629bc6，目录 `lf1218-formal5/`，F1+F2 并行）**：F1 推进至 **70/150**
（60 对比较、0 错误）后随修复停机；F2 在 4/6 正常参考完成后**死锁**（重试 0/120、
父进程空转 33 分钟、0 错误）。根因与修复见 findings §10 与提交 `537b98b0`。材料均保留。

**第五次正式重跑（537b98b0，目录 `lf1218-formal6/`，F1/F2/F3 并行）**：F1 至
**31/150**（0 错误）；F2 死锁修复生效（6/6 正常参考、重试启动），但
`AfterGrants-10000-L1-16` w1/w2/w4/w8 因报告游标占位（0）与观测（10403）不符
首错停止（10 重试、4 错误）；F3 参考阶段 5 个完成（0 错误）。三份材料均保留。
根因与修复见 findings §11 与提交 `4cb74402`。

**第六次（F2 单独重跑，方案 B，实现提交 `0b49c172`，目录 `lf1218-formal8/F2`）**：
final 观测缺终态步进结果首错（见 findings §12）后，以 `bin5`/`environment5.json`
全新重跑；修复只触及 runtime 测试目标（不进入 harness 二进制），故 **F1/F3 继续
使用 `bin4`（`4cb74402`）在 `lf1218-formal7/` 跑完**，结果文档按矩阵分别登记
implementation commit（F1/F3=`4cb74402`，F2=`0b49c172`）。

**最终完成（2026-11-03）**：F1（4cb74402，`lf1218-formal7/F1`）150/150、140/140、
0 错误，14:32；F2（0b49c172，`lf1218-formal8/F2`）6/6 正常参考、120/120 重试、
112/112、0 错误，14:56；F3（4cb74402，`lf1218-formal7/F3`）10 参考 + 300 恢复、
0 错误，15:09。三矩阵均 `complete=true`，最终结果见 §2–§4。

**冻结环境汇总**：`bin4`+`environment4.json`（F1/F3，`4cb74402`）；
`bin5`+`environment5.json`（F2，`0b49c172bcd5d6ff319c1a87e410eea7f0819298`）；
输入与 manifest 来源/生成器提交保持 `d28a8fbc`。旧环境 `bin2`+（f6629bc6）、
`bin3`+（537b98b0）保留作中断证据。

## 8. 实现提交标识与 Model trailer 修正（2026-11-03）

矩阵运行时绑定的实现提交（`environment4/5.json` 与各 `summary.json` 记录）：

- F1/F3：`4cb74402fe4ca9ab21eb4f803f5a1bd82ad9ba6a`
- F2：`0b49c172bcd5d6ff319c1a87e410eea7f0819298`

上述提交在矩阵完成后按 `docs/reference/commit-convention.md` §6.1/§214 修正
`Model` trailer（原误记为上一会话的 `xiaomi/mimo-v2.6-pro-ultraspeed`；本会话
实际模型为 **LongCat-2.5**，官方 2026-11-02 GA，与 2026-09-25 的
LongCat-2.5-Preview 为不同版本）。修正仅变更提交消息、源码树不变，重写后对应：

| 运行期实现提交 | 重写后提交 | tree 哈希（两者相等） |
| --- | --- | --- |
| `4cb74402fe4ca9ab21eb4f803f5a1bd82ad9ba6a` | `0541f1daeb6bb66eb3ea41a7b21ebbefc8192c2e` | `838665977085b5a4d16482b30d5f2e592f81b441` |
| `0b49c172bcd5d6ff319c1a87e410eea7f0819298` | `f9b5476c74073aade94cfcb9ffbaf4f06b7e97eb` | `4d123518ddc26ab82b79afe88c1ebedba6334f03` |

校验：`git diff --stat <old> <new>` 均为空，`git rev-parse <old>^{tree}` 等于
`<new>^{tree}`——被测源码状态可唯一核验；原实现提交 ID 作为运行期记录保留在
environment/summary 中。

**模型登记**：`meituan/longcat-2.5` 已按 §6.1 在
`xtask/data/model-registry.json` 的 `local_models` 登记（来源：LongCat 官方
change-log），并向上游 models.dev 汇报补录。
