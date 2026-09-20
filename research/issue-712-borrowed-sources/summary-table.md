| 场景 | 数据集 | before µs | after µs | 差值 |
| --- | --- | ---: | ---: | ---: |
| adapter_full | all_active_10000 | 2436.844 | 2367.925 | -2.828% |
| adapter_full | all_active_100000 | 15312.497 | 13977.369 | -8.719% |
| adapter_full | high_completed_10000 | 199.038 | 195.05 | -2.003% |
| adapter_full | mixed_parking_10000 | 1092.897 | 1002.078 | -8.31% |
| adapter_full | mixed_parking_100000 | 11218.859 | 10965.112 | -2.262% |
| adapter_full | sparse_presentable_10000 | 540.325 | 533.172 | -1.324% |
| alternate | all_active_10000 | 2497.756 | 2146.65 | -14.057% |
| alternate | all_active_100000 | 16010.356 | 14348.678 | -10.379% |
| cold | cold_probe | 193.3 | 160.1 | -17.175% |
| fresh_output | all_active_10000 | 3055.719 | 2340.975 | -23.39% |
| fresh_output | all_active_100000 | 16218.878 | 15094.219 | -6.934% |
| source_full | all_active_10000 | 420.706 | 126.975 | -69.819% |
| source_full | all_active_100000 | 2466.197 | 2043.344 | -17.146% |
| source_full | high_completed_10000 | 74.828 | 65.084 | -13.022% |
| source_full | mixed_parking_10000 | 364.45 | 288.6 | -20.812% |
| source_full | mixed_parking_100000 | 4636.272 | 4043.072 | -12.795% |
| source_full | sparse_presentable_10000 | 504.691 | 519.094 | 2.854% |
| transform_convert | all_active_10000 | 14.088 | 11.984 | -14.929% |
| transform_convert | all_active_100000 | 200.809 | 143.031 | -28.773% |
| transform_convert | high_completed_10000 | 1.162 | 1.166 | 0.269% |
| transform_convert | mixed_parking_10000 | 8 | 6.262 | -21.719% |
| transform_convert | mixed_parking_100000 | 69.906 | 73.259 | 4.797% |
| transform_convert | sparse_presentable_10000 | 0.116 | 0.119 | 2.703% |

## 测量环境（由各 run 的 environment.json 汇总）

| 项 | 值 |
| --- | --- |
| CPU | AMD Ryzen 9 9955HX 16-Core Processor           （32 logical processors） |
| OS | Microsoft Windows NT 10.0.29667.0 |
| 工具链（取证记录） | rustc 1.98.1 (48a229cea 2026-09-01)；cargo 1.98.1 (797e8a9bc 2026-08-05) |
| 电源方案 | 电源方案 GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (平衡) |
| before 生产基线 | b52f9ec4b792a158aed45f0ca4f536379073684f（features=legacy-source） |
| after 生产基线 | 2b85e9fa5dcb5c8037ae7677fb481ef1ebf8d7e0（features=默认） |
| 测量程序 main.rs（A/B 相同） | 275810F50F5BEE03518468324DFADE40D6AEA7FD3DD6F1AFAC85A35E81936A0A |
