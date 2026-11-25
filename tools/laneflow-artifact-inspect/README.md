# laneflow-artifact-inspect

LFCA / LFSM / LFSD / LFCP 可移植规范制品的只读检视与结构化差异工具（库 + CLI），交付于 [#1424][issue-1424]。

## 边界

- 只读：不写回、不修复输入；所有预检在 `without_digest_probe` 内执行，损坏输入不触发 #794 取证转储或探针输出。
- 检查语义完全复用 `laneflow-format` 正式预检与登记：本工具不复制线格式偏移常量、不新增独立 lf* 解析器、不放宽任何检查。
- 检查范围是「单对象结构与直接值域检查」；跨对象绑定、运行时静态路网构建与发布真实性不在检查范围内。LFSD 中明确登记为 RowV1 且已受检的 Bytes 才展开内嵌行，其他 Bytes 保持不透明预览，不凭内容猜类型。
- 诊断文本不是数据契约：报告正文与退出码是 CLI 行为约定，不承诺机器可读 schema 或跨工具版本文本兼容。
- 不替代 `check_post_emission_bundle`、`check_canonical_network_input` 或 `xtask classify-digest-probe`。

## 命令

```powershell
cargo +1.100.0 run --locked -p laneflow-artifact-inspect -- inspect tests\fixtures\example.lfca
cargo +1.100.0 run --locked -p laneflow-artifact-inspect -- diff expected.lfsm actual.lfsm
cargo +1.100.0 run --locked -p laneflow-artifact-inspect -- inspect --hex expected.lfcp.hex
```

子命令与选项（完整说明见 `--help`）：

- `inspect [选项] <PATH>`：有界摘要（对象种类、格式版本、字节数、节/表/行数、chunk 概览、检查结果、检查范围声明）。
  - `--expand=preamble,sections,chunks,fields`：显式展开前导、节目录、chunk 目录与按登记名称显示的字段。
  - `--section=<序号|登记名>`、`--table=<kind|登记名>`、`--rows=START..END`：筛选；行序号为零基全表逻辑行，范围左闭右开。
  - LFCP 的节为 singleton 表：摘要标「chunks: 不适用」，chunk 展开标「chunk directory 不适用」，不伪造目录或 chunkDigest。
- `diff [选项] <EXPECTED> <ACTUAL>`：方向固定 expected/old（左）→ actual/new（右）。报告三类差异——原始字节（解码后对象 bytes 的首个分歧绝对偏移，前缀相同时报短侧 EOF 位置）、容器（对象长度、节偏移/长度、chunk 数/偏移/长度/摘要）、字段（按节、表、逻辑行、tag 顺序，含路径、旧值、新值与物理位置）。
  - 精确字节相等是相等判据；摘要仅用于定位与汇总。
  - 目录摘要/偏移/长度变化不占用字段差异展示名额；实际表内的摘要、修订和绑定字段照常比较。
  - 行按逻辑序号对齐（不做 StableId/LCS）；字段按 tag 合并，可选字段缺失显示「缺失」，与零值、空字符串区分；嵌套 RecordVector 与已登记内嵌行沿完整路径定位（如 `SourceModules.SourceLocation[7].startLine`）。
  - 任一侧预检失败：报告两侧检查状态与 `FormatError`（偏移为结构局部坐标或未知，不是文件绝对位置），停止字段解释；在两侧均已完整读入的前提下仍报告原始字节差异。
- `--hex` / `--hex-left` / `--hex-right`：输入外层按 hex 文本解码（可含 ASCII 空白）。默认 raw，raw 失败不自动回退 hex；奇数位、非法字符、读入或解码超限均报错。

## 退出码

| 结果 | 退出码 |
| --- | --- |
| inspect 完成规定的单对象检查与请求的有界展示；或 diff 检查完成且 bytes 相同 | 0 |
| diff 检查完成且 bytes 不同 | 1 |
| 无法完成：参数、I/O、格式、版本、对象种类或资源限制错误 | 2 |

预定展示限额造成的明确截断不等于失败，仍按完整比较结果退出；两个相同的非法对象不能因 bytes 相等而成功。报告正文走 stdout，执行错误走 stderr。

## 限额与内存行为

- `--max-object-bytes`、`--max-chunks-per-section` 只能收紧到 HARD 默认（4 GiB / 65536）之下，请求更高值直接报错（工具显式执行该规则）。
- 输入在分配前以元数据长度限界，读入后复核；不先无界读取再预检。hex 文本上限为对象预算两倍加 4 KiB 余量，解码后仍受对象预算约束。
- `diff` 两侧对象 bytes 需要共同存续（比较期间同时驻留内存）；默认单对象预算 4 GiB，不据此宣称进程总内存有相同上限。
- 报告独立设限：`--max-report-bytes`（默认 65,536）以及内部条数/行数/单值/向量限额（差异条目 200、展开行 64、单值 128 字节、向量 16 项）。先限制再展开，截断有明确标记；展示限额与筛选不改变检查与比较结论——选定范围未发现变化时与全对象结论分别说明。

## 确定性

同一工具版本、相同输入与相同选项，稳定报告正文逐字节一致：固定遍历顺序、固定换行与值格式，不依赖散列容器遍历顺序或本机 locale，不注入工作目录、时间或平台信息。合法有限浮点同时显示十进制与位模式；正式读取器拒绝负零、NaN 与无穷，本工具保持该拒绝语义。本仓库测试验证固定期望文本与同机重复性；跨平台验证在声明支持的平台分别执行，单机重复性不替代跨平台验证。

## 依赖

`laneflow-format`（std feature）与 `laneflow-static-contract`；不反向依赖 compiler 或 runtime。compiler 通过 dev-dependency 使用本工具的断言辅助：

```rust
laneflow_artifact_inspect::assert_portable_object_matches!(actual_bytes, expected_bytes, "fixture 标识");
```

相等路径立即返回（不解码、不构造诊断、不触发探针）；失配时给出有界结构化差异，panic 位置指向测试调用处。fixture 标识与重生成提示由调用方提供，本库不猜测 dump 命令。

## 测试

```powershell
cargo +1.100.0 test -p laneflow-artifact-inspect --locked
```

[issue-1424]: https://github.com/illusion-tech/laneflow/issues/1424
