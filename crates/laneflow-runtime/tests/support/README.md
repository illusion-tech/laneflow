# 私有执行资源计量

`kernel::execution::memory_tests::execution_resource_memory_probe` 是独立进程探针，
只验证 #704 的私有执行原语。公开世界仍只支持 1 worker；此探针不证明交通并行收益。

先用 `cargo test -p laneflow-runtime --lib --no-run --message-format=json` 找到当前
`laneflow_runtime` 测试可执行文件。在两个 PowerShell 7 终端中使用同一组绝对路径，
`$probeDirectory` 必须是新的空目录；探针每阶段等待采样器至多 45 秒。

```powershell
# 终端 1：使用本次编译输出的准确路径。
$testExecutable = '<绝对测试可执行文件路径>'
$probeDirectory = '<绝对新目录路径>'
$env:LANEFLOW_EXECUTION_PROBE_DIR = $probeDirectory
& $testExecutable --exact kernel::execution::memory_tests::execution_resource_memory_probe --ignored --nocapture --test-threads=1
```

```powershell
# 终端 2：在仓库根目录执行；变量取值与终端 1 完全相同。
& ./crates/laneflow-runtime/tests/support/measure_execution_stacks.ps1 -ProbeDirectory $probeDirectory -ExpectedExecutable $testExecutable
```

采样器仅支持 64 位 Windows，核对进程可执行文件身份，然后只读查询线程 TEB 和虚拟
内存页。布局或查询失败直接报错。`native-stack-report.json` 记录 baseline、idle、
busy、dropped 四阶段原生线程数，以及新增辅助线程的栈地址预留、已提交页和 guard 页
字节数。guard 是 committed 的子集，不能重复相加；配置的栈大小不能替代实际读数。

Rust marker 中的 `active_plan_bytes` 包含计划结构和范围向量容量；
`thread_registration_bytes` 是 JoinHandle 向量容量。它们不包含共享根的被引用内容。
`observed_heap_net_bytes_since_baseline` 是同一独立进程从 baseline 起累计分配减去释放
的净差，包含 Rayon registry、队列、登记、计划变化和探针自身开销，不能当作纯队列
字节或精确峰值。baseline 时交通世界已存在，因此 dropped 的净差可以为负，不能据此
声称堆零泄漏。栈预留单列，禁止混入堆读数。

跨修订切换测试 `execution_plan_prepare_and_commit_fail_closed_then_publish_fresh_workset`
（用 `cargo test -p laneflow-runtime execution_plan_prepare_and_commit -- --nocapture`）
另行输出活动、候选、最终及退休计划容量和
候选状态字节；最终计划准备时三份计划短暂并存。静默窗口计时包含最终验证、计划准备、
发布与旧状态退休，来自小型测试夹具和插桩 debug 构建，不代表城市规模或生产延迟。
