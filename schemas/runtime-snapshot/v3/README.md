# Runtime Snapshot v3（`LFRS`）

这是当前唯一 production 运行时快照 wire schema。容器采用 size-prefixed FlatBuffers、
file identifier `LFRS`、`format_version = 3` 与独立的
`runtime_state_version = 3`。clean regeneration 使用固定 flatc 25.12.19：

```text
cargo +1.98.0 run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>
```

v3 的 `WorldConfigBinding` 保存车辆、路线、路线边出现项与路线冲突出现项四项容量；
`ConflictPassageOccurrence`、Gate ranges、最终 clearance、occupancy index 与其它派生热表
均由稳定路线输入和目标共享根重建，不进入容器。

车辆的可选 `parking` 仍是 tagged binding：

- `Reserved + ExplicitSpace`：ParkingSpace StableId 与 entry route occurrence；
- `Reserved + VirtualPool`：ParkingFacility StableId、entry route occurrence，以及精确
  `(LaneEdge StableId, progress_mm)` semantic entry；
- `Occupied + ExplicitSpace | VirtualPool`：tagged target StableId；
- 无 binding：`parking` table 缺席。

owner-local virtual selector、密集 ordinal、runtime handle/slot/generation、共享静态数组与
派生 pose 不入容器。Reader 在确认 v3 两条版本轴后执行 closed-shape vtable field-count
检查；旧 v2 与未知版本均拒绝，不提供兼容 reader 或迁移 shim。
