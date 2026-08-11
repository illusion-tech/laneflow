# #296 RoadEditingSource 校准

本包只承载 `LF-ROAD-EDITING-P100-v1` 的 test/research generator、跨语言 fixture 和正式
校准证据，不是 production JSON frontend。production compiler 和 writer 不依赖本包，
也不读取旧 Geometry JSON。

语义种子只有在以下条件全部成立后才可进入生成器：

- `road-editing-source-workload-definition-v1.json` 绑定的 seed 路径与 SHA-256 精确匹配；
- 外层 seed 和每份嵌入 Geometry 文档都通过 `serde_json 1.0.151` 有类型反序列化；
- 所有 DTO 使用 `deny_unknown_fields`，重复逻辑字段由 serde 失败关闭；
- 五份文档独立复算的结构计数与冻结 workload 完全一致。

本阶段不恢复、包装或兼容已经删除的 production JSON parser。

本地入口：

```powershell
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- seed-audit
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-p100 2 2
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-regularity
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-fixture-identities
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-cross-language target/road-editing-codegen/cross-language/cpp.lfre target/road-editing-codegen/cross-language/csharp.lfre
cargo +1.96.0 run --release --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-evidence-sample base 1 1 formal 1 target/road-editing-evidence/smoke.json
cargo +1.96.0 run --release --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-evidence-run target/road-editing-evidence/raw.json
cargo +1.96.0 run --release --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-evidence-compact target/road-editing-evidence/raw.json target/road-editing-evidence/compact.json
cargo +1.96.0 run --release --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-evidence-verify target/road-editing-evidence/compact.json
```

`road-editing-p100` 通过第一方 typed model 和 writer 生成五个 size-prefixed `LFRE`
buffer，再让它们通过 production reader、preflight、lowering、geometry compile、common
admission 与 Canonical LIR；输出每模块 byte length、retained capacity、SHA-256 及完整编译
指标。正式 G3 测量仍必须由后继 evidence 子命令按冻结的 fresh-process 协议执行。
`road-editing-fixture-identities` 只审计九组合及 companion 的确定性 byte identity，不是
正式计时或峰值样本。

跨语言输入不提交仓库。`xtask check-road-editing-cross-language` 使用固定 `flatc` 和精确
FlatBuffers source commit `7e163021e59cca4f8e1e35a7c828b5c6b7915953`，分别编译最小
C++/C# writer，并在 ignored `target/road-editing-codegen/cross-language/` 生成两份
size-prefixed `LFRE`。上述 `road-editing-cross-language` 子命令随后让两份输入各自经过
production reader、共同 admission 与完整 compiler，要求 Canonical LIR 语义指纹和唯一
`CanonicalFrame` StableId 一致。CI 以同一流程形成证据；这不是新增 C++/C# SDK，也不把
generated binding 或语言 runtime 源码提交仓库。

`road-editing-evidence-sample` 是正式 fresh-process 编排器使用的单样本角色，不是完整
G3 evidence。它要求封闭的 workload/profile/sample identity 和 repository-relative 新 JSON
路径；semantic seed 在计时前读取，随后分别记录 typed-model build、encode、三个
production admission stage 与 complete compile。fixture digest/byte length/retained capacity
来自本次 writer 输出，来源、table、几何点、regularity、LIR 与 peak 则只读取同一次成功
`CompilationMetrics`，不在 research 侧重算资源账本。只有后继编排器完成每组合一次预热、
七个独立正式进程、环境/exact-commit 绑定、统计和剩余观测/改写协议后，才可形成正式证据。

`road-editing-evidence-run` 只允许 clean exact commit，在冻结参考机上串行运行 80 个计时
进程（十个 workload/profile 各一次预热、七次正式样本），随后另起四个不参与计时统计的
allocator probe 进程：base complete compile、regularity complete compile、rewrite candidate
build+encode 与 rewrite candidate complete compile。后两者在 profiler 启动前保留旧五模块
buffer 和旧 accepted revision；complete-compile probe 要求实际新增 heap peak 不超过同次
production `compiler_controlled_peak_bytes`。build+encode peak 单独报告，不伪装成 compiler
账本或 RSS。四个角色使用固定 `dhat 0.3.3` global allocator；该 experimental profiler 只
存在于 research-only `calibrate-alloc` binary，MIT OR Apache-2.0，不进入 production crate
依赖方向，也不污染 80 个正式 CPU 样本。

raw evidence 保留 80 个样本、四个 allocator probe、完整 argv 和环境；compact evidence
通过 `road-editing-source-calibration-evidence-v1.schema.json`，重新读取 measurement commit
中的六个绑定、重算十组中位数/MAD、核对 4097 点观测完整性与单模块 rewrite identity，
再绑定 raw artifact 的 repository-relative path、byte length 与 SHA-256。`compact` 和
`verify` 都拒绝绝对路径、父目录穿越、覆盖已有输出、字段漂移或脱离 exact commit 的
artifact。

本 research crate 是唯一启用 compiler 非默认 `road-editing-g3-evidence` feature 的 workspace
调用方。种子可先由 `load_p100_seed` 在计时区外闭合，再由 `build_*_from_seed` 独立执行
typed-model build；`compile_encoded_modules_with_stage_timing` 从同一 production admission
准备函数取得 size/identifier、verifier、semantic preflight + lowering 三段耗时和完整 compile
耗时。该 seam 不暴露或复制 wire/Typed AST 私有对象，也不改变默认产品构建。
