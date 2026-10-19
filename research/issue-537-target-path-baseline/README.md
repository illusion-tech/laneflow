# #537 目标路径性能与资源基线收口

> 状态：测量完成（2026-10-19/20 夜间窗口），回执见
> [#537](https://github.com/illusion-tech/laneflow/issues/537#issuecomment-6268897779) 起的后续评论。
> 本目录保存结论摘要；原始输出与生成器制品在外部证据根
> `<evidence-root>/issue-537/6cf90563/`（`batch1-small/`、`b*.txt`、`run-all.log`、
> `cn-urban-10k/`、`cn-urban-100k/`、`run-all.mjs`）。

## 1. 冻结身份

- 生产代码：main `6cf9056365dcf106bedeba85035863c4fda5a8c1`（含 #538/PR #980；生产文件零改动）。
- 工具链：rustc/cargo 1.98.0，`--release --locked`；计时窗外预编译。
- 硬件/环境：LF-P100-REF-01（AMD Ryzen 9 9955HX 16C/32T、66,232,508,416 B 内存）、
  Windows 11 build 29683、BatteryStatus=2（交流电）、平衡电源方案。
- 安静纪律：计时件运行期无编译、无并发重负载（每件前自动门禁）；计数件不计时。
- 口径声明：小档墙钟=未插桩普通 release；生成器（批 3）墙钟=stats_alloc 全局插桩口径
  （与 #543 历史同口径）；`retained_logical_bytes`=逻辑字节；stats_alloc/dhat 字节=请求布局字节。

## 2. 结果表

### 2.1 compile/emit/check/build/load 全管线（LF-CN-URBAN 生成器，插桩口径）

`laneflow-urban-generator --config examples/config/cn-urban.toml --scale <档>`，单发；
产物 7 个规范文件 SHA-256+字节数与仓库冻结清单（`tools/laneflow-urban-generator/fixtures/v1/`）
逐文件一致，`network_revision` 与 `compiler_controlled_peak_bytes` 精确匹配。

| 阶段 | 10k（100 cells） | 100k（1,000 cells） |
| --- | ---: | ---: |
| source | 243 ms | 2,666 ms |
| compile | 2,826 ms | 229,631 ms |
| emit（file-backed 发射） | 2,554 ms | 26,727 ms |
| post-emission check | 288 ms | 2,901 ms |
| headless-build | 28 ms | 309 ms |
| spatial-build | 45 ms | 482 ms |
| headless-install | 5 ms | 71 ms |
| spatial-install | 4 ms | 68 ms |
| routes-and-topology | 16 ms | 198 ms |

- 编译器控制峰值：10k 392,995,880 B / 100k 3,930,516,050 B（=冻结值=#819 登记的负面结果）。
- 共享根 retained：10k headless 3,993,748 / spatial 10,046,516 B；100k 39,910,548 / 100,536,436 B。
- **观察（描述性，后续项）**：compile 阶段 10k→100k 为 81× 增长（其余阶段 ≈10–11× 线性），
  强超线性；登记为观察，不在本项修复。

### 2.2 共享静态路网小档（#441 口径，未插桩墙钟 warmup1+7 样本中位）

| 项 | min-headless | full-lane-spatial | corridor |
| --- | ---: | ---: | ---: |
| build 墙钟 | 11.7 µs | 108.6 µs | 1.228 ms |
| identity 查找 | 0（0 次） | 0.7 µs（56 次） | 7.2 µs（302 次） |
| kernel 1024×4 ms step | — | 1.424 ms | 1.292 ms |

- 插桩校准：同窗 instrumented build 3.847 ms vs 未插桩 1.228 ms（≈3.1×；两二进制纪律成立）。
- 分配账本（batch1-small/）：min-headless 2 次/2,844 B；full-spatial-omit 247/12,797 B；
  full-lane-spatial 255/13,361 B；corridor 241+58/119,038 B、retained 92,335 B。
- **多世界（2/8/32）**：每世界净 live 16,372 B（< 16 KiB 硬预算 16,384 B，差 12 B），
  总量严格线性、`static_retained=92,335 B` 恒定、`Arc::ptr_eq` 同根——共享静态载荷未随世界数复制。

### 2.3 快照五维（published，v6，夹具 LFCA 425,418 B）

| 维度 | 当前（2026-10-19） | 切片 B 初值（v1，420,332 B） |
| --- | ---: | ---: |
| LFRS exact bytes | 3,312 B | 3,248 B |
| capture 主线程停顿 | 0.0016 ms | 0.002 ms |
| 后台 encode | 0.0029 ms | 0.0072 ms |
| published restore 整次 | 0.0324 ms | 0.063 ms |
| 基线/竞争 tick 中位 | 0.0012 / 0.0027 ms | 0.001 / 0.001 ms |
| DHAT 恢复增量峰值 | 45,472 B / 418 blocks | 19,240 B / 272 blocks |
| restore 返回 live | 44,152 B | 17,920 B |

账本：capture 37 次/5,391 B（净 3,563）；encode 34+13/9,672 B（净 7,640）；
restore 562+12/50,368 B（净 43,960）。保存前后稳态 tick 账本 0/0（硬断言过）。
editable 侧沿 #529 未覆盖。restore 账本/峰值较初值约翻倍：v6 schema 与夹具漂移，
属"建立当前基线"，非回归判定。

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

同修订调用账本 456 次/30,936 B（净 −25 B）；共存双根 retained 184,670 B。
跨修订日志算术 16 tick×(37 B+78 B×8 车)=10,576 B 硬断言过；Prepare 204 次/8,409,372 B
（8 MiB arena）；共存=journal 10,576+arena 8,388,608+target 根 3,424+LFSD 2,041 B。

### 2.5 cutover 规模档（#531 夹具，3 样本中位，µs）

| 车辆/边/路线（模式） | 准备 | 泵入 | 提交 | 独立摘要 |
| --- | ---: | ---: | ---: | ---: |
| 1k/16（online） | 735 | 107 | 1,408 | 263 |
| 1k/256 | 530 | 106 | 1,266 | 295 |
| 4k/16 | 6,558 | 459 | 9,223 | 1,230 |
| 4k/256 | 1,689 | 431 | 4,528 | 1,199 |
| 10k/16 | 36,797 | 1,114 | 43,816 | 2,837 |
| 10k/256 | 5,128 | 1,091 | 11,840 | 2,930 |
| 10k/256/万路线 | 11,270 | 1,197 | 19,991 | 4,896 |
| 10k/256（drain） | 5,387 | 0 | 13,626 | 3,130 |
| 10k/256（paused） | 5,386 | 0 | 11,805（整段 17,112） | 2,839 |

万车 256 边提交 11.84 ms，较 #593 的 124.287 ms 降一个量级——`18c64ced`/`dada8b4a` 等
占用重建/重叠索引并行化的效果，属当前基线重建；3 样本中位不构成三轮静默认证。
分配侧（phase=digest/prepare/commit）逐 case 数字见原始输出。

### 2.6 单世界 tick（道路组，未插桩，3 轮中位）

| 场景（100 ms 步进，64 拍） | p50 | p95 |
| --- | ---: | ---: |
| 1k/256 边 | 125.8 µs | 148.9 µs |
| 10k/256 边 | 1.297 ms | 1.412 ms |
| 10k/16 边 | 1.260 ms | 1.364 ms |
| 10k/256 边（日志武装） | 1.622 ms | 1.767 ms |
| Waiting/1 车 | 1.1 µs | 1.5 µs |
| Conflict/2 车 | 1.7 µs | 1.8 µs |

- 对照 #583（298da8a8）：10k/256 p50 4.108→1.297 ms、p95 4.707→1.412 ms（并行 kernel）。
- 日志武装档 `journal_bytes=56,162,664 B` 与 #583 逐字节相同——跨 28 个提交的窗口字节级等价。
- 全部场景测量窗分配 0 次/0 B（跨墙钟/分配两二进制 digest 一致）。
- 注意：runtime_profile 三件须以 `--features placement-fixtures` 运行（cb3bca94 起新鲜摆放
  在门线处失败关闭，#742 起 CI 即用该 feature；#583 README 旧命令已过时，本 PR 一并修正）。

### 2.7 观测/动态成本/注册接缝

复用 seams v4 三轮（85ec5892=当前树）：delta 导出 338.7–339.1 µs 等，见 #538 回执，不重跑。

## 3. 与既有初值的差异解释

OS build 29648→29683、corridor 夹具 420,332→425,418 B、kernel 并行化（298da8a8..HEAD
+5.4 万行）、快照 v1→v6、#284 策略代、生成器 LF-CN-URBAN-v1（退役）→v2——凡差异均按
"历史初值不可直接比较，建立当前基线"处理并注明来源，不包装为性能提升。

## 4. 分类结论

- **预算通过**：#707 城市档四负载三轮（10c625ae 条件内复用，本轮核验 10c625ae..6cf90563
  生产步进路径未变）。
- **预算未通过（保持）**：#819 城市编译器峰值 3,930,516,050 B > 2,147,483,648 B 目标——
  本轮重跑精确复现冻结值并验证产物哈希，维持未通过，不批准例外。
- **描述性完成（无产品 Pass）**：全管线 10k/100k 九阶段、共享静态小档墙钟/账本/多世界、
  快照五维 published、cutover 小档+规模档、tick 道路组、每世界状态。
- **未测及原因**：LF-SYNTH-v1（目标路径无生成器，仅有生成契约）；1M 档（生成器 Scale 仅
  fixture/10k/100k；100k 编译峰值 3.93 GB 线性外推 ≈39 GB，远超 LF-COMP-SINGLE-NETWORK-1M-v2
  的 6.44 GB 失败关闭天花板，属预测失败关闭）；editable 恢复（#529 延期）；requested
  capacity 与 RSS（无测量面，本轮不新增）；规模档 compile/build 普通未插桩墙钟（生成器为
  插桩口径，与 #543 历史同口径可比）。
- **已授权延期**：无。

## 5. 过程事件与后续项

- 停止条件触发一次：runtime_profile 按 #583 旧命令首跑 panic（`StopConstraintUnsatisfiable`，
  conflict 场景暖机新鲜摆放）——定位为 #583 README 命令过时（cb3bca94 语义收紧 + #742 的
  CI feature 迁移未同步该 README），非内核缺陷；改用 CI 同款 `--features placement-fixtures`
  后三件全过。失败可由旧命令确定性复现，原始失败记录存
  `<evidence-root>/issue-537/6cf90563/b2-runtime-profile-wall-FAILURE-old-command.txt`。
- 后续项（不在本项修复）：① compile 阶段 10k→100k 超线性（81×）；② #583 README 命令修正
  （本 PR 顺带完成，属文档修正非领域实现）；③ 1M 档若要做，需新生成入口+专门授权。
