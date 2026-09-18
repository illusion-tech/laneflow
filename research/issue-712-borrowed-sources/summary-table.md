| 场景              | 数据集                   | before µs |  after µs |     差值 |
| ----------------- | ------------------------ | --------: | --------: | -------: |
| adapter_full      | all_active_10000         |  2125.888 |  1981.134 |  -6.809% |
| adapter_full      | all_active_100000        | 15205.378 | 14273.588 |  -6.128% |
| adapter_full      | high_completed_10000     |  1480.066 |  1371.194 |  -7.356% |
| adapter_full      | mixed_parking_10000      |  1052.588 |   972.441 |  -7.614% |
| adapter_full      | mixed_parking_100000     | 12822.322 | 11905.216 |  -7.152% |
| adapter_full      | sparse_presentable_10000 |   476.962 |     489.6 |    2.65% |
| alternate         | all_active_10000         |  2147.747 |  2014.344 |  -6.211% |
| alternate         | all_active_100000        | 15336.572 | 14606.128 |  -4.763% |
| cold              | cold_probe               |       184 |     162.3 | -11.793% |
| fresh_output      | all_active_10000         |  2847.706 |  2522.647 | -11.415% |
| fresh_output      | all_active_100000        | 15792.638 | 15419.275 |  -2.364% |
| source_full       | all_active_10000         |   400.759 |    125.65 | -68.647% |
| source_full       | all_active_100000        |  2361.672 |  1924.838 | -18.497% |
| source_full       | high_completed_10000     |   174.556 |   124.956 | -28.415% |
| source_full       | mixed_parking_10000      |   306.744 |   320.222 |   4.394% |
| source_full       | mixed_parking_100000     |  5005.584 |  4241.038 | -15.274% |
| source_full       | sparse_presentable_10000 |     469.7 |   471.881 |   0.464% |
| transform_convert | all_active_10000         |    11.212 |    11.328 |   1.031% |
| transform_convert | all_active_100000        |   121.497 |   135.828 |  11.796% |
| transform_convert | high_completed_10000     |    11.419 |    11.119 |  -2.627% |
| transform_convert | mixed_parking_10000      |     5.778 |     5.734 |  -0.757% |
| transform_convert | mixed_parking_100000     |    60.909 |    62.372 |   2.401% |
| transform_convert | sparse_presentable_10000 |     0.112 |     0.112 |       0% |

## 测量环境（由各 run 的 environment.json 汇总）

| 项                           | 值                                                                       |
| ---------------------------- | ------------------------------------------------------------------------ |
| CPU                          | AMD Ryzen 9 9955HX 16-Core Processor           （32 logical processors） |
| OS                           | Microsoft Windows NT 10.0.29661.0                                        |
| 工具链（取证记录）           | rustc 1.98.1 (48a229cea 2026-09-01)；cargo 1.98.1 (797e8a9bc 2026-08-05) |
| 电源方案                     | 电源方案 GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (平衡)              |
| before 生产基线              | b52f9ec4b792a158aed45f0ca4f536379073684f（features=legacy-source）       |
| after 生产基线               | dc5a3685575e3488fb34d09622253c8150e3e4b5（features=默认）                |
| 测量程序 main.rs（A/B 相同） | D5D7D08B9AF63BD66A8B5EDF6BB5B2B02366176C2302AC0F3F3552B5C83064D3         |
