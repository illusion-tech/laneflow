# 当前主干 P3／Waiting 工作范围研究

Refs #757；#707 的独立研究切片。基线为
`46fdfaf47ae0c000ddc63420c8a71443baa8fb04`，历史 #734 结果只提供方向。

实测结论、窗口和未覆盖项见 [结果报告](results.md)。审阅修正了完成状态的急减速误计，
并增加每次运行 UUID、质量文件与行程文件摘要、封存输入校验和进程输出索引。
原始测量目录保留，修正后重新采集到 `target/review-*`，不混合两批计时。

本目录保存隔离研究源码与重建工具。`prepare.py` 只用于导出的基线副本，
不修改工作区正式 Runtime 文件。研究模块不加入 Cargo workspace，补丁中的实验入口、
环境变量与计数 API 不构成正式接口。P3 的正式 Runtime 接入由 #759 独立交付，并同步设计实现说明；不以 #707 正式认证作为前提。

## 实验

- 四臂 `all / p3 / waiting / both` 使用同一个正常 release 二进制。
- P3 删除一次预数，按 Active 上界预留；在复制完整状态前过滤本拍到不了门的对象。
  保留 Active 缓存下标、live 顺序和已有 reservation 分支；空输入不分发。
  非空小工作集保留融合阈值，回退也执行同一过滤。
- Waiting 对没有 membership 且本拍到不了 entry 的车辆返回空预览；Motion 仍正常推进。
- 可达性复用主干 `MotionReach`，保留其有效数值域和 50 mm 余量；未知则继续求值。
  站在下一边起点时保留前一门，不跨拍缓存，因此读取当前句柄代次和路线。
- frontier 接近来源、预检频率、边界事件与运动算法均不修改。清零稠密侧表尚未优化。
- 对照不要求逐拍旧轨迹相同；接纳、守恒、吞吐、资源归属与质量共同决定结果。

## 重建

从仓库根导出固定基线到全新目录，例如 `target/review-source`：

```powershell
git archive --format=tar --output=target/base-source.tar 46fdfaf47ae0c000ddc63420c8a71443baa8fb04
New-Item -ItemType Directory target/review-source
tar -xf target/base-source.tar -C target/review-source
python research/issue-757-current-scope/prepare.py target/review-source
$env:CARGO_INCREMENTAL='0'
$env:CARGO_TARGET_DIR='<独立构建目录的绝对路径>'
cargo +1.98.0 build --manifest-path target/review-source/Cargo.toml -p laneflow-urban-harness --release --locked --offline
```

封存普通二进制后，另加 `--features laneflow-runtime/scope-counts` 构建诊断二进制。
计数为线程本地，仅支持一个 worker，入口核验构建模式，禁止伪报多 worker 完整统计。
正常 release 的计数增量被编译移除。计数包含 P2 进入／过滤／完整预览数、P3 预数扫描／
考虑／过滤／求值／逻辑输入状态复制字节及全部运动计算调用数；计数周期为整个
`advance`，包含当拍生命周期命令可能触发的运动计算，不等同于仅 P5 的调用数。
逻辑复制字节不是实测内存带宽。单 worker 不进入分发预数路径，该列为零不代表
多 worker 没有预数扫描。不包含完整资源 cells/claims
字节、错误后额外计算、线程 CPU 或屏障开销，不以零计数填这些认证义务。

使用 `run.ps1` 为每个新进程记录源码全树、二进制与计划身份、运行前后稳定性、环境、
退出码和进程峰值工作集。原输入默认只读路径为
`E:/projects/laneflow-evidence/issue-707/4de40e04`。例如：

```powershell
./research/issue-757-current-scope/run.ps1 -Binary <封存程序> -Source target/review-source -OutputRoot target/scope-runs -Label 10k-a1 -Scale 10k -Mode all -Ticks 512

```

先 A/A，然后四臂交错并反转次序。入口 1–64、筛查 65–512 分列；较长运行至少
经过两个信号周期，再单列后段回流。不把某一个窗口的结果外推给未覆盖窗口。

## 证据口径

`Harness::install/advance` 仍走当前正式公共生成、替换与步进入口，不用恢复状态绕过接纳。
`initial.jsonl` 保存每个接纳对象的身份、实际状态与停车绑定；初始化失败立即报错。
原计划不删车、不改速、不改初态，记录原计划摘要与期望终点，实际输出明确为有限前缀。

