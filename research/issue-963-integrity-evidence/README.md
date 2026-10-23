# 共享路网认证注入与城市短时浸泡

本研究回答 [#963](https://github.com/illusion-tech/laneflow/issues/963) 的首轮问题：
现有认证、宿主存档、独立进程重放与写后读回能否连成有完成性边界的取证链路，
以及新增注入是否保持 [完整性设计](../../docs/design/traffic-runtime-state-integrity.md)
§2/§4/§5 的失败分类。研究使用 Rust 测试目标，不增加生产 API、默认运行中扫描或新的摘要算法。

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

| 场景                                     | 证据入口                                                                  | 本轮判据与边界                                                                                                |
| ---------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `origin` 的六个静态契约版本轴            | `every_static_contract_origin_field_is_rejected`                          | 六轴分别失真均 `Mismatch / Indeterminate / DisableRoot`；修订号与分区提示派生版本由既有测试覆盖               |
| `origin` digest、length 及二者同时失真   | `origin_digest_length_and_combined_distortions_are_declared_misses`       | 均 `Pass`，明确记为设计 §5 的声明漏检面                                                                       |
| 切换描述符字节身份                       | Runtime `origin_mismatches_fail_closed`                                   | digest、length、二者同时失真，在 base/target 两侧均拒绝；与认证字节身份漏检按两个入口分别验证                 |
| 修订瞬时失真、复算失配位置变化           | `transient_root_and_moving_mismatch_are_untrusted_through_real_recompute` | 真正执行两次比较，保留首次位置并判 `Untrusted`                                                                |
| 修订失配后复算分配失败、取消             | `root_mismatch_is_not_erased_by_recompute_allocation_failure_or_cancel`   | 在复算前触发既有分配钩子或取消标志，均保留首次证据并判 `Untrusted`                                            |
| 来源首次失败后恢复、复算证据变化         | `source_transient_and_different_failures_use_real_reader_results`         | 分别为带瞬时诊断的 `Pass` 与 `OperationFailed(SourceEvidenceUnstable)`                                        |
| 两侧同值零列                             | `common_cause_equal_zero_columns_are_a_declared_comparison_miss`          | `Pass`，声明漏检；只证明比较器不能区分相同损坏内容，不证明物理共页已注入                                      |
| 派生列零值、替换、截断                   | `zero_replaced_and_truncated_derived_columns_are_rejected`                | `Mismatch / SharedStaticRoot / DisableRoot`；列小于一页，不计真实 4 KiB 根页覆盖                              |
| 发布来源零页、页替换、正负一页位移、截断 | `published_source_page_patterns_are_source_failures`                      | 30,400 B LFCA 上按 4,096 B 对象内偏移注入，五种均 `Mismatch / Source / IsolateSource`；不是内存地址页对齐证明 |
| 安装/恢复前认证门禁与独立进程重放        | `short_chain_replays_in_an_independent_process`                           | 真正启动成功与错误引用两个子进程；错误引用在认证处退出，不产生成功响应                                        |

既有测试继续覆盖不同修订、自洽重发布、引用失真、reader 限额、builder 预算、首次分配失败、
取消、来源准备/修订比较 panic、正负零、NaN、失配证据身份和遥测失败。
#954 的写入注入矩阵继续在原模块执行；这里调用其交付能力，没有复制 `ArtifactIo` 注入机制。

以下仍不作为已覆盖项：真实根 backing 的 4 KiB 替换/整数页位移与物理共页；已验证来源
在 builder 重读前改变、builder 消费中受损 panic、reader 通过而 builder 闭合失败的完整输入链；
扫描已读字节随后改变；非 `Pass` 审计与正在执行的 `step` 精确重叠时的宿主输出处置；
故障机长期自然取样及匿名堆失真归因。编辑来源仍随 #529，未混入发布来源验收。

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

长期测试入口为 `integrity_soak::bounded_soak`（默认 ignored）。先构建测试二进制，
由外层资源看护启动该二进制，设置 `LF_SOAK_OUTPUT` 为新的绝对目录、`LF_SOAK_SECONDS`
为约定秒数，并设置 `LF_SOAK_COMMIT` / `LF_SOAK_BINARY_SHA256` 记录身份；用
`--ignored --exact integrity_soak::bounded_soak --nocapture` 运行。复跑须新目录，
不能覆写原证据。持续时间与资源范围须先由用户约定；正确性浸泡不要求计算安静窗口。

完整源制品、存档、每段请求/响应、stdout/stderr 与资源采样保存在本机忽略目录，
不随本报告承诺远端原包长期可恢复。入仓摘要保存用于审阅的规模、结果、边界与内容摘要；
原始数据不进入产品基线。二期建议维持现行宿主审计与认证边界：本次未观测到失配，
尚未满足设计 §9 第 1 项的运行中失真证据条件，因此不据此启动运行中保护 G1；
短时链路验证也不足以关闭长期故障机浸泡与上述未覆盖矩阵要求。
