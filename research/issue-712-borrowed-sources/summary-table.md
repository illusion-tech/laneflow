| 场景              | 数据集                   | before µs |  after µs |     差值 |
| ----------------- | ------------------------ | --------: | --------: | -------: |
| adapter_full      | all_active_10000         |  3126.453 |  2549.597 | -18.451% |
| adapter_full      | all_active_100000        | 15229.128 | 14574.734 |  -4.297% |
| adapter_full      | high_completed_10000     |   189.203 |   212.188 |  12.148% |
| adapter_full      | mixed_parking_10000      |  1042.266 |  1023.044 |  -1.844% |
| adapter_full      | mixed_parking_100000     | 11362.316 | 11191.972 |  -1.499% |
| adapter_full      | sparse_presentable_10000 |   529.231 |     590.8 |  11.634% |
| alternate         | all_active_10000         |  2662.928 |  2300.575 | -13.607% |
| alternate         | all_active_100000        | 15485.378 | 14448.869 |  -6.693% |
| cold              | cold_probe               |       204 |     188.7 |    -7.5% |
| fresh_output      | all_active_10000         |  3371.509 |  2572.706 | -23.693% |
| fresh_output      | all_active_100000        | 16033.753 | 15650.088 |  -2.393% |
| source_full       | all_active_10000         |   159.388 |   154.753 |  -2.908% |
| source_full       | all_active_100000        |  2749.756 |  2512.034 |  -8.645% |
| source_full       | high_completed_10000     |    67.819 |    81.181 |  19.703% |
| source_full       | mixed_parking_10000      |   371.259 |   326.494 | -12.058% |
| source_full       | mixed_parking_100000     |  4634.341 |  4323.691 |  -6.703% |
| source_full       | sparse_presentable_10000 |   545.694 |   556.134 |   1.913% |
| transform_convert | all_active_10000         |    11.666 |    15.616 |   33.86% |
| transform_convert | all_active_100000        |   152.803 |   150.969 |    -1.2% |
| transform_convert | high_completed_10000     |     1.606 |     1.453 |  -9.533% |
| transform_convert | mixed_parking_10000      |     6.041 |     6.462 |   6.984% |
| transform_convert | mixed_parking_100000     |    69.972 |    69.162 |  -1.157% |
| transform_convert | sparse_presentable_10000 |     0.159 |     0.153 |  -3.922% |

## 测量环境（由各 run 的 environment.json 汇总）

| 项                           | 值                                                                       |
| ---------------------------- | ------------------------------------------------------------------------ |
| CPU                          | AMD Ryzen 9 9955HX 16-Core Processor           （32 logical processors） |
| OS                           | Microsoft Windows NT 10.0.29667.0                                        |
| 工具链（取证记录）           | rustc 1.98.1 (48a229cea 2026-09-01)；cargo 1.98.1 (797e8a9bc 2026-08-05) |
| 电源方案                     | 电源方案 GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (平衡)              |
| before 生产基线              | b52f9ec4b792a158aed45f0ca4f536379073684f（features=legacy-source）       |
| after 生产基线               | 298daed89e521f17de00afcf9b1951824494753a（features=默认）                |
| 测量程序 main.rs（A/B 相同） | 8503A4DCBFD7A0207558EDEDFAE6302D41D40A07AD5983BC53097A57A51AC2AE         |
