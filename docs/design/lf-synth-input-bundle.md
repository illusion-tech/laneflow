# LF-SYNTH 规范输入交付

**文档状态**: Active  
**最后更新**: 2026-10-29

## 1. 职责与身份

本文冻结 `laneflow-synth-generator` 的离线输入文件编码。交通语义以
[`core-runtime-performance-baseline.md` §4](core-runtime-performance-baseline.md#4-canonical-workload)
的 `LF-SYNTH-v2` 为唯一权威。生成器交付输入；执行、实际命令回执、Spatial/Adapter
计时和五轴认证由后续 harness 承担，不从生成成功推出 Product Pass。

正式身份为 `(LF-SYNTH-v2, scale, case, fixed_delta_ms, seed=0)`；`scale` 仅为
`10000` 或 `100000`，`case` 为 `W1`–`W4`，步长为 `16` 或 `33` 毫秒。
所有组合均可生成；十万 16 ms 是 observation 输入。表现比例不进入交通输入身份。
不读取环境随机源、时钟、路径或主机信息，不保留旧工作负载的兼容入口。

## 2. 文件与精确字节

每个目录恰有四个普通文件：

- `network.lfca`：现行 LFCA，包括完整几何、信号、停车、车辆参数和显式政策。
- `state.json`：身份、稳定引用、路线与初始人口目录。
- `input-plan.json`：身份、窗口、固定 boundary 操作与事件驱动规则。
- `manifest.json`：身份、两个来源提交、规范配置摘要、归一化计数和三个文件记录。

JSON 为 UTF-8、无 BOM；对象键按 UTF-8 字节升序，数组保持定义顺序；无空白，末尾
恰有一个 LF。数字仅使用十进制整数，不含前导零、正号、指数或负零；距离用毫米、
速度用毫米每秒，时间用毫秒或 tick。字符串使用 JSON 转义规则，不转义 `/` 或
非 ASCII 字符。每份 JSON 的 `format` 是域分隔和编码版本，分别为
`lf-synth-state-1`、`lf-synth-input-plan-1`、`lf-synth-manifest-1`；配置摘要前像的
`format` 为 `lf-synth-config-1`。LFCA 使用自身现行域与版本。

输入的三个 SHA-256 分别覆盖对应文件的完整字节（含 JSON 尾 LF），输出为 64 位
小写十六进制。配置摘要覆盖 `{format, identity}` 的同一规范 JSON 编码。
这些都是 ADR 0033 的互操作摘要：外部复核者使用通用 SHA-256 工具校验实际交付
文件。生成模块头沿用 LFCA 家族的 SHA-256；参数摘要使用上述配置摘要，前端选项
摘要前像为 ASCII `lf-synth-frontend-1`。不存在运行时状态摘要或热路径哈希。

`source_commit` 与 `generator_commit` 只写入 manifest，均为 40 位小写 Git 提交
标识。前者记录交通设计来源，后者记录实际生成实现；调用者必须提供真实提交。
生成器不声称 SHA 校验能认证提交来源。文件中不写本机绝对路径、生成时间或测量耗时。

## 3. 静态网络与引用

编制命名空间固定为 `lf-synth`，文档键为 `lf-synth.document`，生成构建身份与
LFCA 发射身份均为 `laneflow-synth-generator-1`。实体键使用
`c{cell:04}-r{route}-{name}`；`name` 为 `entry`、`exit`、`junction`、`movement`、
`path`、`stop`、`group`、`gate` 或 `space{slot:02}`。每 cell 的 `facility`、
`controller` 和 `phase0`–`phase5` 使用 route 0。公共参与类型与车辆参数键为
`car`；政策键为 `protected-entry`；几何坐标框架键为 `world`。
稳定标识按编译器现行 `derive_canonical_stable_id_v1` 派生，输入目录使用
现行 `StableId<K>` 文本 `lfid1_<entity-kind>_<32 lowercase hex digits>`。
文件内顺序与编译器内部 ordinal 顺序分别处理，
消费方通过稳定标识解析 ordinal，不能把 logical rank 当成静态 ordinal。

按 §4.1 的整数毫米坐标生成 102 个完整中心线顶点，并在 progress 5130000 mm
插入分割点；entry/exit 各 52 点，合计每 cell 416 个几何点（边界重复）。
先以整数计算全局毫米坐标，最后一次转换为规范 f32 米；不重复浮点累加 1.2 m。
逻辑边长保持 5130 m；f32 几何量化不反向改变交通边长。
政策对所有已声明 gate 明确使用 `ProtectedGroup`，不靠扫描补默认通行规则。
使用现有 `single_network_1m_v2` 编译预算档：旧 `p100_initial_v1` 的静态实体与
几何预算不足以承载本合同的十万个停车位，不扩大生产全局硬上限。

编译后走 LFCA/LFSM/LFSD 发射及 `check_post_emission_bundle`，只交付已受检 LFCA。
LFSM/LFSD 为生成时验证材料，不进入本输入包；本工具不产生 LFCP 发布声明。

## 4. 初始状态目录

`state.json` 包含 `format`、`identity`、`policy_id`、`vehicle_profile_id`、`routes`、
`vehicles`。route 按 `(cell, route)` 升序，记录 `route_rank`、`cell`、`route`、
两个 `edge_ids`、`signal_group_id`。vehicle 按 logical rank 升序，记录
`logical_rank`、`route_rank`、`slot`、`status`（`active`/`parked`）、
`edge_occurrence`、`progress_mm`、`speed_mm_per_s`、`parking_space_id`。
`progress_mm` 是所在 edge 的 progress。停车位标识对全部 slots 都存在；只有
`parked` 表示初始 Occupied 绑定。logical rank 同时作为宿主管理的外部逻辑身份，
替换继承该值；Runtime handle 不写入输入文件。

## 5. 固定输入计划

`input-plan.json` 包含 `format`、`identity`、`warm_up_ticks`、`observation_ticks`、
`boundary_order`、`scheduled`、`rules`。所有操作按 cell 升序展开，数组位置即执行
顺序；`scheduled` 的 `boundary` 为调用 step 前的已提交 tick index，`commands`
包含 `operation`、`logical_rank`、`route_rank`、`edge_occurrence`、`progress_mm`、
`speed_mm_per_s`。不同 boundary 递增，同一 boundary 保持合同中的批次顺序。
每个 command 都使用同 rank 对应的停车位、路线和逻辑身份；不按运行时 handle 排序。

- W1/W2：`scheduled` 为空，`rules` 只声明禁止调用方变更和人口不耗尽。
- W3：boundary 0 为逐 cell `reserve_parking`；首个 observation boundary 为
  lifecycle probe 的逐 cell `despawn`/`spawn` 配对。`rules` 冻结 arrival barrier、
  下一 boundary 批量 park、一条成功 step 后 leave、按 rank 观察完成并在下一
  boundary atomic replace，以及每拍 parking commits → leaves → lifecycle 的顺序。
- W4：`warm_up_ticks-1` 为逐 cell 终点 `despawn`/`spawn`；`warm_up_ticks` 先全体
  atomic replace，再全体 parking leave；`warm_up_ticks+1` 为逐 cell transition
  probe 的 `despawn`/`spawn`。B0 未成功不得推进命令游标或执行 B1 命令。
- 事件驱动规则有明确的 rank 集合、参数、先决条件、错误条件和失败重试约束。
  不预写 arrival/完成的实际发生 tick，不把计划当执行回执。

## 6. manifest 与受检消费

manifest 记录 `format`、`identity`、`source_commit`、`generator_commit`、
`config_digest`、`counts`、`files`。计数包括 cells、individuals、routes、edges、
controllers、groups、gates、stop_lines、junctions、movements、maneuver_paths、
facilities、spaces、geometry_points、active、parked。文件记录按名称升序，包含
`name`、`byte_length`、`digest`，恰有三个输入文件；不做 manifest 自摘要。

验证入口拒绝非法身份、提交格式、缺失/额外文件、非普通文件、超过上限的文件、
计数/摘要/字节长度不符、非规范编码以及与同身份重新生成结果不同的任何字节。
文件上限分别为：manifest 64 KiB、计划 4 MiB、状态 64 MiB、LFCA 256 MiB；
它们是本工具读取边界，不修改 LFCA 格式硬上限或编译器预算。
验证会完整重新编译，是离线输入认证，不能放进计时窗口。重新生成与文件 SHA 的
双重验证能发现重新计算摘要后伪造的人口顺序或计划；不能替代对生成器实现的审阅。

测试必须覆盖固定编码向量、几何边界/长度/分割点、W1–W4 计数与顺序、两种步长、
两个独立进程的逐文件字节比较、篡改拒绝、LFCA 受检加载与 TrafficWorld 安装消费。
正式规模完整矩阵的证据应保存输入身份、三个摘要与文件长度；不把生成耗时作为
Runtime 性能结果。

固定测试向量见
[`probe-vectors.json`](../../tools/laneflow-synth-generator/fixtures/probe-vectors.json)：
2 cell、两种步长、W1–W4 各自的三个文件 SHA-256 与精确长度。该小规模只供内部回归，
正式命令行与公开生成 API 拒绝它。规范编码基础向量为 ASCII
`{"a":1,"b":2}` 加一个 LF，SHA-256 为
`e8d38819d39f705646bfb643368eca78f7db476c16471dbc33b941b27326410d`，已用
独立的 .NET SHA256.HashData 核对。
