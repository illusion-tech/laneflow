# #536 安全验收范围与 unsafe 审计

**文档状态**：Active；源码审阅与验证协议。运行结论以同一精确提交的执行回执为准。

本切片只增加安全验证 harness、现有工作流接线与审计证据，不新增生产 API 或格式。
执行协议、目标及已知覆盖边界见 [fuzz/README.md](../../fuzz/README.md)。
#536 的 G0/G2 记录见 [开工记录](https://github.com/illusion-tech/laneflow/issues/536#issuecomment-6504544294)。

## unsafe 例外清单

根 workspace 的默认规则为 `unsafe_code = "forbid"`。仅下列四个登记 crate 为例外；
`xtask check-wire-audit` 验证例外分类、源码载体、依赖钉版和隔离编译边界。
本表是安全前提的人工审阅，不以 token 数量替代正确性检查。

| 例外 crate                       | 审阅的安全前提与调用边界                                                                                                                                                                                                                               | 验证与限制                                                                                                                         |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `laneflow-road-editing-wire`     | 只承载钉版 flatc 生成物；包装器与生成源码分别做字节/摘要校验。生产消费者 `road_editing/reader.rs` 在访问 root 前检查 size prefix、标识、长度和 verifier 的深度/表数/apparent-size 上限，调用受检 `size_prefixed_root_as_road_editing_source_with_opts` | wire 审计与 Schema Codegen 重生成；LFRE fuzz。未把生成代码公开的 unchecked 能力当作安全准入入口                                    |
| `laneflow-runtime-snapshot-wire` | 同样只有钉版生成物；`admin/format_admission.rs` 执行大小/版本限制、受限 verifier，再进行 LFRS 语义、修订与来源核验                                                                                                                                     | wire 审计与 Schema Codegen；LFRS Published/Editable 种子、变异和 roundtrip；不宣称覆盖所有状态组合                                 |
| `laneflow-format-mmap`           | 唯一 unsafe 调用是只读 `MmapOptions::map`。backing 由私有临时文件创建，File 不外借；`seal(self)` 消耗写能力；映射前复核 exact length 和 usize 转换；映射只暴露只读切片                                                                                 | 六项本地测试覆盖 seal、长度、patch、Debug 和 Windows 禁止 reopen；Linux 分支由 CI 覆盖。同 UID 恶意代码/调试器内省明确在威胁模型外 |
| `laneflow-motion-kernel`         | `Kernel::for_backend` 先经 CPU/OS 特性检测再选 AVX2/AVX-512 函数表，字段私有；安全方法核对全部输入/输出列长，向量循环只处理完整组，尾部回标量；load/store 受切片边界与独占借用约束，`unsafe_op_in_unsafe_fn` 禁止隐式放宽                              | 12 项差异/拒绝测试覆盖所有 tail、mask、量化、unsigned 域、非有限值和短列零写；仅执行宿主支持的 ISA，不能把不支持后端视作已运行     |

审阅源：`crates/laneflow-format-mmap/src/lib.rs`、
`crates/laneflow-motion-kernel/src/{lib,x86,tests}.rs`、两个 wire crate 包装器、
上述两个生产 reader，以及 `xtask/src/wire_audit.rs` 的 pin/manifest/源码闭合规则。
生成代码不逐行声称人工证明，依靠固定生成工具、字节一致性和受检消费边界。

发现一处文档陈旧：mmap 模块仍自称“唯一登记的手写 unsafe 例外”，与已登记的
motion-kernel 不一致。本切片只修正此注释为“之一”，不改变映射或数值行为。
依赖本身、外部宏、工具链、CI 工作流与审阅权限仍是独立信任边界；不把本表写成
任意第三方代码或恶意 PR 的安全证明。

## 证据闭合方式

交付 PR 冻结 `H_pr` 后，六个目标在该 SHA 手工运行每目标 1800 秒并产生
`laneflow-security-fuzz-1` receipt。PR 与 Merge Group 的 300 秒 smoke 另列，
不能替代正式窗口，也不能把主线不同 SHA 的扫描拼成当前通过。

验收回执应绑定：

- 六个目标的实际源码提交、工具链、二进制、初末语料、执行量、时长和停止状态。
- 同 SHA 的 CI、Wire toolchain audit、Schema Codegen、CodeQL actions/rust 结果。
- 仓库扫描读取时点：active advanced setup、Secret Scanning 与 push protection、
  Dependabot security updates；三类开放告警及 push-protection bypass 的实际范围。
- `cargo-deny 0.20.2` 对根工作区和独立 fuzz 包的四类检查结果；已有 #935 限定
  advisory 例外照实保留，不用“零告警”抹去已有风险接受。
- 制品下载位置与独立 SHA-256 复核；失败/废弃运行的真实标签及原因。

最终验收以 [#536](https://github.com/illusion-tech/laneflow/issues/536) 的精确提交
回执与其运行制品链接闭合；本文件不随每次 CI 自动回写而制造新的待验收提交。
#305 选择不同的总验收提交时必须重新复核安全相关差异与适用检查。

## 首轮正式验收失败与修复（2026-10-30）

`b14f77a0` 的首轮六目标正式窗口（run `40933268900`，workflow_dispatch，1800 秒/目标）
在 `lfrs_restore` 开跑约 5 分钟时 panic：flatbuffers 25.12.19 的 verifier 只保证
vtable 落在缓冲区内且偶对齐，接受声明长度 0/2 的 vtable；恢复路径封闭性检查
`num_fields() = num_bytes()/2 - 2` 的派生减法随之下溢（vtable.rs:48）。其余五目标
通过；同 head 的 PR smoke（run `40933225763`）同因失败。最小化 crash 单元
`crash-9303b8943eeeb463f8aaf9bd7000b712b776935c`（15 字节，SHA-256
`9e0b84d175a5c20275d5b2c809fb9331a2930550242fbe352f505561b7119f57`）已逐字节嵌入
`lfrs_restore` 种子（`truncated-vtable`）与运行时定向回归测试；LFRE 预检 `closed()`
加同判守卫（当前 schema 的 required 字段使下溢在该入口不可达，属防演进兜底）。

失败运行六目标制品与回执已下载归档至工作树外本地目录
`E:/projects/worktrees/2933/fuzz-archive-536/failed-formal-40933268900/`
（含 SHA256SUMS；`lfrs_restore` zip 哈希与工作流上传日志宣布值一致）；GitHub
原始制品 90 天保留。修复提交另以新的六目标正式窗口重跑闭合。
