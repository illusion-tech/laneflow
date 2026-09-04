# Runtime Snapshot v5

`runtime-snapshot.fbs` 是当前唯一 LFRS schema，`format_version = 5`、
`runtime_state_version = 5`。必填 `world_policy` 保存闭合选择：`NotRequired = 1`
禁止携带 policy，`Pinned = 2` 必须携带 policy StableId；0 和未知 tag 拒绝。
策略内容由 LFCA origin 绑定，步长派生间隙在恢复时重新计算。
既有 Waiting traversal、semantic membership 与单调 admission counter 继续保存；
queue link、occupancy index、latest output 与 tick-local claim 不保存。

Conflict 持久状态按实施合同六分类闭合：每车可选的 Gate occurrence
`firstEligibleTick`、`Clearing` reservation owner/acquired tick、passage stable locator 与
exact route occurrence、committed downstream 物理区间，以及按 stream/zone StableId
严格排序的非 `NoHistory` lag 行。lag 行封闭为 `ActualClear | CutoverFloor`；缺行表示
`NoHistory`，因此 tick/time 0 不与 absent 混淆。occupant/cleared、frontier、tick-local
grant、内部 serial 和索引不入档；恢复从 reservation、整车位置与 passage 锚点重建并
整体校验，悬空 locator、重复/未来历史或不完整 authority 失败关闭。

downstream wire 行只含 `(lane_edge StableId, start_mm, end_mm)`，表示规范物理区间并集。
循环路线的同一物理边可由多个 route occurrence 合并，因而不保存含糊的 per-interval
route index；owner 由车辆记录给出，最小跟车间隙由 profile 派生。writer 与 reader 都从
reservation 的 exact Gate/passages、route、车辆全长和绑定根边长重建完整并集并精确
比较。wire 使用 StableId 规范序，lowering 后才按当前根 ordinal 排序。已清 cell 只能由
同地址 `ActualClear` 闭合，`CutoverFloor` 不冒充真实清空。

clean regeneration 使用固定 flatc 25.12.19：

```text
cargo +1.98.0 run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>
```

仓库只保留当前 reader/writer，旧版本输入明确失败关闭。
