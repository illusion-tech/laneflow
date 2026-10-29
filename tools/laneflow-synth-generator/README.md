# LF-SYNTH 正式输入生成器

生成 `LF-SYNTH-v2` 的一万/十万车辆输入，覆盖 W1–W4、16/33 ms。输出一个包含
`network.lfca`、`state.json`、`input-plan.json`、`manifest.json` 的新目录。
文件合同见 [规范输入交付设计](../../docs/design/lf-synth-input-bundle.md)，交通语义见
[性能基线 §4](../../docs/design/core-runtime-performance-baseline.md#4-canonical-workload)。

编译：`cargo +1.99.1 build --release --locked -p laneflow-synth-generator`。

命令行的位置参数为：

```text
laneflow-synth-generator generate <10000|100000> <W1|W2|W3|W4> <16|33> <source-commit> <generator-commit> <new-directory>
laneflow-synth-generator verify <directory>
```

两个提交参数必须是真实的 40 位小写提交标识；生成实现应先提交，再生成交付证据。
输出目录的父目录须已存在，目标目录必须不存在。生成器不会覆盖旧交付；写入后逐文件
回读，manifest 最后写入。失败时保留不完整目录，退出非零。

`verify` 检查目录、规范编码、摘要、计数及稳定顺序，并完整重新编译比较精确字节；
它是离线验证，不属于 Runtime 计时窗口。成功时两条命令都向 stdout 输出 manifest。
这不认证调用者提交参数的真实性，也不是 LFCP 发布或产品认证。

`state.json` 的标识使用现行 `lfid1_<entity-kind>_<32 lowercase hex digits>`，可直接
解析为 `LaneEdgeId` 等类型，再通过共享路网 identity index 解析 ordinal。目录中的
logical rank 只管理宿主逻辑身份，不能作为 Runtime handle 或静态 ordinal。
`input-plan.json` 包含 W3 arrival barrier 和完成观测驱动规则；计划不是实际执行回执。

普通验证：

```text
cargo +1.99.1 test --locked -p laneflow-synth-generator
cargo +1.99.1 clippy --locked -p laneflow-synth-generator --all-targets -- -D warnings
```

`fixtures/probe-vectors.json` 冻结 2 cell × W1–W4 × 16/33 ms 的三个文件摘要和长度，
只用于编码回归；公开生成入口拒绝该非正式规模。测试通过真实 LFCA 受检读取、保留
Spatial 的共享路网构建、显式政策安装、路线注册、初始人口构造和一拍推进核对可消费性。

完整交付验收先构建 release 二进制，再为测试进程设置四个环境变量：

- `LF_SYNTH_GENERATOR_BIN`：刚构建的二进制绝对路径。
- `LF_SYNTH_EVIDENCE_ROOT`：尚不存在的证据目录绝对路径。
- `LF_SYNTH_SOURCE_COMMIT`：交通合同来源提交。
- `LF_SYNTH_GENERATOR_COMMIT`：生成实现提交。

随后执行：

```text
cargo +1.99.1 test --release --locked -p laneflow-synth-generator --lib full_scale_independent_process_matrix -- --ignored --nocapture
```

测试依次处理 16 个正式身份，每个身份运行两个全新生成进程并比较四文件全部字节，
重新验证目录，再核对静态计数、完整几何、信号程序及全部车辆的生产 API 初始化和
一拍推进。原始双份输入与逐行更新的 `summary.json` 保留在证据目录；中断时
`complete=false`，不能把部分记录标成完整通过。它不执行 W1–W4 的完整测量窗口、
停车 barrier 或性能认证；后续执行 harness 仍须独立交付。
