# Runtime Snapshot v5

`runtime-snapshot.fbs` 是当前唯一 LFRS schema，`format_version = 5`、
`runtime_state_version = 5`。必填 `world_policy` 保存闭合选择：`NotRequired = 1`
禁止携带 policy，`Pinned = 2` 必须携带 policy StableId；0 和未知 tag 拒绝。
策略内容由 LFCA origin 绑定，步长派生间隙在恢复时重新计算。
既有 Waiting traversal、semantic membership 与单调 admission counter 继续保存；
queue link、occupancy index、latest output 与 tick-local claim 不保存。

clean regeneration 使用固定 flatc 25.12.19：

```text
cargo +1.98.0 run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>
```

仓库只保留当前 reader/writer，旧版本输入明确失败关闭。
