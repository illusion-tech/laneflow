# 道路编辑预检索引

关联 [#680](https://github.com/illusion-tech/laneflow/issues/680)。生产基线固定为
`fceafad26f919780eb3fd2a47757325480e2d259`；候选生产代码与本报告同提交。
基线只添加 [baseline-harness.patch](baseline-harness.patch) 中的测试入口，未修改生产
算法。候选使用 `road_editing::preflight::benchmark` 的同一来源构造、规模和计时边界。

## 实测结论

通用去重的两两比较已替换为排序和相邻检查。下表的时间是三轮各自三次样本中位数的
中位数，单位 ms；scratch 是同时存续的请求容量峰值，单位 B。逐轮数据见
[results.csv](results.csv)。比较计数单独运行，未放入计时区。

| 键数量 | 原比较次数 | 候选比较次数 |  原耗时 ms | 候选耗时 ms | 候选 scratch B |
| -----: | ---------: | -----------: | ---------: | ----------: | -------------: |
|    256 |      32640 |         2273 |   0.349800 |    0.019300 |           4096 |
|    512 |     130816 |         5260 |   1.324900 |    0.042200 |           8192 |
|   1024 |     523776 |        11840 |   4.495600 |    0.104900 |          16384 |
|   2048 |    2096128 |        25286 |  16.444300 |    0.365800 |          32768 |
|   4096 |    8386560 |        54407 |  64.939800 |    0.763100 |          65536 |
|   8192 |   33550336 |       117620 | 229.987200 |    1.493600 |         131072 |

排序的最坏 key 比较次数为 O(n log n)，借用键存储为 O(n)。原算法的计数与
`n × (n - 1) / 2` 精确相等；这里比较的是 key 次数，不把变长字符串比较当作单字节操作。

完整预检样例每组包含一个走廊、区段、车道、车道边、车道组和设施带，另有共享的
道路走向与规范坐标框架；稳定声明数为 `6 × 组数 + 1`。各组重复使用局部键
`section`、`lane`、`group`、`band`，由完整 owner 地址区分。所有权查询已采用二分
索引和饱和计数，规模为 M 个声明、R 个引用时，该闭合路径为
O(M log M + R log M)。

| 走廊组数 | 稳定声明数 |   原预检 ms | 候选预检 ms | 候选 scratch B | LFRE bytes |
| -------: | ---------: | ----------: | ----------: | -------------: | ---------: |
|      128 |        769 |   25.022400 |    0.932600 |           5120 |      85320 |
|      256 |       1537 |  102.785800 |    1.113800 |          10240 |     169800 |
|      512 |       3073 |  375.972600 |    3.036700 |          20480 |     338760 |
|     1024 |       6145 | 1393.434500 |    6.418400 |          40960 |     676680 |
|     2048 |      12289 | 5037.197100 |   12.903800 |          81920 |    1352520 |

这组完整预检没有启用比较插桩，CSV 的比较次数留空。基线在这两个 workload 的预检
scratch 都为零；候选最大分别为 128 KiB 和 80 KiB。所测为内部语义预检，不包含
来源构造、writer、FlatBuffers verifier、几何编译、HIR/MIR/LIR、制品发射或来源受检。
这些结果不代表端到端编译耗时，也不替代 #537/#539 的正式资格验证。

## 诊断与资源边界

- 同一份覆盖所有声明种类的 3056-byte 官方 LFRE，逐字节两种翻转及 4096 组双处
  破坏，共 10209 个输入；两版均接受 1632 个、拒绝 8577 个。完整诊断与通过时的
  语义计数摘要均为
  `27ebdb530365137da8c7191819fdde89c541efa39ee485d56254c6f5c6628395`。
- 另穷举短字符串集合，覆盖当前模块限定、导入模块、空串和多重分隔符的原去重
  判定；逐项检查后的重复诊断保留最早原始左下标，不按排序后的 key 决定首错。
- 每次索引分配前检查容量乘法、平台表示范围、阶段 scratch 和既有 builder 加当前
  索引的总存续字节。覆盖精确边界、边界少一、共存与释放、溢出和未分配前拒绝。
  连续 32 次预算拒绝不提交任何候选状态，同一 builder 随后可接纳同名合法模块。
- [用户接受的 G1 收窄](https://github.com/illusion-tech/laneflow/issues/680#issuecomment-5703330314)：
  索引预算不足时先报既有预算超限诊断；预算充足时保留原首错。不为超预算输入保留
  平方扫描后备路径。既有策略集关系预算门槛和全部公开配置档数值不变。
- 该前端无取消参数或可恢复的 allocator OOM 诊断，本项保持普通 Rust 分配失败
  边界。scratch 数字是请求容量账本，不是 allocator 实测或进程 RSS。

## 环境与复现

测量时间：2026-09-17 04:51–04:53（Asia/Tokyo）。AMD Ryzen 9 9955HX，16 核 / 32
逻辑处理器；Windows 11 Pro Insider Preview 10.0.29661；可见内存 66232508416 B。
两版同为 Rust 1.98.1 / LLVM 22.1.8、`x86_64-pc-windows-msvc`，release
`opt_level=3, debuginfo=1, debug_assertions=false, overflow_checks=false`。
每进程单测试线程，固定逻辑处理器 16；每规模一次预热、三次样本。进程顺序为
A/B、B/A、A/B，总计 198 次正式计时。测量期间未运行 Cargo 构建。

二进制 SHA-256：

- 基线：`4361340DA089CE60322A8398761EBDDC4F7407F69FDB0912AD11321FA3F94237`
- 候选：`D2550667928B5746CA1C0C523723F242AFB3C005052E2AE43C41123E5E0BA655`

在独立的基线 checkout 运行 `git apply --unidiff-zero <baseline-harness.patch 的路径>`，
保留候选 checkout。两侧分别运行：

```powershell
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_RELEASE_DEBUG = '1'
cargo test --locked -p laneflow-compiler --lib --release --no-run --message-format=json
```

从 Cargo JSON 的 `compiler-artifact` 选取 `target.name=laneflow_compiler` 且
`executable` 非空的路径，确认上面的 profile 后分别复制为 `baseline.exe` 与
`candidate.exe`。然后在候选 checkout 运行：

```powershell
pwsh.exe -NoLogo -NoProfile -NonInteractive -File research/issue-680-preflight-index/run.ps1 -Baseline <baseline.exe> -Candidate <candidate.exe>
python research/issue-680-preflight-index/analyze.py target/preflight-680/measurements research/issue-680-preflight-index/results.csv
```

`run.ps1` 拒绝覆盖已有 manifest。`analyze.py` 要求三轮成对齐全、每轮每规模三个样本、
测试成功、结构计数稳定及相同来源字节数；缺项、重复、失败或工作负载不一致都会拒绝
生成汇总。原始日志和二进制位于本地 `target/preflight-680/`，仓库保留紧凑 CSV。
诊断对拍在两版二进制分别运行：

```powershell
& <test.exe> road_editing::preflight::benchmark::preflight_diagnostic_oracle --ignored --exact --nocapture --test-threads=1
```

本地验证：

```powershell
cargo test --locked -p laneflow-compiler -p laneflow-format -p laneflow-static-network -p laneflow-road-editing-wire
cargo clippy --locked -p laneflow-compiler --all-targets -- -D warnings
cargo fmt --all -- --check
python -m unittest discover -s research/issue-680-preflight-index -p test_analyze.py
```
