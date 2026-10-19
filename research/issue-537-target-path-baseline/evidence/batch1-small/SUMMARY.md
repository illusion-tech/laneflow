# #537 批 1 计数件回执（exact head 6cf90563，2026-10-19 23:02–23:03）

环境：LF-P100-REF-01（Ryzen 9 9955HX）、build 29683、BatteryStatus=2、平衡方案、rustc 1.98.0 `--release --locked`；计时窗外预编译，六件串行；用户机在用（计数件不受计时干扰）。全部硬断言通过（各自 `ok`）。原始输出见同目录 `*.txt`。与初值差异只作"建立当前基线"解释，不作回归比较。

## shared_network_allocation_evidence（#441 账本）

| 场景 | allocs/reallocs | allocated B | live B | retained B | lfca_exact |
|---|---|---|---|---|---|
| min-headless | 2/0 | 2,844 | 2,840 | 2,824 | 1,231 |
| full-spatial-omit | 247/18 | 12,797 | 7,819 | 7,803 | 31,677 |
| full-lane-spatial | 255/18 | 13,361 | 8,635 | 8,619 | 31,677 |
| corridor | 241/58 | 119,038 | 92,351 | 92,335 | 425,418 |

（worlds 2/8/32 每世界 live、held 候选账本在同文件原始输出内。）

## cutover_budget_evidence（同修订，切片 A 口径）

- 稳态 tick 32 拍：0 次分配 / 0 B（与初值一致，硬断言过）
- cutover 调用：456 次分配 / 30,936 B（初值 349/18,824）；净 live −25 B（与初值同）
- 共存双根 retained 逻辑字节 184,670（初值 176,220；corridor 夹具 420,332→425,418 漂移解释）

## cutover_cross_budget_evidence（跨修订，切片 C 口径）

- 日志算术：16 tick × (37 B 头 + 78 B×8 车) = 10,576 B ✓（硬断言过）
- Prepare：204 次分配 / 8,409,372 B、净 live +8,403,034（初值 154/8,397,066 ≈ 8 MiB arena）
- 共存输入量：journal 10,576 + arena 8,388,608 + target 根 retained 3,424 + LFSD 2,041（初值 2,852/1,673，夹具代际差异）
- drain+commit：58 次分配 / 18,938 B、净 live −8,403,840（旧世界随事务释放）

## snapshot_budget_evidence（published 五维之账本）

- LFRS exact 3,312 B（初值 3,248）
- capture 37 次 / 5,391 B、净 live 3,563（初值 34/3,435/2,955）
- encode 34+13 / 9,672 B、净 live 7,640（初值 34+12/9,480/7,448）
- restore 562+12 / 50,368 B、净 live 43,960（初值 386+13/23,240/18,256——v6 schema+夹具漂移，量级翻倍属预期重建路径变化）
- 保存前后稳态 tick 账本均 0/0 ✓（硬断言过）

## snapshot_peak_evidence（DHAT 恢复峰值）

- 恢复新增堆同时存续峰值 45,472 B / 418 blocks（初值 19,240/272）
- 返回时 live 44,152 B / 412 blocks（初值 17,920/266）
- 累计 50,944 B / 574 blocks

## shared_network_resource_evidence（常规 5 项 ledger）

七项全过（含 2 项失败面）；profile-frame-only：LFCA 1,656 / LFSM 1,797 / LFSD 1,075、identity retained 40、root retained 2,864、scratch_required 24、facility_geometry 0、lane_pose=false。四场景完整 SceneLedger 与共存项见原始输出。

## 待办（须用户窗口）

批 1 计时件 ×5（resource wall_clock / snapshot wall_clock / cutover wall_clock / cross wall_clock / instrumented 校准）；批 2（cutover_scale ×2、runtime_profile ×2）；批 3（新增最小入口 + cn-urban 10k/100k；1M 视资源）。
