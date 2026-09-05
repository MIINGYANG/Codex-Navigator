# v1.0 验收记录

日期：2026-09-06。环境：本机 Linux x86_64，Rust/Cargo 1.98.1，Codex 样本版本 0.153.2。

## 自动检查

| 检查 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 通过，零 warning |
| `cargo test` | 91 passed，0 failed |
| `cargo build --release` | 通过 |
| `cargo build --release --examples` | 通过 |
| `python3 scripts/terminal_qa.py target/release/codex-nav` | 通过 |
| `codex-nav --help` / `--version` / `doctor` | 通过，版本 1.0.0 |

测试细分：parser 33、增量读取 10、discovery/config/CLI 定义 20、state/search/render 19、后台 worker 7、CLI 进程 2。全部使用合成内容或临时测试目录。

## 终端及真实数据

- 使用伪终端启动被测试的 Navigator binary；产品本身没有 PTY 包装代码，也从不启动 Codex。
- 已验证中文/多行 Prompt、搜索 Enter、历史查看时新 Turn 提示、G 最新、帮助、Viewer/Timeline 切换、120 列到 70 列及 1×1 后恢复。
- 已验证剪贴板不可用时提示、空会话 Picker、q 和 Ctrl+C 后 termios 与 alternate screen 恢复。
- 在真实本机 Codex 会话上打开、导航、resize、Ctrl+C；终端内容未保存或输出至验收日志。
- 三份真实历史 rollout 分别约 10.2 MiB、16.9 MiB、0.38 MiB；解析没有 malformed/oversized，SHA-256 前后一致。大文本的可见内容按预算截断并报告。
- 当前活动会话只读观察 25 秒：新增 13,776 bytes，0 次 reset。本轮观察没有人类再提交新 Prompt；新 Turn 自动跟随与历史保护通过真实线程追加合成 fixture 验证。
- 用户原 Codex 进程持续正常运行，本次实现未改动其源码、配置、rollout 或 session_index。

## 性能

以下为本机 release 运行实测，未清理操作系统文件缓存，不代表冷磁盘或所有机器的保证。

| 场景 | 结果 |
|---|---|
| 56,791,806 bytes / 4096 Turn 合成会话 | 解析 173.28 ms；312.56 MiB/s |
| 上述会话索引 | 0.71 ms |
| 中文 / substring / fuzzy / 无匹配搜索 | 最慢 0.204 ms |
| 256 MiB 超大单行后接正常 Prompt | 33.90 ms；跳过 1 行；后续 1 Turn 正常 |
| 整个合成基准峰值 RSS | 62,384 KiB（约 60.9 MiB） |
| 本机 doctor / 30 个近期 rollout | 0.05 s；峰值 RSS 6,216 KiB |

## 发布产物

`target/release/codex-nav`，Linux x86_64，约 2.9 MiB。

SHA-256：`a7c71d49de9fce385414dec7992f62f6386ef2ae64d41336c629238bc537fcf1`

```bash
./target/release/codex-nav
./target/release/codex-nav doctor
cargo install --path . --locked
```

## 适用边界

- macOS / Windows 使用跨平台库，但本次没有对应平台的构建与交互验证。
- 超大记录与总可见文本按 README 所述预算跳过/截断，不承诺展示原始 rollout 全部内容。
- 未知协议事件忽略；缺乏可靠完成证据时保持中性状态。rollout 不是稳定官方 API。
- 无 newline 的最后一行等待后续提交，包括静态文件。复杂原地改写若同时保留身份、前缀、长度与时间戳，无法可靠检测。
- v1.0.0 验收时尚未初始化 Git 或配置 GitHub remote，因此当时未添加或发布 GitHub Release workflow；本地 release binary 已完成。

## v1.0.1 — 2026-09-06

修复正文 g/G 在宽窄布局切换后仍错误选择 Turn 的问题。实现按焦点导航，同步帮助和状态栏提示，并初始化 main 分支的本地版本历史。

