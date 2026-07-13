# Proptest 回归样例

`vehicle_following_properties.rs` 发现失败时，会把最小化后的 seed 写入本目录。
回归文件应随修复一并提交，使后续本地和 CI 运行优先复现历史失败。
