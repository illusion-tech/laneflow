# Runtime Snapshot v2（`LFRS`）

这是 #541 后唯一 production 运行时快照 wire schema。容器采用 size-prefixed
FlatBuffers、file identifier `LFRS`、`format_version = 2` 与独立的
`runtime_state_version = 2`。clean regeneration 使用固定 flatc 25.12.19：

```text
cargo +1.98.0 run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>
```

根表字段 id 仍连续为 `0..=16`。车辆表的可选 `parking` 是 tagged binding：

- `Reserved + ExplicitSpace`：ParkingSpace StableId 与 entry route occurrence；
- `Reserved + VirtualPool`：ParkingFacility StableId、entry route occurrence，以及精确
  `(LaneEdge StableId, progress_mm)` semantic entry；
- `Occupied + ExplicitSpace | VirtualPool`：tagged target StableId；
- 无 binding：`parking` table 缺席。

owner-local virtual selector、密集 ordinal、runtime handle/slot/generation、共享静态数组、
occupancy index 与派生 pose 不入容器。Reader 在确认 v2 两条版本轴后执行 closed-shape
vtable field-count 检查；状态矩阵、target kind、stable ID、Reserved route ownership、exact
entry 闭包/前向可达、显式排他与虚拟容量守恒均在 staging world 内验证。
