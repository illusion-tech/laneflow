# LF-SYNTH-v2 正式输入生成验证

## 验证边界

2026-10-29，在 Windows x86-64、Rust 1.99.1 上验证输入生成切片。
交通合同与生成实现均绑定
`9029c021d1f10a7506eedbf5f2fae3b5a7d57629`。本记录不认证 Runtime 性能、完整
W1–W4 执行窗口、停车 barrier、表现层或五轴结果。

完整矩阵为 `10000/100000 × W1/W2/W3/W4 × 16/33 ms`，共 16 个正式输入身份。
每组启动两个全新 release 生成进程，逐字节比较四文件，再重新编译校验输入目录，
并通过生产 API 消费。**16/16 通过，无重试**。共比较 64 对文件；48 个输入文件摘要
另用 PowerShell `Get-FileHash -Algorithm SHA256` 独立重算，全部与 manifest 相等。

机器可读记录：

- [完整矩阵](matrix-summary.json)：全部身份、计数、三个摘要、精确长度和验证结果，
  `complete=true`。
- [外部工具摘要复核](external-digests.json)：48 个文件的通用 SHA-256 核对结果。
- [首轮部分记录](partial-977b7ebe.json)：`977b7ebe` 的 8/16 行，`complete=false`；
  该轮未通过，不是正式交付结果。

原始双份输入与运行日志保留在本机 `E:\projects\laneflow-evidence` 下的
`issue-1156-9029c021` 和 `issue-1156-9029c021-run.log`；首轮失败制品保留于
`issue-1156-977b7ebe` 与对应日志。目录位于工作树外，清理工作树不会删除证据。
仓库只保存摘要记录，完整制品可按固定提交和工具 README 重建。

## 消费路径与断言

每个正式身份均检查：

- 生产编译、LFCA/LFSM/LFSD 后发射检查；目录中只交付 LFCA。
- LFCA 受检读取、保留完整 Spatial 的共享路网构建。
- 归一化边、控制器、信号组、gate、停止线、路口、movement、path、设施和停车位计数。
- 每条边长精确 5130000 mm、速度上限 13900 mm/s；entry 只连接同 route 的 exit，
  exit 无后继；全量几何点与冻结输入相等。
- 16/33 ms 的六阶段时长、周期和逐 cell offset；W4 使用统一同步 offset。
- 稳定标识解析到本修订 ordinal，显式政策安装，所有路线注册，全部车辆按目录顺序
  spawn/spawn_parked；推进一拍后人口总数与 active/parked 比例保持正确。

这是一拍可消费性检查；没有执行 W3 预留命令或 W4 warm-up/burst，因此不替代后续
执行 harness 对实际输入计划、事件因果和完整窗口的验证。

| 对象                   |      一万输入 |        十万输入 |
| ---------------------- | ------------: | --------------: |
| cells                  |           100 |            1000 |
| 路线                   |           400 |            4000 |
| 有向边                 |           800 |            8000 |
| 停车设施               |           100 |            1000 |
| 停车位 / logical slots | 10000 / 10000 | 100000 / 100000 |
| 几何点（含边界重复）   |         41600 |          416000 |

W1/W4 初始 75% active、W2 初始 100% active、W3 初始 25% active；其余为 parked。
输入包仍标记为 `LF-SYNTH-v2`；没有将十万 16 ms observation 输入宣称为产品主行。

## 几何失败与修正

首轮在一万 8 个身份全部通过后，十万 W1/16 的提交边长断言失败。直接把解析毫米
坐标转换为各位置的最近 f32 点，会让不同坐标区域的段长累加产生不同舍入；ADR 0028
要求编译器从规范弧长提交交通毫米边长，不能强行采用编制声明 5130 m。

修正采用最近 `1/1024 m` 二进制格点（输入包设计 §3），最大坐标误差 <0.489 mm；
原点、端点、分割点、拓扑、人口和命令不变。全部 1000 cells 的轴向段长和逐段 f32
累计均精确可表示，提交边长恢复为 5130000 mm。新增数值回归覆盖全部 cells；
编译器、Runtime、Spatial 的生产 API 和算法均未改动。

## 复现与其他检查

复现入口与四个环境变量见
[工具 README](../../tools/laneflow-synth-generator/README.md)。使用上述固定提交构建
release 二进制，将 source/generator commit 均设为该提交，证据根设为一个新目录，
运行 `full_scale_independent_process_matrix`。原始文件无时钟、本机路径或随机输入。

- `cargo +1.99.1 test --locked -p laneflow-synth-generator`：6 passed，正式矩阵测试
  在默认运行中按显式标记忽略，已另行运行通过。
- release 下同 6 项测试通过；包括 2 cell × 8 组固定文件向量、规范 JSON、非法身份、
  全范围几何与重算摘要后的篡改拒绝。
- `cargo +1.99.1 clippy --locked -p laneflow-synth-generator --all-targets -- -D warnings`：通过。
- release 构建及 CLI `generate` / `verify` / `--help`：通过。
- `cargo deny --locked --all-features check advisories bans licenses sources`：通过；沿用
  已锁定依赖版本，仅增加本工具依赖边和 SHA-256 直接依赖方登记。
- fmt、差异空白与提交 hook 检查通过。完整 workspace 回归由 PR CI 承担；本地未运行
  无关 crate 的完整测试。

全部耗时只用于执行过程诊断；未采集 Runtime 性能分位数或内存峰值，不形成性能结论。
