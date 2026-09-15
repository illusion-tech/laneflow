# Rust 代码风格

**文档状态**: Active<br>
**最后更新**: 2026-09-16<br>
**适用范围**: LaneFlow workspace 中的 Rust 源码、测试、基准和治理工具

## 1. 目标

本文记录 `rustfmt` 无法表达、但需要在 LaneFlow 中保持一致的 Rust 可读性约定。通用格式仍以 `rustfmt` 和 Clippy 为基础；本文件只补充仓库级规则，不替代 Rust 语言或工具链规范。

### 1.1 外部审阅参考

下列资料可以辅助 Rust API、常见模式与设计原则审阅，但不是 LaneFlow 的权威事实源，
也不得机械覆盖仓库 ADR、design、MSRV、性能证据或具体分层边界：

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/intro.html)
- [Rust Design Patterns：Design principles](https://rust-unofficial.github.io/patterns/additional_resources/design-principles.html)

审阅引用这些资料时仍须指出具体收益或风险，例如所有权、newtype/typed handle、
错误边界、API 可预测性、失败原子性或测试可验证性；不得只以通用原则名称要求抽象。

## 2. 十进制数字字面量

十进制字面量表达时间、数量、容量、规模、字节或其他可度量值时，整数部分达到四位即使用 `_` 从右向左按三位分组：

```rust
let tick_ms = 1_000;
let vehicle_capacity = 10_000;
let city_vehicle_count = 1_000_000;
let milliseconds_per_second = 1_000.0;
```

因此，同一语义下使用 `1_000`，不使用 `1000`。下划线只改善源码可读性，不改变数值、类型、性能或运行时输出。

负数的 `-` 是一元运算符，仍按其数值部分分组，例如 `-10_000`。带小数的字面量先按整数部分应用上述规则；较长小数部分只在分组能明显提升可读性时按三位分组：

```rust
let epsilon = 0.000_001;
let short_ratio = 0.125;
```

不要为短小数机械增加分隔符。

## 3. 其他进制

二进制、八进制和十六进制字面量应按位、半字节、字节或字段边界分组，不套用十进制的三位规则：

```rust
let byte_mask = 0b1111_0000;
let color = 0xFF00_FF00;
```

分组应表达值的结构；如果没有明确语义边界，优先采用常见的四位或八位分组，且同一上下文保持一致。

## 4. 适用边界与例外

以下内容可以保留惯用的不分组写法：

- 年份和日期组成部分，例如 `2026`；
- 端口、协议码、状态码、版本号或格式标识，例如 `8080`；
- 本身作为固定编号或外部规范 token 使用的值；
- 宏或测试数据中分组会掩盖领域结构的值。

例外应能从变量名、类型或邻近说明看出原因；无法判断时，优先使用分组写法。

本规则只约束 Rust 数字字面量，不要求修改字符串、日志、错误信息、JSON、Schema、文档示例或其他序列化数据。格式清理不得改变用户可见文本、持久化内容或测试所验证的运行时输出。

## 5. 注释与 rustdoc 契约段落

注释语言遵循 `docs/governance/documentation-policy.md` §6（中文优先）；本节规定注释的
**功能分层**与形态。核心原则：注释只写代码无法自表达的内容。可被类型系统或属性承载的
契约（`#[must_use]`、newtype、常量）不得降级为注释；注释承载类型表达不了的契约：
不变量、失败语义、成本契约与治理出处。

### 5.1 按受众分类

- **API 契约文档**（`///` 条目文档、`//!` 模块文档）：面向宿主与调用方，写规范语义、
  失败语义、性能契约与治理锚点。
- **实现约束注释**（`//` 行内注释）：面向维护者，写不变量、非显然的决策理由与成本
  注记；不写"下一行在做什么"的叙述。
- **诊断字符串**（错误 `Display`、`assert!` 消息、`#[must_use]` 文案）：见 5.3，
  三类受众不同，规则不同。

公共边界密、纯逻辑内部稀。crate 级文档可用 `#![doc = include_str!("../README.md")]`
让 README 兼作 rustdoc 首页。

### 5.2 rustdoc 契约段落

- 公开可失败 API（rustdoc 可见的公开函数或公开 trait 方法，返回 `Result`）必须有
  `# Errors` 段落，逐条对应实际失败路径；写不出准确条件时先读实现再写，宁缺毋错。
- 存在 panic 路径的公开 API 必须有 `# Panics` 段落；`debug_assert!` 不进入发布构建，
  不属于 panic 契约，不要求登记。
- 段落标题保持 rustdoc 标准英文（属政策 §6"工具字段明确需要英文"例外），段落正文
  中文。
- 既有散文描述的失败语义应收敛进段落，不保留重复表述。

### 5.3 诊断字符串

- 错误 `Display` 消息：中文、面向人。enum 变体名是稳定机器身份，宿主按变体匹配，
  不按字符串匹配。消息语言与日志编码的最终裁决保留给政策 §6 预留的专门 ADR。
- `assert!` 消息：短英文诊断标签是显式例外——它们与编译器、CI 输出混排，ASCII
  最可检索。
- `#[must_use = "..."]`：中文祈使句，写后果不写事实（如"丢弃记录会静默丢弃交付"）。
  公开返回类型的 `#[must_use]` 义务随包装类型继承，包装层不得丢失。

### 5.4 治理锚点与反模式

- 注释可锚定治理出处（如"（#534 G1 冻结）"）；契约被改写时，注释必须在同一变更内
  改写。代码中不留"原先如何"的考古注释，历史归 git。
- 注释使用 glossary 权威中文术语，精确标识符用反引号保留原文。
- 禁止：无 Issue 锚点的 TODO/FIXME（需要跟踪就开 Issue）；变更日志式注释
  （"改自 #x"）；注释掉的代码。
- 与数字字面量规则相同：本节只约束本次触及范围，历史欠账通过有界治理 Issue 清理，
  不在无关功能 PR 中顺带制造大范围注释 diff。

## 6. Retained-memory 计账

Core 的 retained-memory 测试账本按 Rust storage ownership 统计，不沿 handle 或
borrowed reference 重复统计目标对象。拥有 `Vec`、`String`、`IndexMap`、`Box`、
`Arc` 或其他 heap backing 的 Core 类型应在 crate-private、`cfg(test)` 的计账方法中
使用不带 `..` 的穷尽 struct 解构：

- 新增字段必须触发 test target 编译失败，迫使作者明确把字段分类为 owned heap、
  inline-only 或 non-owning reference/handle；
- nested container 同时统计 outer backing capacity 与实际拥有的 inner allocation；
- component 子指标只进入其 owner total 一次，不得在 world total 重复相加；
- world complete total 使用单一 component ledger 求和，测试和日志不得复制另一份
  独立公式。

每个新增 heap owner 至少需要一个使对应 component 非零的 smoke fixture；无该
owner 的场景应在可稳定判断时断言为零。常规 PR 运行 complete retained-memory
smoke，一万/十万 matrix 继续作为对应 Delivery/G3 的显式验证。

## 7. 工具与执行

- `rustfmt` 不负责统一数字分组，不能把 `cargo fmt` 通过解释为本规则已经满足。
- Clippy 的 `clippy::unreadable_literal` 可以发现部分较长字面量，但不覆盖本规则关注的四位数 `1000`，只能作为补充检查。
- 当前不使用全仓库正则 CI 强制本规则，避免把字符串、年份、端口和外部 token 误报为数字字面量问题。
- 注释规范同样无自动门禁：`missing_docs` 等 lint 不启用，`# Errors`/`# Panics` 完整性由审阅按 5.2 核对。
- 新增或修改 Rust 代码时，由作者在本次变更范围内遵守本文件；审阅者只对触及区域提出一致性要求。
- 历史不一致通过有界治理 Issue 清理，不应在无关功能 PR 中顺带制造大范围格式 diff。

Rust 对数字字面量下划线的语言语义见 [Rust Reference: Literal expressions](https://doc.rust-lang.org/reference/expressions/literal-expr.html)；Clippy 补充检查见 [`unreadable_literal`](https://rust-lang.github.io/rust-clippy/stable/index.html#unreadable_literal)。

## 8. Review 检查

Review Rust 变更时：

1. 先判断该 token 是否是真正的 Rust 数字字面量，而不是字符串或外部格式。
2. 对四位及以上的十进制度量值检查三位分组。
3. 对年份、端口、协议码等例外检查上下文是否足够明确。
4. 把纯格式问题限定在当前变更范围；历史问题应单独跟踪。
5. 不得把等价字面量格式评论提升为运行时、API 或数据格式缺陷。
6. Core owning struct 新增 heap-backed 字段时，确认 owner-local 穷尽计账、world
   component ledger 与零/非零 smoke fixture 已同步。
7. 对新增公开 API 核对 `# Errors`/`# Panics` 段落（见 5.2；两项独立判断——返回
   `Result` 的核对 `# Errors`，存在 panic 路径的核对 `# Panics`，与是否返回
   `Result` 无关）；对新增 `#[must_use]` 公开返回类型核对其义务在包装层未丢失
   （见 5.3）。