- 新增 4 个导航回归测试：先验证旧实现失败，再验证正文顶部/底部可见、历史选择及提示保留、宽→窄→宽、Shift+G、空/短/极小视口、搜索输入。
- 全量 `cargo test`：95 passed，0 failed。
- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release` 均通过。
- 终端验收脚本增加长正文场景；70/122/70/120 列反复切换后，g 显示 USER，G 显示正文末尾，始终保留 TURN 02 和 +1 new turn。
- `target/release/codex-nav --version` 返回 1.0.1。退出旧 Navigator 后重新运行该 binary 即可使用新版本。
- 本地版本标签：v1.0.0 基线、v1.0.1 修复。未配置远程、未执行 push；仍不修改任何 Codex 数据。

## v1.1.0 — 2026-09-06

三项需求已实现并验证：最新 Prompt 滚动保留、会话来源辨认、f 最终回复定位。

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features -- -D warnings`：通过，零 warning。
- `cargo test`：121 tests 通过（新增 26 项），包含 parser、discovery、state、incremental、worker、CLI 和宽窄渲染。
- `cargo build --release` 及 release examples：通过；`target/release/codex-nav --version` 返回 1.1.0。
- `python3 scripts/terminal_qa.py target/release/codex-nav`：通过；新增 f、completion phase 升级、MAIN/WATCHING、无 final 提示及到期消失，保留 g/G、搜索、历史保护、剪贴板降级、q/Ctrl+C 和 termios 恢复验收。
- 真实只读验收：MAIN 12,794,851 bytes / 10 Turn / 9 条 final；SUBAGENT 838,023 bytes / 5 Turn / 4 条 final，父会话字段存在。两份历史文件 SHA-256 不变，均无 malformed/oversized；真实 MAIN 终端打开、resize、导航、Ctrl+C 通过，未记录内容。
- 合成 56,791,806 bytes / 4096 Turn：解析 175.23 ms，309.08 MiB/s；索引 0.66 ms，最大搜索 0.186 ms；滚动保留正文 50,590,239 bytes，最新回复保留。
- 256 MiB 超大单行流式跳过：40.95 ms，之后 Prompt 正常；该基准进程峰值 RSS 57,420 KiB（并非所有工作负载的内存保证）。

已知边界：Prompt 单轮 256 KiB；默认单记录 4 MiB；100,000 Turn / 200,000 活动仍为硬上限。超限旧正文仅保留预览/省略标记，不做磁盘分页恢复；f 只定位仍保留的可靠 final。16 / 48 MiB 是正文预算，不是总进程内存上限。Windows/macOS 未在本机实测。

## v1.1.1 — 2026-09-06

- 默认先选主会话，不再自动打开唯一候选；启动、s 返回和 r 刷新的列表均只包含 MAIN，--all 仅扩展日期范围。
- 替换旧自动选择策略测试，新增显式非主会话解析测试；全量 122 tests 通过。
- fmt、clippy（--all-targets --all-features，零 warning）、release build、git diff --check 通过；binary 返回 codex-nav 1.1.1。
- `python3 -B scripts/terminal_qa.py target/release/codex-nav`：5 组全部通过。新入口测试先在旧 release 复现自动打开错误，新版覆盖唯一 MAIN 选择、Enter、s、r、隐藏来源搜索、--all、仅非主空态、显式子会话 path/ID 不重定向父会话。
- 保留导航/搜索/历史保护/实时追加/resize/gG/final f/剪贴板降级/q/Ctrl+C/终端恢复回归，合成 rollout 前后 SHA-256 不变；本轮无需修改或读取真实 Codex 数据。
- 来源缺失或不认识的 UNKNOWN 不显示在默认列表；需要查看时使用 --session ID/PATH。未改变历史正文预算与平台限制。

## v1.1.2 — 2026-09-06

