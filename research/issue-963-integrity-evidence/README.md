# 共享路网认证注入与城市短时浸泡

本研究回答 [#963](https://github.com/illusion-tech/laneflow/issues/963) 的首轮问题：
现有认证、宿主存档、独立进程重放与写后读回能否连成有完成性边界的取证链路，
以及新增注入是否保持 [完整性设计](../../docs/design/traffic-runtime-state-integrity.md)
§2/§4/§5 的失败分类。研究使用 Rust 测试目标，不增加生产 API、默认运行中扫描或新的摘要算法。

后续已补入时序、在途步进、实际整数页及长期取证记录；五项验收的证据入口和剩余缺口
汇总见[逐项验收证据](acceptance-evidence.md)。

## 仪器与覆盖单位

[城市驱动](../../tools/laneflow-urban-harness/src/integrity_soak.rs) 以 `cfg(test)` 编译，
消费 `cn-urban.toml` 生成的 fixture（名义 2,000 辆车），运行 `SUSTAINED-ACTIVE` 的
实际完成后回收需求。每个 Runtime 使用一个工作线程；原世界以 16 ms 固定步长推进，
宿主把每拍墙钟间隔至少留到约 16 ms。写存档、认证及等待子进程期间原世界驻留但不推进。
本仪器有观测、哈希、读写和人为节奏控制，数据不能用来评价 tick 性能。

初始化完成一拍后，从 tick 1 开始留档；这一拍之前不计审计覆盖。每段最多推进
1,024 拍，在起点、64 拍边界和终点取得 `(command_cursor, tick, digest)`。每段：

1. 用既有城市存档保存 Runtime 与宿主耐久状态；三个存档文件直接复用 #954 的
   `verified_write`，没有另一套重试或读回实现。
2. 父进程推进原世界，周期认证其持有的同一共享修订，并保存完整期望检查点。
3. 启动当前测试二进制的独立子进程。子进程先用初始化时冻结的发布来源身份认证修订，
   只有 `Pass` 后才恢复存档并重放到父进程指定的终点。原世界不会在同一进程内恢复复制。
4. 直接复用 #960 的 `laneflow_audit_test_support::compare`，先判完成性，再比较
   全部计划序号与检查点。另比对宿主状态摘要和整段逐拍记录流摘要（含命令、事件、观测）。
5. `CompleteMatch` 才累加匹配覆盖。`Diverged` 留下完整不一致结果后停止；失败、
   缺点、未到终点、子进程超时均不计匹配覆盖。每段开始即写 `Incomplete / audit pending`，
   只有收到完整审计结果才更新；外层中断不把前面成功段的结论借给未完成段。

父子进程还各自生成 64 KiB 确定性文件探针；子进程独立生成期望，核对父进程写出的探针，
再写自己的探针。探针也复用 #954 的双路径读回。它们只能说明同一文件 API 与缓存下的
可见字节一致，不是绕过 LaneFlow 的独立环境归因实验，也不证明物理持久化、断电耐久或
真实物理页没有损坏。发现失配时须按 `validation-matrix.md` 补独立环境 A/B 对照，
不能直接归因 Runtime 匿名堆。

单次时长必须为 1–1,800 秒；证据目录必须不存在；子进程超时为 30 秒，证据目录在
每段前检查 512 MiB 阈值。采集机外层看护每秒检查同一归档二进制的父子进程合计
工作集与私有字节，超过 2 GiB 或总时长超过约定值加 180 秒即停止并记 `Incomplete`。
这个阈值是采样停止条件，不是分配器硬限额；采样峰值只是观测下界。

## 注入覆盖清单

[新增矩阵](../../crates/laneflow-static-network/src/attestation_matrix_tests.rs) 使用线程局部、
测试专用的阶段读取视图与复算前钩子。实际 reader、builder、比较、来源复核和分类继续执行。
替换视图由测试独立拥有，不制造不可变借用的可变别名；不把这类逻辑注入说成真实 OS 页故障。

[发布来源时序矩阵](../../crates/laneflow-static-network/src/attestation_source_timing_tests.rs)
进一步在真实 reader 接受后替换 builder 的重读视图，并让来源复核对独立拥有的阶段字节
真实计算 digest / exact length。builder 重读处的 panic 为受控阶段注入；闭合失败则使用
重新登记 chunk 摘要与修订声明的 LFCA，经过真实 reader 与 builder，不直接构造失败结果。
这些测试不证明 immutable backing 被实际改写，也不覆盖 OS 物理页故障。

| 场景                                     | 证据入口                                                                                                                                                                                                      | 本轮判据与边界                                                                                                                       |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `origin` 的六个静态契约版本轴            | `every_static_contract_origin_field_is_rejected`                                                                                                                                                              | 六轴分别失真均 `Mismatch / Indeterminate / DisableRoot`；修订号与分区提示派生版本由既有测试覆盖                                      |
| `origin` digest、length 及二者同时失真   | `origin_digest_length_and_combined_distortions_are_declared_misses`                                                                                                                                           | 均 `Pass`，明确记为设计 §5 的声明漏检面                                                                                              |
| 切换描述符字节身份                       | Runtime `origin_mismatches_fail_closed`                                                                                                                                                                       | digest、length、二者同时失真，在 base/target 两侧均拒绝；与认证字节身份漏检按两个入口分别验证                                        |
| 修订瞬时失真、复算失配位置变化           | `transient_root_and_moving_mismatch_are_untrusted_through_real_recompute`                                                                                                                                     | 真正执行两次比较，保留首次位置并判 `Untrusted`                                                                                       |
| 修订失配后复算分配失败、取消             | `root_mismatch_is_not_erased_by_recompute_allocation_failure_or_cancel`                                                                                                                                       | 在复算前触发既有分配钩子或取消标志，均保留首次证据并判 `Untrusted`                                                                   |
| 来源首次失败后恢复、复算证据变化         | `source_transient_and_different_failures_use_real_reader_results`                                                                                                                                             | 分别为带瞬时诊断的 `Pass` 与 `OperationFailed(SourceEvidenceUnstable)`                                                               |
| 两侧同值零列                             | `common_cause_equal_zero_columns_are_a_declared_comparison_miss`                                                                                                                                              | `Pass`，声明漏检；只证明比较器不能区分相同损坏内容，不证明物理共页已注入                                                             |
| 派生列零值、替换、截断                   | `zero_replaced_and_truncated_derived_columns_are_rejected`                                                                                                                                                    | `Mismatch / SharedStaticRoot / DisableRoot`；列小于一页，不计真实 4 KiB 根页覆盖                                                     |
| 发布来源零页、页替换、正负一页位移、截断 | `published_source_page_patterns_are_source_failures`                                                                                                                                                          | 30,400 B LFCA 上按 4,096 B 对象内偏移注入，五种均 `Mismatch / Source / IsolateSource`；不是内存地址页对齐证明                        |
| 安装/恢复前认证门禁与独立进程重放        | `short_chain_replays_in_an_independent_process`                                                                                                                                                               | 真正启动成功与错误引用两个子进程；错误引用在认证处退出，不产生成功响应                                                               |
| 验证后、builder 重读前来源变化           | `repeated_post_validation_source_change_is_source_mismatch` / `transient_post_validation_source_change_passes_only_after_full_recompute` / `different_post_validation_changes_preserve_both_source_evidences` | 同证据为 `Mismatch / Source / IsolateSource`，瞬态完整恢复为带诊断 `Pass`，异证据为 `SourceEvidenceUnstable`；核对完整摘要与长度证据 |
| 修订侧首次证据后，复算来源变化           | `established_root_evidence_survives_recompute_source_change`                                                                                                                                                  | 保留首次修订证据，`Untrusted / Indeterminate / DisableRoot`，不被后续来源问题撤销                                                    |
| builder 消费期间受控 panic               | `panic_during_builder_reread_stays_on_source_side`                                                                                                                                                            | 在第 4 次重读处注入；同证据为 `Mismatch / Source / IsolateSource`，瞬态完整恢复为带诊断 `Pass`                                       |
| reader 通过、builder 身份闭合失败        | `reader_accepted_builder_closure_failure_precedes_root_comparison`                                                                                                                                            | 真实 reader 接受，builder 报 `UnexpectedOrdinal`；来源失败优先于修订比较，健康与已失真修订两种输入均不停用修订                       |

既有测试继续覆盖不同修订、自洽重发布、引用失真、reader 限额、builder 预算、首次分配失败、
取消、来源准备/修订比较 panic、正负零、NaN、失配证据身份和遥测失败。
#954 的写入注入矩阵继续在原模块执行；这里调用其交付能力，没有复制 `ArtifactIo` 注入机制。

时序矩阵发现并修复了复算中未再次复核来源的分类缺陷：每次尝试独立记录已准备的字节身份，
每次修订比较失配都先复核本次来源，再按首次失败阶段归类。相同来源变化不再被复算中的
修订比较失配误判为应停用修订的 `Untrusted`；已成立的首次修订侧证据仍保持失败关闭。

实际整数 backing 的完整虚拟页覆盖见下文；上表中的阶段视图本身不计这项覆盖。
以下仍不作为已覆盖项：物理页帧与物理共页，以及来源 immutable backing 在验证期间被实际改写；
匿名堆自然失真的对象、频率及归因，以及设计 §9 所需的运行中失真证据。
当前主机的 24 小时取证留档已完成；主机历史背景及适用范围见对应报告，
二期准入尚未满足。编辑来源仍随 #529，未混入发布来源验收。

## 扫描后与瞬态变化

[扫描时序矩阵](../../crates/laneflow-static-network/src/attestation_scan_temporal_tests.rs)
核对设计 §4.2 的逐段观测时刻与 §5 第 8 / 17 条声明漏检面。比较器读完车道长度列后，
仅在测试中把后续 traffic 列的实际侧切换到另一份独立拥有的修订；两份修订只在指定列不同。
队列消费断言证明切换点已执行，未读列变化的对照必须经过真实两次比较及来源复核。

| 场景                         | 证据入口                                                    | 结果与边界                                                                                                                                                                |
| ---------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 已读长度列在后续扫描阶段变化 | `already_read_column_is_not_revisited_in_same_scan`         | 当前轮 `Pass`；下一轮从变化后视图起扫，同一变化得到 `Mismatch / SharedStaticRoot / DisableRoot`                                                                           |
| 尚未读取的限速列变化         | `unread_column_change_is_detected_through_both_comparisons` | 两次真实比较检出同一列、序号、期望与实际值；`Mismatch / SharedStaticRoot / DisableRoot`                                                                                   |
| `Pass` 返回后修订变化        | `returned_pass_does_not_cover_later_revision_changes`       | 前次报告仍为该次 `Pass`，不随之后变化更新；下一次认证检出本例的长度列变化                                                                                                 |
| 来源构建期间 `A → B → A`     | `source_aba_is_not_identified_by_equal_endpoint_hashes`     | reader 读 A，首次 builder 消费 B，来源复核重算 A；首次修订比较证据在完整复算 A 后不再复现，返回 `Untrusted / Indeterminate / DisableRoot`，没有 `SourceBytesChanged` 诊断 |

最后一项说明首尾 digest / exact length 相等不能识别构建全过程的来源变化，也不代表
此类变化必定返回 `Pass`。本例留下不可复现的修订侧证据，按现行规则保留首次证据并失败关闭；
不能把结果写成来源变化已被识别，或据此断言修订 backing 已损坏。
返回后变化使用借用结束后的唯一 `Arc` 安全修改；其余阶段视图也不改写实际不可变 backing。
这些是已声明观测边界的逻辑回归，不证明真实物理页故障或所有瞬态变化均已覆盖。

## 在用审计与在途步进

[并发处置测试](../../crates/laneflow-runtime/src/admin/tests/audit_step_overlap.rs) 在真实 `step`
成功准备、尚未提交的位置设置一次性同步点。宿主收到到达通知后完成真实认证调用，再按
处置提示裁决新调用，最后释放原 `step`。通知与释放均有 30 秒失败界，异常退出自动释放，
没有通过睡眠或多次碰运气取得重叠。同步点及其线程间搬运仅进入 `cfg(test)`。

四个用例各覆盖一个工作线程与两个工作线程；池内场景同时核对调用线程与实际执行线程不同。
两个世界与一个 Spatial 会话持有同一修订 `Arc`：

| 真实认证返回                             | 证据入口                                                           | 宿主处置与在途调用                                                                               |
| ---------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------ |
| `Pass / None`                            | `pass_audit_overlaps_step_without_stopping_consumers`              | 新调用继续；在途 step 正常提交并发布结果                                                         |
| `Mismatch / Source / IsolateSource`      | `source_mismatch_isolates_input_while_in_flight_step_finishes`     | 损坏 LFCA 被隔离；两个世界、观测导出和 Spatial 新调用继续                                        |
| `OperationFailed(BuilderBudgetExceeded)` | `operational_failure_retries_without_stopping_consumers`           | 仅记录重试；已安装修订继续使用                                                                   |
| `Mismatch / Indeterminate / DisableRoot` | `disabling_audit_stops_new_calls_and_quarantines_late_step_output` | 停止两个世界、观测导出与 Spatial 新调用；原 step 仍提交并返回，晚到 `StepOutcome` 由测试宿主隔离 |

停用场景在原 step 返回后继续阻止其下一拍，未调用 Runtime 取消或排空入口，也未在修订中
加入失效标志。结论前已发布的非空事件、观测和位姿批次仍保留在宿主，未被撤销。
该宿主仅展示单一协调器对新调用的裁决与输出隔离，不是产品宿主调度器；不证明任意宿主的
多发起者并发协议。停用场景把来源与引用一起换成另一份自洽输入，得到真实的无法判因
`Mismatch`；没有把它标成真实修订页失真或 `Untrusted` 注入。上述具体故障下的重叠仍须
各自的故障取证，本矩阵只证明这里列出的四种返回与调用边界。

## 实际整数内存页注入

[页级取证底座](page-injection.md) 从健康、真实构建的整数列中选择完整系统虚拟页，
只在唯一拥有且没有活动借用时安全改写，再调用真实认证。修订组与普通整数控制组各六种
请求，由独立子进程执行，父进程复核实际地址、完整读回、逻辑失配和内容指纹。
这补充了建根后、投入使用前的实际整数页覆盖，不证明物理页帧、物理共页、在途消费者与
具体页故障重叠，或故障机自然失真。完整方法、失败标签与剩余边界见该记录。

## 2026-10-24 首轮采集

浸泡源码冻结为 `445f75a4ba3415a0813b2c71c35f157491127ba1`，Rust 1.99.1
（LLVM 23.1.1，`x86_64-pc-windows-msvc`）。运行二进制 SHA-256 为
`4d588ca0f338d97b8c4d66d96bc87220d485e08f89de38f27cff96790e69641b`。
这次运行在新增认证注入矩阵之前冻结；矩阵钩子只进入 static-network 自身单元测试。

冻结版在初始化时从已经过城市制品/目录核对的根提取一次引用，此后不再从在用根更新期望。
报告复核逐段检查该引用的 digest、length、revision 与发布制品登记相等。
后续驱动已改为直接从文件登记与目录冻结引用；该修改另经短回归验证，未用新源码身份
冒充这次运行。此次结果覆盖冻结引用之后的审计区间，不计安装与建根阶段的失真检出能力。

预检先完成 60 秒节奏控制运行：4 段、3,463 个覆盖 tick 均匹配，采样合计工作集峰值
53,432,320 B、私有字节峰值 32,595,968 B。它只用于确认资源量级与取证链路可执行。

30 分钟采集已完成；逐段复核见 [机器可读摘要](results-2026-10-24.json)：

- UTC 2026-10-23 16:31:31.987 至 17:01:34.178（日本时间 2026-10-24 01:31–02:01）；
  原世界驻留观察窗口 1,800.882 秒，外层含初始化/退出共 1,802.190 秒。
- 100 段全部 `CompleteMatch`，`Diverged = 0`、`Incomplete = 0`；共核对 1,794 个检查点
  （含相邻段共享端点），连续覆盖 tick 1 → 102,019，共 102,018 个推进 tick。
  初始安装与第一拍仍不计覆盖。持续回收累计完成 6,193 次替换，不据此主张交通质量验收。
- 初始、父进程周期与子进程恢复前的 201 次修订认证均 `Pass`；父子文件探针重写次数为 0，
  父进程与全部子进程 stderr 均为空，未观测到写后读回异常。
- 外层取得 1,773 次资源样本，观测到的合计工作集峰值为 55,836,672 B（53.25 MiB），
  私有字节峰值为 35,545,088 B（约 33.90 MiB）；未触发 2 GiB 停止阈值。
  这些是父子测试进程的采样值，不含外层 PowerShell 看护，也不是分配事件堆峰值。
- 独立复核核对全部请求/响应的进程身份、序号/终点、逐点摘要、宿主记录流、存档文件长度与
  SHA-256、发布引用三轴、资源计数及日志摘要；100 段无缺口。原始证据目录 1,111 个文件的
  排序内容清单摘要记录在 JSON 中，完整清单与原件留在本机。

对应实现验证：static-network 87 个单元测试通过；Runtime 602 通过、20 忽略；
urban-harness release 47 通过、5 忽略；原 #954 写后读回矩阵 10 通过。
直接从发布登记冻结引用的后续修改另通过独立进程短回归与 Clippy；没有将它表述为重新完成
30 分钟采集。相关 Clippy、rustfmt、Markdown 表格与差异空白检查通过。

CI 源码审计发现最初的跨包 `#[path]` 引用不符合包内源码边界；后续将比较器原样迁入
内部 workspace 测试支持包，两处调用仅使用开发依赖，继续接受完整源码与工具链审计。
该迁移不改变比较判据，也不改变本次冻结二进制及其原始取证结果。
后续审查还将失败记录限定在本次原子创建的目录内；已有目录或非法时长被拒绝时不会改写
原始证据。对应负向回归与独立进程短回归另行验证，不计作重新完成 30 分钟采集。

本轮没有申请安静计算窗口，期间正常进行本机编译与测试。无失配只能表述为该规模、
该时间、该摘要与检查点覆盖范围内“未观测到”；不能证明所有匿名堆字节未变、
检查点之间的瞬时错误不存在，或该机器已经长期稳定。只有 `CompleteMatch` / `Diverged`
取得完整计划的段具有审计覆盖；`Incomplete`、跨管理边界以及初始一拍单列。

## 运行与留档

普通回归：

```text
cargo +1.99.1 test --locked -p laneflow-static-network --lib
cargo +1.99.1 test --release --locked -p laneflow-urban-harness --lib
cargo +1.99.1 test --locked -p laneflow-runtime --lib origin_mismatches_fail_closed
```

当前长期测试入口为 `integrity_soak::bounded_soak`（默认 ignored），包含独立资源看护、
分段证据与同驻留世界的安全暂停 / 恢复。构建、时长 / 资源配置、控制命令和判读规则见
[长期浸泡驱动](long-soak-driver.md)。复跑必须新目录，不能覆写原证据；关闭后的新一轮
不冒充匿名堆连续驻留。持续时间与资源范围须先由用户约定；正确性浸泡不要求计算安静窗口，
需要其他安静开发窗口时应先取得安全暂停确认。本报告的历史 30 分钟样本保持原记录口径，
不因工具扩展而升级为长期验收或暂停功能的测量证据。

2026-10-25 另完成一次[两小时正确性浸泡试采](two-hour-pilot.md)，398 段独立回放均为
`CompleteMatch`，并验证一次真实安全暂停恢复；该报告保留独立的冻结身份、时长、资源口径与未覆盖范围。

2026-10-25 至 26 日完成同一冻结二进制的[24 小时自然取证](24-hour-soak.md)：
4775 段全部 `CompleteMatch`，连续推进 4889009 tick，本轮没有暂停或重启。
全部原件经独立离线复核，并保存经逐文件校验的同机第二份副本；完整方法、主机背景、
资源采样边界及二期建议见该报告，不把本次采集预算完成当作 #963 整体验收。

完整源制品、存档、每段请求/响应、stdout/stderr 与资源采样保存在本机忽略目录，
不随本报告承诺远端原包长期可恢复。入仓摘要保存用于审阅的规模、结果、边界与内容摘要；
原始数据不进入产品基线。二期建议维持现行宿主审计与认证边界：本次未观测到失配，
尚未满足设计 §9 第 1 项的运行中失真证据条件，因此不据此启动运行中保护 G1；
取证记录留档不等于长期稳定性验收，也不消除上述未覆盖矩阵要求；五项验收分别核对，
第 3 项“24 小时取证留档”已完成，“未观测到失配”是本轮有效结果，不要求为复现故障重测。
二期准入尚未满足，整体 Issue 仍保留剩余矩阵；完整原件与备份继续保存。
