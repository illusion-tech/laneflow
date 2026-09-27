# #707：封存无计时最佳候选

2026-09-22。状态：**封存 held。** 同一二进制、没有阶段计时。
`public_step_ns` 的 old 均值 30.517 ms，held 均值 28.462 ms，少 2.056 ms。
四臂短窗交通输出与 M3 逐字节一致，重叠为 0。16 ms 目标未达到。

合同见 [封存计划](untimed-best-plan.md)。M3 记录里的 29.334 ms 不改写。

## 1. 候选

`old` 是 M3 的配置：屏障跳过、稠密 binding，`LF707_COAST=off`、
`LF707_DEMAND=off`。`held` 在同样的屏障和 binding 上打开
`LF707_COAST=on`、`LF707_DEMAND=held`。Core 用 `public_step_ns`。
阶段计时列是 0。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 old、held、held、old。单位是毫秒。

| 臂     | Core 均值 | 标准差 |    p95 | observation 均值 | iteration 均值 |
| ------ | --------: | -----: | -----: | ---------------: | -------------: |
| old 1  |    30.511 |  2.032 | 33.809 |           62.623 |         96.005 |
| held 2 |    28.639 |  2.687 | 32.443 |           63.355 |         95.001 |
| held 3 |    28.284 |  1.975 | 31.692 |           62.591 |         93.694 |
| old 4  |    30.523 |  1.959 | 33.907 |           62.732 |         96.188 |

old 的均值是 30.517 ms，极差 0.012 ms。held 的均值是 28.462 ms。
两次 held 都低于两次 old，节省 2.056 ms，大于 old 的极差。
held 的 p95 是 32.443 和 31.692 ms。16 ms 未达到。

本文历史值先把两轮显示到三位小数后再平均，得到 28.462 ms；机器记录按两轮全部
384 个样本重算为 28.461283 ms。差异只来自中间舍入，不改变封存判断。

同一配置的 old 比 M3 记录的 29.334 ms 高 1.183 ms。这是今天的负载，
不是算法退步。29.334 ms 仍只属于那天的记录。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。重叠峰值是 0。

## 4. 封存

无计时最佳候选是 `held`。可复用身份由版本化 manifest、源码补丁和独立 Research
Evidence 制品共同绑定：

- manifest：[`retained-best-evidence-manifest.json`](evidence/retained-best-evidence-manifest.json)
- 源码：基线 `4de40e045398e4b010b2aa36522afc02a4094c4d` 加
  [`retained-best-source.patch.gz`](evidence/retained-best-source.patch.gz)；解压后明文补丁
  SHA-256 为 `c15fbaf8ffbf4cf0a61c72d2644804412846e9ab0ae2d29b860b85fd1c82abc7`
- 精确二进制：draft Research Evidence prerelease 的
  `issue-707-m16-best-19bf7ecb5dfd.zip`；草稿身份和上传摘要见
  [`retained-best-release.json`](evidence/retained-best-release.json)
- 运行输入：同一草稿的 `issue-707-m16-fixture-8b4294eb5ca8.zip`，保存 `input/`、
  `plan.toml` 与 `inventory.json`
- SHA-256：
  `19bf7ecb5dfd93a7f77c4deae10ffabf36f53fb70f13483d0a98cbb0f8992602`
- 环境：`LF707_COAST=on`、`LF707_DEMAND=held`、`LF707_BARRIER=skip`、
  `LF707_BINDING=dense`，以及 `approx/both/combined/boundary=on/decision=every`、
  `LF707_OBSERVE=fused`。不开 `short-profile`。

原 `target/issue707-m16-source` 和结果目录只保留历史采集位置，不再承担长期身份。
源码补丁已从基线提交的全新导出应用，并用 Rust/Cargo 1.98.0、release、locked、
offline、`CARGO_INCREMENTAL=0` 成功重建。不同绝对路径的重建 EXE 字节数相同但
SHA-256 不同，所以字节身份以 Research Evidence 包内精确 EXE 为准，不以重编译替代。

两个准备归档已在空目录解压，11 个 fixture 文件通过 manifest 摘要校验，精确 EXE
完成 4-worker、8-tick smoke；结果已进入版本化小封套。公开该 prerelease 后还须从
稳定 URL 再做一次定位、下载与 SHA-256 验证，下一次新候选才可继续使用这份 best；
before 是新源码关掉候选开关，after 是候选，顺序 best、before、after、after、
before、best。公开前状态是“已上传草稿、待发布”，不得只靠本机 `target/` 继续复用。

## 5. 证据

- 源码是已保留的近门判断，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的 49 文件 Rust/Cargo 补丁。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。没有 `short-profile`。
- 四轮 prefix/extended-quality 小封套已随 manifest 版本化；压缩包另含 timing、
  quality、gap/boundary/decision/entry 工作量和逐文件 `SHA256SUMS`。
- 原结果目录是 `target/issue707-m16-untimed-results/`。平衡电源方案，WPR 未在录制。
  旧驱动未保存每轮内存、AC 状态或厂商性能模式，这些字段明确为 null；未跑完整
  Runtime 测试套件，也不据此宣称 #707 正确性或最终性能认证完成。
