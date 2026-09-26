# 当前主干 P3／Waiting 工作范围研究

Refs #757；#707 的独立研究切片。基线为
`46fdfaf47ae0c000ddc63420c8a71443baa8fb04`，历史 #734 结果只提供方向。

实测结论、窗口和未覆盖项见 [结果报告](results.md)。本次封存源于研究提交
`7e34e008cece324c867413864fc8851007ae9034`，后续提交只补报告和分析工具。

本目录保存隔离研究源码与重建工具。`prepare.py` 只用于导出的基线副本，
不修改工作区生产文件。研究模块不加入 Cargo workspace，补丁中的实验入口、
环境变量与计数 API 不构成产品接口。生产采用需独立交付与设计同步。

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

从仓库根导出固定基线到全新目录，例如 `target/study-source`：

```powershell
git archive --format=tar --output=target/base-source.tar 46fdfaf47ae0c000ddc63420c8a71443baa8fb04
New-Item -ItemType Directory target/study-source
tar -xf target/base-source.tar -C target/study-source
python research/issue-757-current-scope/prepare.py target/study-source
$env:CARGO_INCREMENTAL='0'
$env:CARGO_TARGET_DIR='<独立构建目录的绝对路径>'
cargo +1.98.0 build --manifest-path target/study-source/Cargo.toml -p laneflow-urban-harness --release --locked --offline
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
./research/issue-757-current-scope/run.ps1 -Binary <封存程序> -Source target/study-source -OutputRoot target/scope-runs -Label 10k-a1 -Scale 10k -Mode all -Ticks 512
python research/issue-757-current-scope/analyze.py target/scope-runs target/scope-summary.json
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
超过 8 m/s² 的拍间减速次数。距离排除命令搬移；8 m/s² 仅作诊断阈值。
原 Harness 继续校验车辆守恒、资源归属与同边车身重叠，首次失败保留失败包。
同区多 owner 不等于碰撞；同边无重叠不证明完整二维安全；未完成行程不从报告中删除。
不宣称急减速阈值、公平性容差或交通质量已经获得产品认证。

`verify_analysis.py` 对本次已保存的第一臂运行施加八种损坏，确认分析器拒绝错误证据。
`aggregate.py` 是本次 28 个完整运行及一个明确中断尝试的封存清单，数量与排除项固定；
新的复测批次使用 `analyze.py` 独立报告，不追加到旧清单里改变历史统计。
