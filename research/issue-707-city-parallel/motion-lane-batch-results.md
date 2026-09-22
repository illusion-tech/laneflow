# #707：运动按车道边成批

2026-09-22。状态：**淘汰。** 分发慢了 2.159 ms，准备阶段多了 12.656 ms。
四臂交通输出逐字节一致，重叠为 0。16 ms 目标未达到。P5 的布局尝试结束。

合同见 [按车道边成批计划](motion-lane-batch-plan.md)。保留组合仍是
frontier 按 hop 记住。

## 1. 候选

`off` 按 Active 序号分发。`lane` 先读出每辆车的车道边序号，只排序下标，
再按这个顺序分块。不在排序比较里搬动车辆状态。消费仍按 Active 序号。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 off、lane、lane、off。单位是毫秒墙钟。每一臂都开着 frontier 缓存、
屏障跳过、稠密 binding 和阶段计时。

| 臂 | Core 均值 | p5 准备 | p5 分发 | 运动循环 |
| --- | ---: | ---: | ---: | ---: |
| off 1 | 30.577 | 0.877 | 6.440 | 9.247 |
| lane 2 | 53.042 | 13.216 | 8.646 | 31.414 |
| lane 3 | 53.418 | 13.832 | 8.565 | 32.134 |
| off 4 | 30.499 | 0.859 | 6.453 | 9.269 |

p5 分发的 off 均值是 6.446 ms，极差 0.013 ms，lane 均值是 8.605 ms。
p5 准备从 0.868 ms 增到 13.524 ms。Core 大约多 22.7 ms。

取出车道边序号要为每辆车打开路线。这一次查找落在准备阶段。分发按车道边
挨在一起之后，前车查询仍会走到后续边，占用桶没有因此留在缓存里。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 相同，重叠峰值
都是 0。

## 4. 淘汰

研究组合不变，源码仍以 `target/issue707-m8-source` 为准。本轮原型在
`target/issue707-m12-source`。没有进入正式 PR。

热行、按路线边排序、按车道边排序，三次都没有让分发变快。P5 的布局尝试
结束。下一次若继续做性能，回到冲突准备里 frontier 缓存还没吃掉的部分，
以及 p3 准备。

## 5. 证据

- 父源码是第 1 刀，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `cf3482be21bef2a7ae80e248f1a6026eba6ea6158c012df760b0b7e797bb8c4e`。
- 结果目录：`target/issue707-m12-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
