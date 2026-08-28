# Runtime Snapshot v1（`LFRS`）

运行时快照容器的 wire schema（#302 G1 快照合同 §4；#512 切片 B）。size-prefixed
FlatBuffers、file identifier `LFRS`、`format_version = 1`。生成物隔离于私有
[`crates/laneflow-runtime-snapshot-wire`](../../crates/laneflow-runtime-snapshot-wire)，
clean regeneration 由
`cargo +1.98.0 run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>`
校验（固定 flatc 25.12.19）。

生产 writer 位于 `laneflow-runtime::encode_lfrs`：先由
`TrafficWorld::capture_snapshot` 在固定步进安全边界捕获不可变逻辑状态，再离线编码；
writer 不回读活动 world、不推进命令游标，并逐字段写入完整 `WorldConfig`（包括
`route_edge_occurrence_capacity`）、LFCA origin、已提交来源、路线/车辆表与 live 顺序。

本 schema 只冻结 wire shape。局部标识唯一性、引用闭包、排列精确性、停车绑定
一致性、值与时钟不变量、禁绑字段与版本轴核对由 Runtime 语义 lowering 执行
（`Unspecified = 0` 枚举值与未知枚举值在 lowering 拒绝，不在 verifier 层）。

## 版本轴（合同 §2）

| 轴             | 字段                                | 语义                                                                                                                                                    |
| -------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 容器格式       | `format_version`（id 0）            | wire shape 版本；未知值失败关闭                                                                                                                         |
| 逻辑状态形状   | `runtime_state_version`（id 1）     | 被绑定的 Runtime 逻辑状态语义版本，与容器格式分离                                                                                                       |
| 静态契约版本集 | `static_contract_versions`（id 11） | 六轴：canonical format / identity encoding / identity registry revision / network revision derivation / constraint contract / static execution contract |

## 根表字段映射（`RuntimeSnapshot`，field id 连续 0..=16）

| id  | 字段                        | 绑定内容                                                                                                                                                                    |
| --- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0   | `format_version`            | 容器格式版本（=1）                                                                                                                                                          |
| 1   | `runtime_state_version`     | Runtime 逻辑状态形状轴（=1）                                                                                                                                                |
| 2   | `world_id`                  | 世界身份（快照局部）                                                                                                                                                        |
| 3   | `tick`                      | tick 游标                                                                                                                                                                   |
| 4   | `time_ms`                   | 时钟；lowering 核对 `time_ms == tick × fixed_delta_time_ms`                                                                                                                 |
| 5   | `command_cursor`            | 输入命令游标（已应用命令计数）                                                                                                                                              |
| 6   | `event_cursor`              | 已提交事件游标（v1 无事件通道，恒零）                                                                                                                                       |
| 7   | `world_config`              | `WorldConfig` 全量（含 `route_edge_occurrence_capacity`，#303 G1 已接受合同，运行时面随 #521 落地）；恢复核对按 §2 两分（dt 精确相等 / 语义容量只许放大 / worker 数不参与） |
| 8   | `network_revision`          | LFCA origin 四联之一：`NetworkRevisionId`                                                                                                                                   |
| 9   | `lfca_artifact_digest`      | LFCA origin 四联之二：规范制品摘要（来源审计，非语义兼容门）                                                                                                                |
| 10  | `lfca_artifact_byte_length` | LFCA origin 四联之三：exact byte length                                                                                                                                     |
| 11  | `static_contract_versions`  | LFCA origin 四联之四：静态契约版本集                                                                                                                                        |
| 12  | `source_kind`               | `CommittedNetworkSource` 封闭种类；v1 仅 `Published`                                                                                                                        |
| 13  | `source_published`          | `PublishedLfcaReference`（`asset_key` / digest / length / revision）                                                                                                        |
| 14  | `routes`                    | 路线表：`snapshot_route_id` + 有序边 `StableId128` 序列（允许重复边）                                                                                                       |
| 15  | `vehicles`                  | 车辆表：局部 ID、局部路线引用、`route_edge_index`、毫米状态、status、profile/class `StableId128`、可选停车位                                                                |
| 16  | `live_order`                | `snapshot_vehicle_id` 的规范排序序列（lowering 核对为活跃车辆精确排列）                                                                                                     |

## 禁绑字段（合同 §2，出现即属违规编码）

runtime handle / slot / generation、密集序号（`LaneEdgeOrdinal` 等）、共享静态
数组内容、`EditableDiffBase`、partition / worker assignment、数组地址 / layout /
capacity、调用方自有随机流。派生状态（信号灯色、车道占用索引、profile 派生
车长、compiled 出现项）不入容器。

## 恢复判据（合同 §5）

同修订判据 = `NetworkRevisionId` + `networkRevisionDerivationVersion` + 契约版本
精确相等；origin 字节差异仅承担来源审计，published 重发布的摘要错配在判据满足
时允许恢复。

生产 reader 为 `laneflow-runtime::restore_lfrs`：framing 与 file identifier 先于
有界 verifier；verifier 先于领域分配；版本/绑定/配置与根表基数先于语义 lowering。
确认两个 v1 版本轴后，reader 逐 table 核对 vtable 字段数，拒绝 root / config /
published source / route / vehicle 超过本 schema 登记数的未知槽；FlatBuffers 的默认
forward-compatible verifier 不承担这条禁绑字段义务。
恢复在不可见的局部 world 内完成，路线统一经 `register_admitted_route`，车辆与停车
复用共同 Runtime 不变量，最终重建派生信号/占用并把命令游标还原为捕获值；任一失败
不返回半个 world。
