# 已撤回候选的成功性与资源验证对应

Refs #834。下表绑定候选 `d7624fb21da11566e8a4a4a624da3e84c45937c9`，
不表示当前产品启用了筛选；[普通测量](../admission-distant-results.md)未显示可重复收益。
候选的完整源码与测试保存在该提交及证据包 B.tar 中，诊断补丁另外绑定该提交。

## 证明到实现


`placement::add_proven_empty_spawn_contender` 只由完整重建调用。权利排除继续使用完整
owner 的 `ConflictRead::has_authority`，并单独排除停车绑定、机动穿越和等待 membership。
`tick::admission_preview_stays_inside` 保留受检路线/profile、同边严格位移上界及原可失败校验。

| 成功性义务                              | 原语或受检不变量                                                                                            | 验证入口                                                                                                                                 |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| 当前路线、hop、边长、限速与剩余项均存在 | 完整句柄、`route_hop` 及结果位置必读列；缺失回退                                                            | `admission_preview_proof_preserves_success_and_rejects_unproved_states`                                                                  |
| IIDM、量化与严格同边结果有定义          | `MotionReach` 域；受检 profile 的有限数值范围；零有效期望速度走原零目标分支；travel 与 next-speed 上界      | `admission_proposal_extrema_remain_finite_and_inside_motion_reach`                                                                       |
| 降速首项查询成功且两个循环首项退出      | `admission_speed_drop_proof` 受检切片、目标转换/加法、原分段距离；仅 Finite 且原 `f32` 距离严格大于共享窗口 | `admission_distant_drop_uses_strict_float_window_and_preserves_both_loops`、`admission_distant_drop_rejects_unknown_suffix_and_distance` |
| 前车查询窗、信号/门及路径包络不吞错     | 保留 `leader_query_horizon` 与 `speed_limit_path_envelope_from`；无停车绑定；信号/门只收紧房间              | 原准入边界合同与受检路线完整预览参考对拍                                                                                                 |
| 空贡献、owner、有效身份和命令结果不变   | 原 `contender_notes` / `apply_contender_notes`、预留与增量路径不变                                          | `admission_distant_drop_matches_full_cache_on_checked_routes`、`admission_distant_drop_preserves_commands_faults_and_restored_cache`     |
| 无新增常驻容量或失败后存量差异          | 原 owner/外层表容量账本，参考路径仅在测试夹具中禁用筛选                                                     | `admission_filter_allocation_account` 分别覆盖空后缀与有限远处的冷/暖重建及故障重试                                                      |

空后缀与有限远处由 `AdmissionSpeedDropProof` 分开表达；计数仅在测试/夹具构建中存在，
不新增产品字段或逐车计时。上表说明实现与证明的对应，不替代实际执行测试与测量结果。


## 受检场景与失败记录

在原空后缀测试之外，有限远处首项使用三类受检路网：纯等待区 Synthetic DSL，
道路编辑来源的冲突路网，以及冲突与等待区组合。内部后继边降低限速，普通上游边
提供实际可命中的远处距离；不通过放松身份、权利或运动 guard 来制造命中。
对入口与机动路径 occurrence、零/中间/近边末/边末进度、三档步长和速度逐项比较完整缓存。
另比较连续公共命令、LFRS 字节、事件、返回值、失败重试、槽位复用、替换与恢复。

首轮组合夹具被编译器以 waitingZoneOverlap 拒绝；将冲突通行段放在等待释放门后
使夹具满足既有语义。一个测试断言误把全部 cursor=1 和近边末状态当成必须回退，
修正为真实边界条件；矩阵中的全部状态仍与完整参考逐项对拍。产品 guard 未因这些失败改变。
原始失败与成功复跑见证据包 logs 和 validation-notes.md。

## 资源边界

小夹具分别测量合法空后缀与有限远处首项的冷态构建、64 次暖态重建及外层故障/重试。
候选与完整参考分配统计、保留容量、状态和完整缓存一致。空后缀冷态 4 次/736 B，
有限远处冷态 2 次/384 B；两类暖态均零分配，保留容量分别 736 B、384 B。
这不是十万负载完整分配账本；大负载诊断只另给每拍缓存容量与宿主内存采样。