- 旧实现红测复现：工具输出非零退出码后收到正常 task_complete，状态仍为 Failed。修复后只保留独立活动警告，正常完成显示 ✓。
- 新增 6 项 parser/state 回归：失败→重试→完成、明确执行错误与中断、completion 自带错误、晚到工具输出不覆盖生命周期且不抢选择、rollback 保护、仅有最终回复文本不推断完成。
- 新增 4 项 UI 回归：全部生命周期与 !N 分离、24/60/120/140 列布局、正文正确性免责声明、窄屏帮助图例。
- `cargo test`：132 tests 全部通过。`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、release build 和 diff 检查通过。
- `python3 -B scripts/terminal_qa.py target/release/codex-nav`：6 组通过，新增 ✓ !1 / … !1 / ⊘ / ✕、正文 TURN STATUS、f 最终回复、宽窄切换和免责声明验收，原有主会话 Picker / gG / 退出恢复全部保留。合成源文件 SHA-256 不变。
- 本机只读抽样 12 份 rollout，确认 task_complete / turn_aborted 的实际结构，未输出内容。Web 目前不支持直接查看，仅记录核心可复用与缺少 HTTP/API/前端的边界，未创建服务或网络监听。

## v1.2.0 — 2026-09-06

- B「专注阅读」成为正式 Web，资源内嵌 binary；默认 TUI 不变，只有 --web 才启动 loopback HTTP。
- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`、`cargo build --release --locked` 通过。157 tests，包含 11 个 Web 单测、13 个真实二进制 HTTP 集成；Linux x86_64 release 约 3.9 MiB，版本返回 1.2.0。
- `npm run format:check`、`npm run lint`、`npm test` 通过：Prettier / ESLint 零错误，18 个纯状态/请求代次/活动窗口/安全 Markdown 测试通过。Node/npm 仅开发时需要。
- `node scripts/web_qa.mjs target/release/codex-nav` 与追加 `--no-watch` 均通过：实际浏览器 1280×793 / 390×793，共四组。覆盖默认主会话、子代理隐藏、会话与 Prompt 搜索、105 轮分页、活动 64 条窗口与前后组、整轮复制及剪贴板拒绝降级、f/g/G、帮助、临时 503 自动恢复、历史新增独立计数、DOM/展开分页/滚动位置保持、无 final 提示、无远程资源与恶意 HTML/链接不执行。无监控时不自动读取，手动刷新能取回异步更新。
- 截图自审修正长 Prompt 作为会话标题占满侧栏的问题，标题取首行并限长，桌面目录独立滚动；保留 B 的轻目录、宽正文和最终回复书签。截图仅包含合成数据。
- 安全测试先复现公开顶层导航被错误拦截，再将公开 shell 与受保护 API 分层；API 的跨站、Host/Origin、缺失/重复/错误令牌、方法与正文限制不放宽。覆盖任意路径/符号链接外跳、显式 CLI 授权、两会话缓存换代、流式大响应许可、慢连接期限及有未完成请求时的 Ctrl+C。
- `python3 scripts/terminal_qa.py target/release/codex-nav` 六组全部通过。初次暴露既有 harness 两个竞争：同尺寸重排误清空模拟屏幕，实际重排后立即发键竞争 SIGWINCH/TTY 就绪。修正为同尺寸保持屏幕、等待真实帧结束确认；原内容断言不变。连续五次完整复验及主线程最终复验通过，另验证搜索输入可见光标的重排确认；未修改 Rust/TUI 业务源码。
- 再次只读抽样本机 12 份会话头部，确认 cli/subagent、消息镜像与 final_answer 实际形态。正式浏览器列出 14 个真实 MAIN；主会话 API 加载成功。显式打开一个静态历史 SUBAGENT（12 轮），SHA-256 前后不变，未输出 Prompt/回复。所有 Web/终端合成 fixture 均核对预期字节不变。

边界：仅本机访问，不支持公网/局域网共享；Markdown 为安全子集，不加载图片附件，复杂表格/公式可能保持原始文本。沿用正文滚动预算与省略机制，不能恢复被淘汰历史全文。每进程最多两个打开会话、20,000 已登记会话，API 目录 100 / 活动 8 条每页，浏览器活动窗口 64 条；预算不等于总 RSS。Windows/macOS 与其他浏览器未在本机验收。
