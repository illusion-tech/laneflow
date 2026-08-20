# mcpls 可选开发工具

**文档状态**: Active<br>
**适用范围**: 在 Windows Codex Desktop 中对 LaneFlow Rust workspace 做只读语义导航

## 1. 定位与边界

LaneFlow 把 mcpls 作为可选的本地开发工具，用于通过 `rust-analyzer` 提供 hover、
定义跳转、引用查找、文档符号、工作区符号搜索和诊断。它不属于 LaneFlow 运行时、
Cargo workspace 依赖、CI 门禁或发布产物，也不替代 `cargo check`、`cargo test`、`rg`
和源码阅读。

Windows 正式拓扑为“每个 Git worktree 一个原生 Streamable HTTP mcpls”：

- 同一 worktree 的多个 Codex 任务和 MCP session 复用一个 mcpls 进程及其
  `rust-analyzer`；
- 不同 worktree 使用不同 worktree ID、进程、loopback 端口、状态目录和
  `rust-analyzer`，不共享每进程全局的 LSP 状态；
- mcpls 只监听 `127.0.0.1`，不绑定局域网或公网地址，也不提供远程服务；
- mcpls 以对应 worktree 为工作目录，并显式读取该 worktree 的 `mcpls.toml`；其中
  `workspace.roots = []` 继续从当前工作目录发现 workspace；
- Codex setup 通过 Windows 内置 CIM/WMI `Win32_Process.Create` 创建直接 mcpls PID，
  使常驻服务不再属于 Codex 的 `KILL_ON_JOB_CLOSE` 进程树。该路径不注册计划任务或
  Windows Service，也不需要提权或自研 supervisor。

仓库跟踪 `.codex/config.template.toml`，不跟踪动态 endpoint。Windows setup 在每个
worktree 本地生成被 `.gitignore` 忽略的 `.codex/config.toml`。生成文件包含管理标记、
schema version 和模板 SHA-256；模板变化会在下次 setup 时可见地重新生成。脚本拒绝
覆盖没有管理标记的既有配置，也不使用 `skip-worktree`、`assume-unchanged` 或其他隐藏
Git 变更的机制。

当前开发 Agent 宿主边界如下：

| 宿主              | 状态        | 边界                                                                                                     |
| ----------------- | ----------- | -------------------------------------------------------------------------------------------------------- |
| Windows Codex App | `supported` | Local Environment setup 调用受管 `Ensure`；首次生成配置后，当前任务未加载 mcpls 时仍需新建任务或 Restart |
| Grok Build        | `deferred`  | 已确认能读取项目级 HTTP MCP，但没有进入本轮 Delivery 的可靠阻塞 setup 生命周期                           |
| Kimi Code         | `deferred`  | 已确认能读取项目级 HTTP MCP，但项目配置只在新 session 生效，且生命周期入口未进入本轮验收                 |

`deferred` 表示保留未来薄适配接缝，不表示当前已支持。未来适配必须读取同一个 worktree
状态和 endpoint，不能为同一 worktree 再启动第二个 mcpls。本轮不跟踪
`.grok/config.toml` 或 `.kimi-code/mcp.json`。

## 2. 每台 Windows 电脑的一次性安装

固定安装 mcpls `0.3.9`，并显式启用非默认的 `transport-http` feature：

```powershell
cargo install mcpls --version 0.3.9 --locked --features transport-http --force
```

确保 `rust-analyzer` 可通过 PATH 启动；使用 rustup 时可执行：

```powershell
rustup component add rust-analyzer
```

验证版本和 HTTP 参数：

```powershell
mcpls --version
mcpls --help
rust-analyzer --version
```

`mcpls --version` 必须返回 `0.3.9`，帮助中必须同时出现 `--listen` 和 `--http-path`。
setup 也会执行相同检查，并验证 `rust-analyzer --version` 可执行，避免启用无法提供 Rust
语义能力的服务。