计时保留公共命令、公共 step、原 Harness 观测、额外质量观测与完整 iteration。
iteration 包含所有交通日志写出和质量观察，不含计时/计数 CSV 写出及结束 flush。
分位数使用 nearest-rank；各部分 p95 不相加。研究观察仍会影响缓存与温度。

质量工具从历史研究的 `research_quality.rs` 提取并修改；记录左／右截断的行程、
最长停车、Conflict enter/clear 配对与 reservation 持有；新增步进推进距离和
前后均为 Active 且超过 8 m/s² 的拍间减速次数；完成或停车清零速度不算急减速。距离排除命令搬移；8 m/s² 仅作诊断阈值。
原 Harness 继续校验车辆守恒、资源归属与同边车身重叠，首次失败保留失败包。
同区多 owner 不等于碰撞；同边无重叠不证明完整二维安全；未完成行程不从报告中删除。
不宣称急减速阈值、公平性容差或交通质量已经获得产品认证。

`verify_analysis.py` 对本次真实运行施加损坏，核验退出状态、来源、输入、跨运行质量文件、
拍数、摘要及丢失 stdout/stderr 等情况均被拒绝。质量 JSON 与 summary 共享运行 UUID、
规模、模式、workers、拍数、计划与输入身份，并校验质量及行程文件摘要。
分析器将来源、二进制、计划及输入清单绑定到 `evidence/identity.json`，并校验其绑定的
`files.json` 规范 JSON 摘要。该清单在 `7d9ccb78` 已先行提交，含每个进程元数据（UUID）与
计时、工作量、交通日志和输出的摘要；验证通过后才统计窗口，不能靠重新索引接纳被替换的计时。
这是已发布采集清单的绑定，不宣称旧程序曾输出不存在的 producer digest。
`analyze.py` / `aggregate.py` 用于复核本次封存包，不接受新 UUID 的重跑包。上述运行命令
可用于重新执行算法；新批次需单独建立身份和原始清单、另行分析，不覆盖或追加到旧包。
`aggregate.py` 是本次 28 个完整运行及一个明确中断尝试的封存清单，数量与排除项固定；
新的复测批次独立报告，不追加到旧清单里改变历史统计。

## 归档可移植性与观察边界

`files.json` 的路径相对外部 `target` 包；目录结构为 `review-screen/`、
`review-counts/`、`review-long/`、原中断包 `scope-counts/`、`binaries/` 和 `plans/`。
复制时保持各运行及同级 process/stdout/stderr 的原始字节，另将封存二进制和计划放到
`binaries/`、`plans/`；元数据中的采集机绝对路径是历史信息，不需改写。
`python aggregate.py <外部包根目录>` 可复核迁移后的完整矩阵。
路径迁移只改清单路径与清单自身摘要，344 项文件的大小和 SHA-256 均保持原值。

`inventory.py` 要求完整标签/规模/模式/workers/拍数/诊断矩阵，以及唯一运行 UUID
和目录；缺项、重复项都拒绝。`verify_archive.py` 额外验证真实跨模式 work.csv
替换、重复矩阵与 UUID，并复制真实运行到临时目录验证解析结果一致。
源码重建的追加段明确使用 CRLF，以复现原封存字节；Windows 与 Debian Python
重建的全树摘要均为 `820776cb821d8624228c705bf082f2fc84a46ce269cf921bddf25ac19ad27765`。

本批冻结观察器对命令停车结束的时间戳会晚一个 tick；28 次运行的 `parked` 结束
行数均为零，所以不影响本批统计。分析器明确拒绝含这类结束的行程，不将冻结工具
宣传为支持该场景的通用观察器。后续采集若含命令停车，应在新批次修正为步进前边界，
并建立新的源码身份；不能改写本批原始行程。

采集器先落盘进程 stdout/stderr、退出状态，再采集运行后身份并防御性解析 summary；
畸形 summary 记录解析错误并失败退出。注入畸形 JSON 且退出码为 7 的实际子进程已验证
输出、退出码与运行后身份保留。

所有归档校验使用显式条件与异常，`python -O` 不会禁用验证。聚合在写出任一
结果前核验重新构建的清单摘要；中断包损坏或额外文件也不能被重新索引接纳。
负例验证聚合拒绝后 `files.json` 与 `results.json` 原字节不变。
