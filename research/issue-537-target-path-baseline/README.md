# #537 目标路径性能与资源基线收口

> 状态：测量完成（2026-10-19/20 夜间窗口）。回执见
> [#537](https://github.com/illusion-tech/laneflow/issues/537#issuecomment-6268897777) 起；
> 结果表为当前权威，`docs/design/` 两处再登记节为合同侧摘要，数值以本文件为准。
> 全部原始输出与账本在本目录 [`evidence/`](evidence/)（约 98 KB，随 PR 入库）；
> 生成器两档规范制品（10k 158 MB / 100k 1.6 GB）不入库：仓内生成器 +
> `tools/laneflow-urban-generator/fixtures/v1/` 冻结清单已含逐文件 bytes 与 SHA-256，
> 本轮实测逐文件一致（见 §2.1），确定性再生即复核路径。

## 1. 冻结身份

- 生产代码：main `6cf9056365dcf106bedeba85035863c4fda5a8c1`（含 #538/PR #980；生产文件零改动）。
- 工具链：rustc/cargo 1.98.0，`--release --locked`；计时窗外预编译。
- 硬件/环境：LF-P100-REF-01（AMD Ryzen 9 9955HX 16C/32T、66,232,508,416 B 内存）、
  Windows 11 build 29683、BatteryStatus=2（交流电）、平衡电源方案。
- 安静纪律（如实披露）：`run-all.mjs` 编排启动 8 件——7 件成功、第 8 件（runtime_profile
  旧命令）exit=101 触发停止条件；成功的 7 件每件前自动门禁（BatteryStatus=2 +
  无并发 cargo/rustc/clippy 进程；门禁只检测编译器进程名，不含其他负载类型）。
  停止后其余 5 件（b2 三件 + b3 两档生成器）为同一窗口内手工串行执行——窗口起点
  核对过电源状态、运行期无编译，但**没有逐件门禁日志**，证据文件亦无编排器命令头
  可区分（见 `evidence/run-all.log`：8 条 RUN = 7 成功 + 1 失败）。
- 口径声明：小档墙钟=未插桩普通 release；生成器（批 3）墙钟=stats_alloc 全局插桩口径
  （与 #543 历史同口径）；`retained_logical_bytes`=逻辑字节；stats_alloc/dhat 字节=请求布局字节。
- 复用身份：PR #980 最终 head
  [`85ec5892a718474639b63534863fb3c7a7335504`](https://github.com/illusion-tech/laneflow/commit/85ec5892a718474639b63534863fb3c7a7335504)
  与 `6cf90563` 树等价（两者 tree 均为 `b8d27e4958c78cbc994efbeb7c8592871821ef4c`），
  #538 seams v4 三轮（该 head 上测得）因此可直接映射到本次基线。

## 2. 结果表

### 2.1 compile/emit/check/build/load 全管线（LF-CN-URBAN 生成器，插桩口径）

复现：`target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 10k --output <新建目录>`（`--scale 100k` 同理；`--output` 必须指向不存在的新目录），单发。产物 7 个规范文件 SHA-256+字节数与仓库冻结清单逐文件一致，`network_revision` 与 `compiler_controlled_peak_bytes` 精确匹配。

| 阶段 | 10k（100 cells） | 100k（1,000 cells） | 倍数 |
| --- | ---: | ---: | ---: |
| source | 243 ms | 2,666 ms | 11.0× |
| compile | 2,826 ms | 229,631 ms | **81.3×** |
| emit（file-backed 发射） | 2,554 ms | 26,727 ms | 10.5× |
| post-emission check | 288 ms | 2,901 ms | 10.1× |
| headless-build | 28 ms | 309 ms | 11.0× |
| spatial-build | 45 ms | 482 ms | 10.7× |
| headless-install | 5 ms | 71 ms | 14.2× |
| spatial-install | 4 ms | 68 ms | 17.0× |
| routes-and-topology | 16 ms | 198 ms | 12.4× |

- install 两档与 routes 为小基数（4–16 ms 起），倍数受噪声放大；其余阶段 ≈10–11×。
- 编译器控制峰值：10k 392,995,880 B / 100k 3,930,516,050 B（=冻结值）。
  该 100k 峰值即 [#305 正文](https://github.com/illusion-tech/laneflow/issues/305)与 #819
  登记的负面结果对照值；其"目标 2,147,483,648 B"仅见于该 GitHub 登记，仓库内无独立出处
  （仓内 2 GiB 数值另有含义：共享根构建预算与编译遍暂存上限），且
  `LF-COMP-SINGLE-NETWORK-1M-v2` 并未拒绝该次编译（失败关闭天花板为 6,442,450,944 B）。
- 共享根 retained：10k headless 3,993,748 / spatial 10,046,516 B；100k 39,910,548 / 100,536,436 B。
- **观察（描述性，后续项）**：compile 阶段 10k→100k 为 81× 增长，显著超出其余阶段；
  登记为观察，不在本项修复。

### 2.2 共享静态路网小档（#441 口径，未插桩墙钟 warmup1+7 样本中位）

| 项 | min-headless | full-lane-spatial | corridor |
| --- | ---: | ---: | ---: |
| build 墙钟 | 11.7 µs | 108.6 µs | 1.228 ms |
| identity 查找 | 0（0 次） | 0.7 µs（56 次） | 7.2 µs（302 次） |
| kernel 1024×4 ms step | — | 1.424 ms | 1.292 ms |

- 插桩校准：同窗 instrumented build 3.847 ms vs 未插桩 1.228 ms（≈3.1×；两二进制纪律成立）。
- 分配账本（evidence/batch1-small/）：min-headless 2 次/2,844 B；full-spatial-omit 247/12,797 B；
  full-lane-spatial 255/13,361 B；corridor 241+58/119,038 B、retained 92,335 B。
- **多世界（2/8/32）**：每世界净 live 16,372 B（< 16 KiB 硬预算 16,384 B，差 12 B）；
  三档总 live 32,744/130,976/523,904 B 观测严格线性（=16,372×世界数）、
  `static_retained=92,335 B` 恒定、`Arc::ptr_eq` 同根硬断言——共享静态载荷未随世界数复制。
  CI 断言面为每世界上限与带宽（`max≤2×min+64`），"严格线性"是本输出的观测描述，非断言。

### 2.3 快照五维（published，v6，夹具 LFCA 425,418 B）

| 维度 | 当前（2026-10-19） | 切片 B 初值（v1，420,332 B） | 倍数 |
| --- | ---: | ---: | ---: |
| LFRS exact bytes | 3,312 B | 3,248 B | 1.02× |
| capture 主线程停顿 | 0.0016 ms | 0.002 ms | — |
| encode（无竞争主线程中位） | 0.0029 ms | 0.0072 ms | — |
| published restore 整次 | 0.0324 ms | 0.063 ms | — |
| 基线/竞争 tick 中位 | 0.0012 / 0.0027 ms | 0.001 / 0.001 ms | — |
| DHAT 恢复增量峰值 | 45,472 B / 418 blocks | 19,240 B / 272 blocks | 2.36× |
| restore 返回 live | 44,152 B | 17,920 B | 2.47× |

账本：capture 37 次/5,391 B（净 3,563）；encode 34 次分配+13 次重分配/9,672 B（净 7,640）；
restore 562+12/50,368 B（净 43,960，allocated 为初值 2.17×）。保存前后稳态 tick 账本
**相等**的硬断言通过（实测两侧均 0/0；断言对象是相等性，不是 0 值本身）。
encode 中位为无竞争主线程测量（后台化可行性沿用 §8 初值节"可移到后台"的表述；
后台竞争影响由竞争 tick 行单独覆盖）。editable 侧沿 #529 未覆盖。
restore 账本/峰值较初值放大 2.2–2.5×：v6 schema 与夹具漂移所致，属"建立当前基线"，非回归判定。

### 2.4 cutover 小档（切片 A/C 口径）

| 维度 | 当前 | 初值（2026-08-28/29） |
| --- | ---: | ---: |
| 同修订整次调用（外部借用保持） | 0.023 ms | 0.049 ms |
| 同修订调用内同步回收 | 0.039 ms | 0.077 ms |
| 旧根孤立析构 | 0.008 ms | 0.020 ms |
| 跨修订 Prepare | 0.025 ms | 0.041 ms |
| 跨修订提交（尾已泵空） | 0.032 ms | 0.064 ms |
| 跨修订提交（整本 64 tick 排空） | 0.060 ms | 0.072 ms |
| 维护暂停整段 | 0.082 ms | 0.165 ms |

计时窗构成（按实现源码核对，与 §2.5 规模档不同，勿跨表直比）：
跨修订 Prepare 窗含一次 `source_for` 全文规范校验（实参在窗内求值）；
两种提交窗与维护暂停窗均含旧世界随事务析构（世界是 base 根唯一持有者）；
维护暂停窗在文档定义的"Prepare+排空+提交"之外，**还含目标根构建与二次规范校验**
（`build(ORACLE_TARGET)`/`source_for` 实参在窗内求值），且 paused 形态零步进、
泵为空转——0.082 ms 是这个更宽窗的值。

同修订调用账本 456 次/30,936 B（净 −25 B）；共存双根 retained 184,670 B。
跨修订日志算术 16 tick×(37 B+78 B×8 车)=10,576 B 硬断言过；Prepare 204 次/8,409,372 B
（8 MiB arena）；共存=journal 10,576+arena 8,388,608+target 根 3,424+LFSD 2,041 B。

### 2.5 cutover 规模档（#531 夹具，3 样本中位，µs）

计时窗构成（与 §2.4 不同）：prepare/commit 实参均为廉价 `Arc::clone`/`source()`，
**不含**目标根构建与二次校验；`roots.base` 全程常驻，commit 窗**不含**旧根析构。

| 车辆/边/路线（模式） | 准备 | 泵入 | 提交 | 独立摘要 |
| --- | ---: | ---: | ---: | ---: |
| 1k/16（online） | 745 | 107 | 1,408 | 263 |
| 1k/256 | 530 | 106 | 1,266 | 295 |
| 4k/16 | 6,558 | 459 | 9,223 | 1,241 |
| 4k/256 | 1,689 | 431 | 4,528 | 1,199 |
| 10k/16 | 36,797 | 1,114 | 43,816 | 2,837 |
| 10k/256 | 5,128 | 1,091 | 11,840 | 2,930 |
| 10k/256/万路线 | 11,270 | 1,197 | 19,991 | 4,896 |
| 10k/256（drain） | 5,387 | 0 | 13,626 | 3,130 |
| 10k/256（paused） | 5,386 | 0 | 11,805（整段 17,112） | 2,839 |

对照锚点：仓内同夹具（万车/256 边/online 提交）的并行化前基线是 #583 证据
`cutover.txt` 的三样本中位 **99.266 ms**（298da8a8，见 `research/issue-583-runtime-profile/evidence/cutover.txt`）；
[PR #593](https://github.com/illusion-tech/laneflow/pull/593) 正文（#531 交付，GitHub 侧）另有
124.287 ms（合并前 head，仓库内无出处）。当前 11.840 ms 与两者的量级差及方向和
`18c64ced`/`dada8b4a` 等占用重建/重叠索引改动一致，但**未做受控 A/B 归因**；
OS build 等环境亦不同，只作"建立当前基线"。3 样本中位不构成任何正式预算判定
（cutover 合同未定义多轮静默窗口认证协议）。
分配侧（phase=digest/prepare/commit）逐 case 数字见 evidence/b2-cutover-scale-alloc.txt。

### 2.6 单世界 tick（道路组，未插桩，3 轮中位）

道路组（100 ms 步进、64 拍测量窗）：

| 场景 | p50 | p95 |
| --- | ---: | ---: |
| 1k/256 边 | 125.8 µs | 148.9 µs |
| 10k/256 边 | 1.297 ms | 1.412 ms |
| 10k/16 边 | 1.260 ms | 1.364 ms |
| 10k/256 边（日志武装） | 1.622 ms | 1.767 ms |

资源探针组（4 ms 步进、16 拍测量窗，与 #583 同夹具）：

| 场景 | p50 | p95 |
| --- | ---: | ---: |
| Waiting/1 车 | 1.1 µs | 1.5 µs |
| Conflict/2 车 | 1.7 µs | 1.8 µs |

- 对照 #583（298da8a8，其后至 6cf90563 共 864 个提交）：10k/256 p50 4.108→1.297 ms、
  p95 4.707→1.412 ms。环境（OS build 等）亦不同，按"建立当前基线"处理。
- 日志武装档 `journal_bytes=56,162,664 B` 与 #583 证据（`wall-clock.txt`/`allocation.txt`）
  逐字节一致（72 拍窗 ×(37 B+78 B×10,000 车) 算术自洽）；窗口字节级等价为真，
  但上述 #583 引用为其冻结基线上的记录。
- 全部场景测量窗分配 0 次/0 B（跨墙钟/分配两二进制 digest 一致）。
- runtime_profile 三件以 `--features placement-fixtures` 运行（cb3bca94 后新鲜摆放
  在门线处失败关闭——Conflict 场景必须已有摆放；#742 起 CI 即用该 feature；
  #583 README 旧命令已在该文件中标注过时与替代命令）。

### 2.7 观测/动态成本/注册接缝

复用 seams v4 三轮（PR #980 最终 head 85ec5892 上测得，与本次基线树等价，见 §1）：
delta 导出 338.7–339.1 µs 等，数字与协议见
[#538 seams v4 回执](https://github.com/illusion-tech/laneflow/issues/538#issuecomment-6264645914)
（GitHub 侧，仓库内无副本），不重跑。

## 3. 与既有初值的差异解释

OS build 29648→29683、corridor 夹具 420,332→425,418 B、kernel 并行化（298da8a8..HEAD
kernel +5.4 万行）、快照 v1→v6、#284 策略代、生成器 LF-CN-URBAN-v1（退役）→v2——凡差异均按
"历史初值不可直接比较，建立当前基线"处理并注明来源，不包装为性能提升。

## 4. 分类结论

- **预算通过（历史认证，条件内）**：#707 城市档四负载三轮——绑定源码 `10c625ae` 与
  认证二进制（报告 §2 登记的构建身份）、16 worker。本轮核验 10c625ae..6cf90563 生产
  步进路径在默认构建下未变（改动全部为 cfg 门控），但**该通过不自动延伸到 6cf90563
  的二进制身份**；当前 head 的城市档预算未重测，若需在当前 head 声明通过须重新认证运行。
- **预算未通过（保持）**：#819 城市编译器峰值 3,930,516,050 B 对 #305/#819 登记的
  2,147,483,648 B 目标（目标值出处见 §2.1）——本轮重跑精确复现冻结值并逐文件校验
  产物哈希，维持未通过，不批准例外。
- **描述性完成（无产品 Pass）**：全管线 10k/100k 九阶段、共享静态小档墙钟/账本/多世界、
  快照五维 published、cutover 小档+规模档、tick 道路组、每世界状态。
- **未测及原因**：LF-SYNTH-v1（目标路径无生成器，仅有生成契约）；1M 档（生成器 Scale 仅
  fixture/10k/100k；100k 编译峰值 3.93 GB 线性外推 ≈39 GB，远超 `LF-COMP-SINGLE-NETWORK-1M-v2`
  的 6,442,450,944 B 失败关闭天花板，属预测失败关闭）；editable 恢复（#529 延期）；requested
  capacity 与 RSS（无测量面，本轮不新增）；规模档 compile/build 普通未插桩墙钟（生成器为
  插桩口径，与 #543 历史同口径）；当前 head 城市档预算（见第一条）。
- **已授权延期**：无。

## 5. 过程事件与后续项

- 停止条件触发一次：runtime_profile 按 #583 旧命令首跑 panic（`StopConstraintUnsatisfiable`，
  conflict 场景暖机新鲜摆放）——定位为 #583 README 命令过时（cb3bca94 语义收紧 + #742 的
  CI feature 迁移未同步该 README），非内核缺陷；改用 CI 同款 `--features placement-fixtures`
  后三件全过。失败可由旧命令确定性复现，原始记录存
  `evidence/b2-runtime-profile-wall-FAILURE-old-command.txt`（头部含旧命令原文）。
- 后续项（不在本项修复）：① compile 阶段 10k→100k 超线性（81×，其余阶段 ≈10–11×，
  install 小基数 14–17×）；② #583 README 命令标注（本 PR 完成，属文档修正非领域实现）；
  ③ 1M 档若要做，需新生成入口+专门授权。

## 6. 复现命令（12 件 + 计数件）

全部在仓库根、生产树 `6cf9056365dcf106bedeba85035863c4fda5a8c1` 上执行；先按组预编译再串行测量（计时窗外编译）：

```powershell
# 批 1 计时件（未插桩墙钟，手动 --ignored）
cargo +1.98.0 test --release --locked -p laneflow-runtime --test shared_network_resource_evidence -- --ignored wall_clock_build_identity_and_kernel --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test snapshot_wall_clock_evidence -- --ignored --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_wall_clock_evidence -- --ignored --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_cross_wall_clock_evidence -- --ignored --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test shared_network_instrumented_wall_clock_evidence -- --ignored --nocapture
# 批 1 计数件（随 CI 亦运行，此处取证）
cargo +1.98.0 test --release --locked -p laneflow-runtime --test shared_network_resource_evidence -- --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test shared_network_allocation_evidence -- --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test snapshot_budget_evidence -- --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test snapshot_peak_evidence -- --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_budget_evidence -- --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_cross_budget_evidence -- --nocapture
# 批 2 规模档（cutover_scale 不需要 feature；runtime_profile 三件必须 --features placement-fixtures）
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_scale_evidence -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --test cutover_scale_allocation -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --features placement-fixtures --test runtime_profile_evidence runtime_profile_wall_clock -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --features placement-fixtures --test runtime_profile_allocation -- --ignored --nocapture --test-threads=1
cargo +1.98.0 test --release --locked -p laneflow-runtime --features placement-fixtures --lib runtime_profile_stages -- --ignored --nocapture --test-threads=1
# 批 3 生成器两档（插桩口径；--output 必须是不存在的新目录）
cargo +1.98.0 build --release --locked -p laneflow-urban-generator
target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 10k --output <evidence-root>/issue-537/6cf90563/cn-urban-10k
target/release/laneflow-urban-generator --config examples/config/cn-urban.toml --scale 100k --output <evidence-root>/issue-537/6cf90563/cn-urban-100k
```

注意：带 `--features placement-fixtures` 的预编译产物与不带 feature 的 `cutover_scale_*`
是不同二进制——预编译须按组分别进行（`--no-run`），避免测量前重编译。本轮流水的
编排脚本与逐件门禁见 `evidence/run-all.mjs`（历史原样，其中 runtime_profile 行仍是
会失败的旧命令；续跑以本节命令为准）。