setup 脚本不会联网、安装、升级或下载任何工具。mcpls 缺失、版本不符、没有 HTTP
feature，Windows CIM/WMI 创建被策略拒绝，或 Git worktree / `%LOCALAPPDATA%` context
无法构造时，`Ensure` 都会给出警告并成功结束；能安全定位受管模板时同时生成禁用配置。
LaneFlow 的其他开发工作不受影响。
若 context 失败且无法安全定位/验证受管配置，结果会把 `config_enabled` 报为 `null` 并
明确说明禁用未完成，不能声称旧 endpoint 已禁用。人工 `Start` 则以非零状态严格失败。

当前固定工具的来源为 crates.io，上游为 `bug-ops/mcpls`，许可证表达式为
`MIT OR Apache-2.0`。该本机工具不进入仓库依赖图或分发物；如果未来需要自动下载、
进入 CI 或随产品分发，必须另建治理任务并按 `docs/governance/dependency-security.md`
重新审计来源、校验和、许可证、安全公告和分发影响。

## 3. Codex Desktop Local Environment setup

在 Codex Desktop 的 Windows Local Environment setup 中配置以下命令：

```powershell
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Ensure
```

Codex 为新任务创建 worktree 时会运行 setup。`Ensure` 会：

1. 规范化 `git rev-parse --show-toplevel` 结果，保留文件系统返回的真实大小写，并派生
   SHA-256 worktree ID；
2. 按持久游标轮转、有界检查历史状态；自动清理不会因为单次 HTTP 探测失败而停止有效
   worktree 的已归属服务，也不会执行 HTTP 探测；Git 与 CIM 检查、锁等待和进程终止
   共用本次 setup 的启动截止时间；
3. 在 `%LOCALAPPDATA%\LaneFlow\mcpls\worktrees\.locks\` 下，以禁止共享打开的锁文件
   实现跨 Windows 会话的同 worktree 串行化和全局端口分配串行化；
4. 从 `41000..48999` 按 worktree ID 确定性选择 loopback 端口，并在共享截止时间内线性
   探测完整 8000 端口范围；
5. 同时验证 PID、进程启动时间、可执行路径、命令行中的 `mcpls.toml`/endpoint、
   `mcpls.toml` 内容 SHA-256 以及 HTTP MCP `initialize`；配置内容变化会停止已归属的旧
   服务并启动新服务；解析到的 `rust-analyzer` 路径变化也会阻止复用旧服务；健康探测会
   携带协商得到的 `MCP-Protocol-Version` 删除临时 session，并把删除失败视为不健康；
6. 用 `StartupTimeoutSeconds` 的单一截止时间约束初始 Git worktree discovery、自动清理、
   二进制校验、锁等待、CIM 创建、端口绑定和 HTTP 健康检查；
7. 使用 `Win32_Process.Create` 和 `CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW` 以当前用户、
   目标 worktree 为工作目录创建直接 mcpls PID；不再用普通 `Start-Process` 启动常驻后代；
8. 进程创建后立即原子持久化 `starting` 状态，再等待端口绑定；只有健康检查、启用配置
   与生命周期日志全部提交成功后才保留新进程。任一记账步骤失败都会回收该进程，且失败
   路径只在持有同 worktree 锁时改写禁用配置。进程在 bind 前自行退出会直接中止启动，
   不会遍历其余端口；只有仍存活但未绑定时才尝试下一候选端口。

直接在 Codex setup 中给 `CreateProcess` 增加 breakaway flag 的实机探针仍被外层嵌套 Job
捕获，因此不是支持路径。Windows Task Scheduler 虽能异步托管进程，但会增加持久系统
注册和卸载面；在 CIM/WMI 创建路径已通过真实 mcpls HTTP 初始化后，本轮不采用它。

Codex 官方文档没有保证 Local Environment setup 一定早于当前任务的 MCP 配置读取。
因此首次生成配置后，如果当前任务没有加载 mcpls，应 Restart Codex 或新建任务；不要
把当前任务热重载当作保证。修改模板后也采用相同处理；修改 `mcpls.toml` 后，下次
`Ensure`/`Start` 会重启 worktree 服务，但当前任务是否重新读取生成配置仍遵循上述边界。

Local Environment 的 UI 设置属于用户本机配置，不是仓库单一事实源。仓库只维护脚本、
模板和本文档。只应对自己已经审阅并信任的 checkout 启用项目配置和 setup；
`mcpls.toml` 能决定脚本显式启动的语言服务器命令，不能把不受信任分支当作安全输入。

## 4. 生命周期命令

所有命令都在目标 LaneFlow worktree 根目录执行：

```powershell
# setup 使用：失败时禁用 mcpls，但不阻断任务创建
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Ensure

