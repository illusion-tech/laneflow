| 场景              | 数据集                   | before µs |  after µs |     差值 |
| ----------------- | ------------------------ | --------: | --------: | -------: |
| adapter_full      | all_active_10000         |  2289.303 |  2526.281 |  10.352% |
| adapter_full      | all_active_100000        | 18108.425 | 17770.503 |  -1.866% |
| adapter_full      | high_completed_10000     |   253.356 |   247.153 |  -2.448% |
| adapter_full      | mixed_parking_10000      |  1268.341 |  1322.216 |   4.248% |
| adapter_full      | mixed_parking_100000     |  14204.85 | 14560.966 |   2.507% |
| adapter_full      | sparse_presentable_10000 |   635.869 |   613.234 |   -3.56% |
| alternate         | all_active_10000         |  3146.075 |  2604.253 | -17.222% |
| alternate         | all_active_100000        | 18534.844 | 17505.631 |  -5.553% |
| cold              | cold_probe               |       239 |     194.9 | -18.452% |
| fresh_output      | all_active_10000         |  4562.322 |  3023.038 | -33.739% |
| fresh_output      | all_active_100000        | 19584.709 | 19111.778 |  -2.415% |
| source_full       | all_active_10000         |   199.266 |   146.325 | -26.568% |
| source_full       | all_active_100000        |  3688.719 |  1936.128 | -47.512% |
| source_full       | high_completed_10000     |    74.928 |   107.334 |   43.25% |
| source_full       | mixed_parking_10000      |   432.853 |   376.769 | -12.957% |
| source_full       | mixed_parking_100000     |  5769.084 |  5185.806 |  -10.11% |
| source_full       | sparse_presentable_10000 |   653.769 |   629.912 |  -3.649% |
| transform_convert | all_active_10000         |      13.3 |    17.162 |  29.041% |
| transform_convert | all_active_100000        |   196.147 |   178.469 |  -9.013% |
| transform_convert | high_completed_10000     |     1.753 |     1.806 |    3.03% |
| transform_convert | mixed_parking_10000      |     8.603 |     8.772 |   1.961% |
| transform_convert | mixed_parking_100000     |    77.659 |    94.566 |   21.77% |
| transform_convert | sparse_presentable_10000 |     0.112 |     0.175 |  55.556% |

## 测量环境（由各 run 的 environment.json 汇总）

| 项                           | 值                                                                       |
| ---------------------------- | ------------------------------------------------------------------------ |
| CPU                          | AMD Ryzen 9 9955HX 16-Core Processor           （32 logical processors） |
| OS                           | Microsoft Windows NT 10.0.29661.0                                        |
| 工具链（取证记录）           | rustc 1.98.1 (48a229cea 2026-09-01)；cargo 1.98.1 (797e8a9bc 2026-08-05) |
| 电源方案                     | 电源方案 GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (平衡)              |
| before 生产基线              | b52f9ec4b792a158aed45f0ca4f536379073684f（features=legacy-source）       |
| after 生产基线               | 966411e54ee32e427c0d5aa2eae854bb17967fc6（features=默认）                |
| 测量程序 main.rs（A/B 相同） | BBF659CB2DDF57891914901C8E03B6DF8526E4CE5DA9FF93AA322AB87AAF183D         |
