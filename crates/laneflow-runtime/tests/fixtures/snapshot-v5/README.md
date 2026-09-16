# LFRS 5 拒绝向量

这些字节由 `a29eaa4441fd8273e8a7768bfb931048fcc32753` 的生产 `encode_lfrs` 生成。
输入为 `admin::cutover::tests::transaction_tests::world_with_vehicle(true)` 的捕获结果，
仅将保存配置的 worker 字段分别设为 0、1、99，保留四项容量及 100 ms 固定步长。
生成时逐份用该提交的生产 `restore_lfrs`、原世界配置/根/来源完成恢复，三份均通过；
worker 0 因 FlatBuffers 缺省编码而没有 worker 槽。它们是拒绝测试输入，不是兼容 reader。

当前测试直接读取这些固定字节，并只改根 `format_version` 为 6 再验证拒绝；不使用
当前 v6 绑定冒充旧 writer。运行时不保留旧 schema 或转换路径。

| 文件             | 字节数 | SHA-256                                                            |
| ---------------- | ------ | ------------------------------------------------------------------ |
| `worker-0.lfrs`  | 568    | `121a9f2027f611d2807b5c87736c51593708bbfb9070a6e3f44df500165c684f` |
| `worker-1.lfrs`  | 576    | `2f11dbad57a1437009584d697c52871c010714908a25330d4251f0d9ba863d6d` |
| `worker-99.lfrs` | 576    | `cc3f0aa0a411fda65bb5b6bcd9e85db5268966a9972d3bb785c79bb64bd9ff34` |
