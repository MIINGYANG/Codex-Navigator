# 架构

codex-nav 是本机独立 Rust TUI / Web Sidecar，无数据库、云服务或模型依赖，也不启动或包装 Codex 进程。默认运行 TUI；只有显式 --web 才启动本机 HTTP 服务。

```text
只读 rollout → bounded JSONL → Parser → dirty Turn 更新 → App → ratatui
                    ↑                         ↑
          后台线程 / notify / 100 ms 轮询      键盘 reducer
```

## 模块

- discovery：CODEX_HOME、日期目录、有限 header 解析、索引尾部辅助、cwd 排序。热路径仅扫描近期日期，--all 才遍历完整日期层级。Picker 的发现结果统一过滤为 MAIN；底层发现保留全部类型供显式 --session 解析，避免过滤影响 ID/path 诊断入口。默认始终等待用户选择，不自动打开。
- parser：JSONL 有界缓冲、归一化、Turn 分组、去重、工具活动、可靠状态和 rollback。UI 不接触 serde_json::Value。
- watch：独立读取线程；有界通道最多排队两批。dirty 集合仅传递发生变化的 Turn。文件通知作为提示，100 ms 轮询作为可靠后备；只追加时不重读历史。
- app / index：键盘状态、搜索索引、历史选择保护、可见文本缓存。
- ui / util：宽窄布局、帮助与状态栏；所有非可信文本移除终端控制序列，按 grapheme 和显示宽度处理中文及 emoji。
- main：终端恢复 guard、后台发现、增量更新、剪贴板和 doctor。退出时终止 Navigator 自身 worker。

## 内存及 I/O 边界

默认记录上限 4 MiB，可配置 1 KiB–64 MiB。读取缓冲 64 KiB；未完成的超大记录进入丢弃状态，不累积其后续内容。单条工具/回复正文最多 64 KiB（截断标记另计），单 Turn Prompt 最多 256 KiB。Prompt 与活动分别使用 16 MiB / 48 MiB FIFO 正文预算：新内容入队时释放旧正文，旧 Prompt 保留预览与 omitted_bytes，活动槽位替换为零正文 Omitted，索引不移动。淘汰必须 touch 旧 Turn，后台 dirty 增量同步 UI 与搜索，保留历史选择和计数。最多保留 100,000 Turn 与 200,000 活动，条数上限仍是硬限制。省略量出现在界面诊断，原始文件保持完整。总进程内存还包含预览、索引、去重缓存、结构开销和最多两批更新，因此高于正文预算。

会话身份从首条 session_meta 解析，使用独立 saw_meta 标记，即使首条缺 ID 也不被继承历史覆盖。来源不明保留 UNKNOWN。最终回复由显式 phase 或 completion 确认；镜像记录升级原活动的 phase，不重复添加。Viewer 缓存持有结构化最终回复行锚点，重排时重新计算，避免用户正文伪造标题影响定位。

TurnStatus 仅记录生命周期：未结束、正常完成、明确执行错误、中断、未知、rollback。activity.errors 单独保留活动错误事实，工具输出不能覆盖生命周期，轮次终止事件不计入工具错误；Timeline 将状态符号与 !N 分列。正文只呈现证据，结果正确与否由用户查看最终回复判断。

初次读取在后台按批推进并显示字节进度。Viewer 只渲染当前视口，换行缓存按 Turn revision 和宽度失效。搜索对发生变化的 Turn 更新索引，不落盘。

文件更换通过 Unix dev/ino 或其他平台创建时间检查；长度缩小或已读前缀变化会重新加载。原 inode 的等长重写在 mtime 改变时也触发重载。该策略服务于 append-only rollout；无法保证识别所有刻意保留元数据的原地改写。

## 安全边界

生产代码不包含对 Codex 文件的写入、改名、删除、truncate 或修复操作。配置仅读取 codex-nav 自有路径，不自动生成文件。诊断不输出原始记录、认证配置、完整环境或内容日志。复制只在明确按 c/C 后发生，不使用 OSC 52；剪贴板不可用时提示并继续。

## Web 支持边界

v1.2.0 实现 B「专注阅读」网页，`--web` 在终端初始化前进入独立服务路径。Axum/Tokio 管理 HTTP，静态资源编译内嵌；浏览器不需要构建工具，不访问 Codex 文件系统。

Web 复用 discovery、domain、parser、watch 和 Prompt 搜索 index，通过独立 DTO 对外提供主会话列表、加载进度、目录分页、活动分页和最终回复。状态线程与 HTTP 请求隔离，限制同时打开的会话缓存；每个 SessionWorker 保留既有流式读取与正文预算。浏览器轮询轻量 revision/generation 元数据，只在变化时获取内容；重载与会话切换不复用过期响应，阅读历史不会被新轮次抢走。

每个进程缓存最多两个打开会话，登记最多 20,000 路径；API 最多四个进行中的响应，许可直到流式响应消费/丢弃才释放，避免慢读大正文积压。最多 32 个连接，每连接 60 秒期限；Ctrl+C 最多等待三秒让 HTTP 退出，再停止读取线程。目录每页最多 100，活动每页最多 8；客户端只保留最多 64 个活动节点，刷新当前窗口按页串行重取一致 turn revision。会话 revision 用于发现变化，turn revision 用于判断是否需要替换正文，不能混用。

仅绑定 127.0.0.1，随机访问令牌通过 URL fragment 进入页面，以专用请求头认证 API；静态资源不嵌入令牌。校验 Host/Origin，禁止 CORS，设置 CSP/nosniff/防嵌入及不缓存策略。HTTP 只能选择服务登记的会话 key，不能传入本机文件路径；--session 的显式路径授权只来自本地 CLI。页面使用安全 DOM/Markdown 子集，不执行原始 HTML，不自动加载远程图片。

公开静态页面允许从其他页面正常导航进入，不携带任何会话数据或令牌；API 则始终拒绝 cross-site 请求。拒绝所有跨站顶层导航会让合法启动链接打不开，不能将公开 shell 与受保护的数据接口混为一谈。

继续只读 Codex 文件，不引入云端、登录、遥测或 AI API。令牌是防跨站访问的本机能力凭证，不是面向不可信远程用户的账号系统；具有本机进程、浏览器扩展或终端访问权的其他程序不在隔离范围内。远程/公网访问是另一个安全范围，不能仅把监听地址改成 0.0.0.0。API 与布局约定见 [Web 实施约定](web-implementation.md)。
