# Runtime Snapshot v4

`runtime-snapshot.fbs` 是当前唯一 LFRS schema，`format_version = 4`、
`runtime_state_version = 4`。v4 在 v3 行为配置与 Parking 状态基础上加入 WaitingZone
traversal、semantic membership 和单调 admission counter；queue link、occupancy index、
latest output 与 tick-local claim 继续由 Runtime 重建或丢弃。

clean regeneration 使用固定 flatc 25.12.19：

```text
cargo +1.98.0 run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>
```

仓库不保留 v3 reader、双写、迁移 shim 或转换器。旧 `format_version = 3` 输入由当前
reader 明确失败关闭。
