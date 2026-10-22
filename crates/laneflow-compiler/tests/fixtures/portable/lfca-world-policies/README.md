# 显式世界策略夹具

`expected.lfca` 提供同一根内的两份策略，交换 stream priority、让行目标和 Gate 禁止项。
`signal.lfca` 加入明确右转方向与圆灯，`overflow.lfca` 用于所选策略间隙溢出的失败关闭。
三者由 `compiler/policy_tests/w3_shared_policy.rs` 经正式 Road Editing 输入生成。

`full-spatial.lfca`、`.lfsm`、`.lfsd` 在现有完整空间拓扑上加入一个显式测试策略，
由 `compiler/portable_fixture_tests.rs` 经正式 Synthetic 输入生成。策略明确覆盖两个
信号 Gate 及两个无信号 conflict Gate/stream，并与原 Access 的合成测试法规身份
`CN-test / 2026-01` 相容。该字符串是不透明的测试版本，不是 Runtime 公历或真实法规依据。

在环境变量 `DUMP_W3_POLICY=1` 下运行 `cargo +1.98.0 test -p laneflow-compiler --lib
--locked w3_shared_policy` 与 `cargo +1.98.0 test -p laneflow-compiler --lib --locked
runtime_full_spatial_policy_fixture` 可重生成；取消变量后测试核对检入字节。
Synthetic 贡献来源包含调用行列，修改来源后应先格式化再生成。
