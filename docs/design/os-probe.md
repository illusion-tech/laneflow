# 操作系统计数器读取

> **状态**：Proposed（[#1391][issue-1391] G1）<br>
> **日期**：2026-11-16<br>
> **决策依据**：[#1391][issue-1391]；[`lf-synth-measurement.md`][lf-synth-measurement] §7<br>

## 1. 目标与非目标

本设计定义 `laneflow-os-probe`：仓库内读取本进程与整机操作系统计数器的唯一入口，
供测量与诊断工具在计时区间外调用。

需求来源：

- [`lf-synth-measurement.md`][lf-synth-measurement] §7 的逐段负载记录需要整机与
  本进程 CPU 时间；Windows 侧没有合格的安全封装（#1391 背景），工作区又禁止
  `unsafe`。
- 性能基线 §5 要求所有规模报告 working set、private bytes 与 commit peak，
  synth-harness 目前只在 Windows 上读取进程内存，Linux 记为不可用。
- [`shared-static-network.md`][shared-static-network] §10 与
  [`traffic-runtime-state-integrity.md`][state-integrity] 要求进程 RSS 与逻辑字节、
  堆请求字节分开报告，仓库没有统一读取入口。

本设计提供：

1. 整机、本进程与当前线程的累计 CPU 时间；
2. 本进程内存与缺页计数；
3. Linux 本进程各线程的就绪等待累计时间；
4. 交流电与电池状态。

本设计不提供：

- 分配计数与堆峰值：继续由 `stats_alloc`、`dhat` 在各自 feature 下负责；计时测量
  不与计数分配器同进程运行的规则不变。
- 其他进程或子进程观测、进程快照与环境记录：由 [#1392][issue-1392] 的诊断记录层
  承担。
- 有效频率：周期计数可能以固定速率计数，见 `lf-synth-measurement.md` §7.1。
- 百分比、速率或任何经时间窗换算的派生量：由调用方按差值计算。
- WMI、PDH、ETW、性能计数器服务或常驻采集进程。

## 2. 定位与依赖边界

- crate 位于 `tools/laneflow-os-probe`，`publish = false`。
- 只允许 `tools/` 下的 crate 依赖，或任意 workspace crate 作为 dev-dependency 引用；
  `crates/` 下的生产依赖不得引用。该约束由 xtask 检查。
- Windows 侧唯一依赖为 `cfg(windows)` 下的 `windows-sys = "=0.61.2"`，只开启
  `Win32_Foundation`、`Win32_System_Threading`、`Win32_System_ProcessStatus`、
  `Win32_System_Power`。该版本已在锁文件中，由微软维护，MIT 或 Apache-2.0。
- Linux 侧读取 `/proc` 与 `/sys`，不需要 `unsafe`。`/proc` 中以时钟中断计数的字段
  须在运行时取得每秒计数值换算，不得假定为 100；取值经锁文件中已有 crate 的安全接口
  （候选 `rustix`）取得，在实现 PR 中按依赖策略说明。
- 没有构建脚本、过程宏或额外编译入口；关闭 Cargo 自动 target 发现。
- 其他平台编译通过，全部字段返回不可用。

## 3. 接口与字段语义

### 3.1 通用约定

- 只返回原始累计值。时间统一换算为纳秒 `u64`，字节为 `u64`，计数为 `u64`；
  不返回百分比或速率。
- 平台不提供的字段返回 `None`，不得填零。各平台语义不同的字段分别命名，不强行
  对齐为同名字段。
- 读取失败返回 `io::Error`，携带操作系统错误码，不吞错，不返回部分结果。
- 不暴露句柄、裸指针或 Win32 结构体。
- 读取器在构造时分配所需缓冲区。之后 CPU 时间与进程内存的每次读取不分配堆内存，
  可在帧间高频调用；就绪等待与电源状态需要枚举目录，可能分配，只适合低频调用。
- 累计值的粒度如实记录：Windows CPU 时间按时钟中断（约 15.6 ms）累计；Linux
  `/proc` 的 CPU 时间按时钟中断计数（常见为 10 ms），`schedstat` 为纳秒。调用方
  按粒度选择差值窗口，见 `lf-synth-measurement.md` §7.3。

### 3.2 CPU 时间

| 字段               | Windows                              | Linux                                                        |
| ------------------ | ------------------------------------ | ------------------------------------------------------------ |
| 整机忙碌           | `GetSystemTimes`：内核 − 空闲 + 用户 | `/proc/stat` 汇总行：user、nice、system、irq、softirq、steal |
| 整机空闲           | `GetSystemTimes` 空闲                | `/proc/stat` 汇总行：idle、iowait                            |
| 本进程用户、内核   | `GetProcessTimes`                    | `/proc/self/stat` 的 utime、stime                            |
| 当前线程用户、内核 | `GetThreadTimes`                     | `/proc/thread-self/stat` 的 utime、stime                     |

- 整机值是全部逻辑 CPU 的总和，逻辑 CPU 数一并返回。
- Windows 内核时间包含空闲时间，忙碌值须扣除；Linux 的 guest 时间已计入 user，
  不重复累加。
- 当前线程指调用线程；读取其他线程不在本设计范围内。

### 3.3 本进程内存

| 字段                | Windows（`K32GetProcessMemoryInfo`，`PROCESS_MEMORY_COUNTERS_EX`） | Linux（`/proc/self/status`、`/proc/self/stat`） |
| ------------------- | ------------------------------------------------------------------ | ----------------------------------------------- |
| 驻留字节            | `WorkingSetSize`                                                   | `VmRSS`                                         |
| 驻留峰值            | `PeakWorkingSetSize`                                               | `VmHWM`                                         |
| Windows private     | `PrivateUsage`                                                     | 不可用                                          |
| Windows commit      | `PagefileUsage`                                                    | 不可用                                          |
| Windows commit 峰值 | `PeakPagefileUsage`                                                | 不可用                                          |
| Linux 匿名驻留      | 不可用                                                             | `RssAnon`                                       |
| 缺页总数            | `PageFaultCount`（含软缺页）                                       | minflt + majflt                                 |
| Linux 主缺页        | 不可用                                                             | majflt                                          |

- 峰值都是进程生命周期峰值，包含初始化与证据缓冲，不能按阶段重置。需要阶段峰值时，
  由调用方在独立进程中测量或另行声明口径。
- Linux 没有与 Windows private、commit 语义相同的单进程计数，不以 `RssAnon`、
  `VmData` 或 `VmPeak` 冒充。
- Windows `PageFaultCount` 为 32 位，可能回绕。读取器以 64 位累计返回，并要求两次
  读取间隔内的增量小于 2³²；回绕检测规则在实现中给出并测试。

### 3.4 就绪等待（仅 Linux）

`/proc/self/task/*/schedstat` 第二列之和：本进程全部线程处于可运行但未获得 CPU
的累计纳秒数。读取期间退出的线程不计入，新建的线程从其创建起计入；调用方按差值
使用时，须在其测量协议中说明线程增减带来的误差。Windows 没有不经 ETW 的等价计数，
返回不可用。

### 3.5 交流电与电池

| 字段     | Windows（`GetSystemPowerStatus`）              | Linux（`/sys/class/power_supply`） |
| -------- | ---------------------------------------------- | ---------------------------------- |
| 交流电   | `ACLineStatus`：1 为接通，0 为断开，其余不可用 | `type=Mains` 的 `online`           |
| 电池存在 | `BatteryFlag` 不含“无电池”位                   | 存在 `type=Battery` 条目           |
| 电量     | `BatteryLifePercent`，255 为不可用             | `type=Battery` 的 `capacity`       |

电源计划与厂商性能模式不在本设计范围内，由 #1392 的环境记录采集。

## 4. `unsafe` 边界

### 4.1 登记

本 crate 是 workspace 内第三个受审计的手写 `unsafe` 例外，与
`laneflow-format-mmap`、`laneflow-motion-kernel` 并列：

- crate 级 `unsafe_code = "allow"`、`unsafe_op_in_unsafe_fn = "forbid"`，不继承
  workspace lint；
- `unsafe` 只出现在 Windows 模块的单一源文件中，恰为下表 5 处，每处带
  `// SAFETY:` 说明；Linux 模块与公共接口不含 `unsafe`；
- 源文件清单固定。

| 位置         | 调用                                           |
| ------------ | ---------------------------------------------- |
| 整机 CPU     | `GetSystemTimes`                               |
| 本进程 CPU   | `GetCurrentProcess`、`GetProcessTimes`         |
| 当前线程 CPU | `GetCurrentThread`、`GetThreadTimes`           |
| 本进程内存   | `GetCurrentProcess`、`K32GetProcessMemoryInfo` |
| 电源状态     | `GetSystemPowerStatus`                         |

### 4.2 安全论证

五处调用具有相同的形态，论证在模块文档中集中给出：

1. 输出参数全部是调用方栈上已按零初始化、类型与 `windows-sys` 声明一致的
   `FILETIME`、`PROCESS_MEMORY_COUNTERS_EX` 或 `SYSTEM_POWER_STATUS`；需要声明
   大小的调用传入该结构体的精确字节数。
2. 被调函数只在调用期间写入这些位置，返回后不保留指针，没有回调。
3. `GetCurrentProcess` 与 `GetCurrentThread` 返回伪句柄，不需要也不得关闭，不跨越
   调用传出。
4. 返回值为失败时立即读取 `GetLastError` 并返回错误，不读取输出参数。
5. 不涉及共享可变状态，读取器可在任意线程调用；当前线程字段只描述调用线程。

### 4.3 机器检查

`check-wire-audit` 的受审计 `unsafe` 边界检查扩展到本 crate：

- manifest 路径、包名与形状精确匹配：只允许登记的段，关闭自动 target 发现，
  无构建脚本；
- 依赖与 feature 清单精确匹配 §2；
- 源文件清单精确匹配；
- `unsafe` 出现位置与表达式逐处比对 §4.1；
- 篡改测试覆盖新增 `unsafe`、新增依赖或 feature、新增源文件三类，均须使检查失败。

`crates/` 下生产依赖引用本 crate 时，xtask 检查失败。

## 5. 首批使用方

synth-harness 的进程内存读取改用本 crate，移除 `win32job` 依赖：

- Windows 字段名与口径保持不变，`method` 改为指明本 crate；
- Linux 上不再记为 `unavailable_on_this_platform`，改报 §3.3 的 Linux 字段，
  Windows 专属字段为 `null`；
- 口径变化在 synth-harness README 中说明，已发布测量记录不改写。

[#1388][issue-1388] 的逐段负载记录与 [#1392][issue-1392] 的诊断记录层在本设计
合入后接入。

## 6. 实现验证

实现至少验证：

- 各累计量两次读取单调不减；
- 本进程空转一段时间后，本进程 CPU 增量与墙钟增量的量级一致，整机忙碌增量不小于
  本进程 CPU 增量减去两档粒度；
- 平台不提供的字段为 `None`；
- Linux `/proc/stat`、`/proc/self/stat`、`/proc/self/status`、`schedstat` 与
  `power_supply` 的解析有固定文本测试，包括字段缺失与格式异常时返回错误；
- Windows `PageFaultCount` 回绕处理有单元测试；
- CPU 时间与进程内存读取在构造后不分配堆内存（以计数分配器在单独测试进程中验证）；
- §4.3 的机器检查与篡改测试。

读取开销的量级在实现 PR 中以探索性计量给出；若进入结论，属于负载敏感计量，须在
安静测量窗口内运行。

[issue-1388]: https://github.com/illusion-tech/laneflow/issues/1388
[issue-1391]: https://github.com/illusion-tech/laneflow/issues/1391
[issue-1392]: https://github.com/illusion-tech/laneflow/issues/1392
[lf-synth-measurement]: lf-synth-measurement.md
[shared-static-network]: shared-static-network.md
[state-integrity]: traffic-runtime-state-integrity.md
