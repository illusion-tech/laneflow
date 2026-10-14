# #285 复杂路口有界性能程序退役归档

Refs #926。冻结主线 `ec934c4f56707f101f8bb172ddc075b0031a43d1`，源码树
`e355e0a9ddd02f0dc45916317ee34605abdfdd88`。#285 有界性能报告的 4 个 Bevy example
入口、4 个私有支持模块与 4 个 PowerShell 冻结／运行脚本退出当前源码树，共 12 个
文件、197616 个 Git blob 字节。它们是一次性证据程序，不是 Adapter 用法示例；
example 目录只保留演示与 smoke 用途（#925）。

本次不另发 Release 制品：退役文件全部可从上述冻结提交按 Git blob 恢复，下表绑定
大小、SHA-256 与原 blob 身份。
[`research/issue-285-junction-scale/README.md`](../issue-285-junction-scale/README.md)
保留协议、测量口径与预算结论；其中的 freeze/run 命令只在冻结提交内有效。

## 退役文件

| 原路径                                                               | 字节数 | SHA-256                                                            | Git blob                                   |
| -------------------------------------------------------------------- | -----: | ------------------------------------------------------------------ | ------------------------------------------ |
| `crates/laneflow-bevy/examples/junction_scale.rs`                    |    220 | `8567fce7640a6ede9e35d9d7f68dcda22e0370b9439bb937f25e06fdbcc69402` | `abc741c7517d2400526f70551356af3a4118725e` |
| `crates/laneflow-bevy/examples/junction_scale_allocation.rs`         |    338 | `34202ffdce4ebe561f2b178602ab871376113768afd050952f1cf8f9a7446861` | `0be15ecfb1ff60062a26c5c1efc6d8e15b29ab09` |
| `crates/laneflow-bevy/examples/junction_scale_analyze.rs`            |  29939 | `77a8af350e58d9ae7e1596db401d718d13f16a3404120727877110b5c9ed55fe` | `773f938fb2e5a237f8138cd9839f1235108d1a0b` |
| `crates/laneflow-bevy/examples/junction_scale_render.rs`             |    221 | `45d637d767926b325c5d3593841c96677d37301289f423e59661d38faa1e1d09` | `6f201cda18cb6781eae770d62e8ec763e0cccbfb` |
| `crates/laneflow-bevy/examples/support/junction_scale.rs`            |  44370 | `a4b509cb1ced4916d9980392ed013d6a3d67c748dbc6fadea4e24f43388314b0` | `c53f578d3629b37aa03ab63d0a58d91b78e9858e` |
| `crates/laneflow-bevy/examples/support/junction_scale_geometry.rs`   |  15176 | `dc6b9c916860da1c2afa7731a75065c3dd1a3ad08aca617c652ad2b62d4ce3f4` | `8fa35be9aec6b27e315194418785c24e9b0e904c` |
| `crates/laneflow-bevy/examples/support/junction_scale_native.rs`     |   4486 | `3355a1020d3dc1998a6526f2d1e10ba8daffd6e4a29df0ef8875c4de6707daf5` | `b34a9d1e2fcee208957ced19f79bd1ff878b7b15` |
| `crates/laneflow-bevy/examples/support/junction_scale_validation.rs` |  81768 | `e8e4f288c79269329182d77698e61f429072725856881d0980626064510657c7` | `62f8b9400029021c909dda445ce4c3b267de66d2` |
| `research/issue-285-junction-scale/environment.ps1`                  |   3331 | `88a14fdfb9fa0e79b92f2aac785d3f07e2e941d238899dc8e19aeb7042116ca3` | `62cc205d9ed05d5f7f396fe7dcf448a74f4e84dd` |
| `research/issue-285-junction-scale/freeze.ps1`                       |   6671 | `2d227808d73408b2fb283c2d01124ca202aa90866988228843445545ef0d0832` | `0faa3a9245ed3188c39e4e0cbe01752a0de6b827` |
| `research/issue-285-junction-scale/process-memory.ps1`               |   1342 | `a7e76412c0ae4eb4872dd2d8c1790643a8830a57a7495225608d4bb4f4123471` | `bc885708bef3c0f47a7d70a64a1b94507e7d54a9` |
| `research/issue-285-junction-scale/run.ps1`                          |   9754 | `b0fa829c53debc9c76bcc425b84a46781aa026fc196cbd21ad62827eba3152ac` | `1f87569d7c321c60bc39af9f9436a4d038bd0700` |

`laneflow-bevy` 同步移除仅服务这些程序的 `laneflow-junction-generator`、`serde_json`
dev-dependency、`junction_scale_render` example 声明与 CI 编译项；生成器 crate 本身
保留。

## 恢复与边界

需要复跑历史协议时，在仓库外新建工作树并检出冻结提交，再按该提交内的 README
执行：

```text
git worktree add <新的工作树目录> ec934c4f56707f101f8bb172ddc075b0031a43d1
git -C <新的工作树目录> cat-file -p <Git blob>
```

既有长测结果包、二进制与环境转储仍由 #285 原报告的身份约束；此次没有重跑试验，
也没有把退役程序迁入 `tools/`。若以后需要持续的路口规模基线，应另立 Issue 设计
正式的性能工具，而不是恢复这些 example。