# 人工严格启动或复用：失败时返回非零
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Start

# 输出 worktree、endpoint、PID 身份、健康和 rust-analyzer 后代数量
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Status

# 仅在完整身份和 MCP 健康均匹配时停止该 worktree 服务及其后代
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Stop

# 清理 root 已不存在、已不是有效 Git worktree 或服务已失效的状态
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Prune
```

`Ensure` 和 `Start` 在同一 worktree 中串行或并发调用都必须复用同一个健康 PID 和
endpoint；锁文件采用 `FileShare.None`，因此不同桌面会话或终端也进入同一临界区。
`Stop` 不按进程名批量停止；PID、启动时间、可执行路径、命令行或 HTTP 健康任一不满足
时都会拒绝普通停止，进程树未在期限内确认退出时也不会写入已停止状态。`Prune` 先校验
“状态目录名 = `worktree_id` = 规范化 root 哈希”，
再判断进程身份；三者不一致、状态损坏或 schema 不支持时都拒绝停止和删除并保留证据。
人工 `Prune` 对有效 worktree 的失效服务做两次 HTTP 探测后才可停止；`Ensure` 内部的
自动清理不探测、不停止有效 worktree 的已归属存活服务。只有结构化身份仍匹配，或记录
PID 已确认不存在且状态所有权一致时，才清理失效状态。CIM/WMI 或元数据检查失败与 PID
不存在是不同状态：前者失败关闭并保留记录，不能触发替换或清理。有效 worktree 的配置
无法安全禁用时，`Prune` 会保留 state 并返回拒绝原因，不会丢失死 endpoint 的诊断证据。
若记录的 root 目录仍存在但已无法通过 Git worktree 校验，`Prune` 也会先尝试通过受管
模板禁用其中的配置；禁用失败时同样保留 state。root 已不存在时才跳过配置写入。

本地状态位于：

```text
%LOCALAPPDATA%\LaneFlow\mcpls\worktrees\<worktree_id>\
```

`state.json` 当前为 schema 2，记录 `launch_method=win32_process_create`、规范化 root、PID、
启动时间、可执行路径、命令摘要、端口、endpoint、模板哈希、`mcpls.toml` 内容哈希、
解析到的 `rust-analyzer` 路径和状态；
`lifecycle.log` 记录本脚本的启动、复用、失败、停止与清理事件。两者都不提交到仓库。
状态、轮转游标和生成配置都
通过同目录临时文件原子替换。旧 schema 或不完整状态会被明确报告为 `invalid-state`，
不会被当作“无状态”而启动第二个服务。必填值还会校验非空、类型、范围、绝对路径、
哈希格式以及 endpoint/端口一致性；成功停止或已确认回收的启动失败分别使用
`status=stopped|failed` 与 `process_id=0`，明确表示没有活动 PID，避免系统复用旧 PID
后阻断下次启动。需在核对对应 PID 后人工处理无效状态目录。

外部创建路径不继承 Codex setup runner 的输出管道，也不为 stdout/stderr 增加常驻
wrapper。支持的可观察事实源是 `state.json`、`lifecycle.log`、进程身份和 HTTP MCP 健康；
如果这些证据不足以定位新问题，应另行扩展诊断，不在本脚本中引入生产级监管器。

## 5. 验证

先运行不依赖 mcpls/Pester 的脚本契约测试：

```powershell
pwsh -NoLogo -NoProfile -File .codex/test-setup-mcpls-worktree.ps1
```

setup 完成并 Restart 或新建任务后，检查 Codex 解析结果：

```powershell
codex mcp get mcpls
```

应确认：

- transport 为 HTTP，URL 与 `Status` 的 loopback endpoint 一致；
- `required` 为 `false`；
- `enabled_tools` 只包含 `get_hover`、`get_definition`、`get_references`、
  `get_document_symbols`、`workspace_symbol_search` 和 `get_diagnostics`；
- rename、format 等写能力没有启用。

`Status` 中 `identity_matched` 和 `healthy` 应同时为 `true`。首次 Rust 语义查询需要等待
`rust-analyzer` 完成 workspace 初始化；冷启动期间的空结果不能解释为没有定义。可让
Codex 查询当前 worktree 中已知的 `CoreWorld` 定义和引用，并核对返回路径确实属于当前
worktree。至少两个 worktree 并行验收时，还应确认它们的 worktree ID、endpoint、PID、
`rust-analyzer` 后代和语义查询路径彼此不同。

mcpls 返回的源码行号按一基编号（1-based）解释，不再额外加一。职责分工保持为：

- mcpls：类型感知的 hover、定义、活动配置引用、符号和诊断；
- `rg`：字面量、配置、注释、未启用 feature 和快速文本搜索；
- `cargo check`、`cargo test`：编译、测试与最终正确性依据。

## 6. 迁移、回退与跨平台边界

旧原型或旧 checkout 如果对 `.codex/config.toml` 设置过 `skip-worktree`，先显式恢复
Git 可见性：

```powershell
git update-index --no-skip-worktree -- .codex/config.toml
git ls-files -v -- .codex/config.toml
```

随后审阅并移动或删除旧的本地 STDIO/原型配置，再运行 `Ensure`。脚本不会替用户覆盖
没有管理标记的配置。不要重新设置 `skip-worktree`；共享变化应通过
`.codex/config.template.toml` 的正常 Git diff/review 进入每个 worktree。

PR #435 生成的 schema 2 状态没有 `launch_method` 时仍可读取，但只视为
`start_process_legacy`，不能复用。下一次 `Ensure` 会先按完整 PID、启动时间、可执行路径、
命令行和 HTTP 身份停止旧服务，再以 `win32_process_create` 重建；若旧 PID 已不存在则直接
恢复，若仍存活但身份不匹配则失败关闭并拒绝启动第二个实例。

需要临时停用 HTTP 服务时运行 `Stop`；生成配置会保留但标记为 `enabled = false`。
mcpls 完全不可用时，继续使用 `rg`、源码阅读和 Cargo 检查。卸载本机工具前应先逐个
worktree 执行 `Stop` 或在审阅 `Status` 后执行 `Prune`，再运行：

```powershell
cargo uninstall mcpls
```

本 Issue 的受管 HTTP 生命周期首轮只支持 Windows PowerShell 7。Linux/macOS 上 mcpls
仍为可选工具：可以不启用，或在被忽略的本地 `.codex/config.toml` 中手工配置 STDIO；
后者仍是每任务进程，不具备本文的 worktree 级复用保证。跨平台 HTTP 生命周期需要
独立后续任务，不应把 Windows 脚本描述为已经跨平台。

参考：

- [OpenAI Docs：Codex Config basics](https://learn.chatgpt.com/docs/config-file/config-basic)
- [OpenAI Docs：Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
- [OpenAI Docs：Codex Local environments](https://learn.chatgpt.com/docs/environments/local-environment)
- [OpenAI Docs：Codex Hooks](https://learn.chatgpt.com/docs/hooks)
- [Microsoft：Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [Microsoft：Win32_Process.Create](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/create-method-in-class-win32-process)
- [Grok Build：MCP Servers](https://docs.x.ai/build/features/mcp-servers)
- [Kimi Code：MCP](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/mcp.md)
- [mcpls 0.3.9 文档与元数据](https://docs.rs/crate/mcpls/0.3.9)
- [mcpls source](https://github.com/bug-ops/mcpls)
