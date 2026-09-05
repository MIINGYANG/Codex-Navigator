# 架构

codex-nav 是本机独立 Rust TUI，运行时没有网络客户端、数据库或模型依赖，也不启动或包装 Codex 进程。

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

初次读取在后台按批推进并显示字节进度。Viewer 只渲染当前视口，换行缓存按 Turn revision 和宽度失效。搜索对发生变化的 Turn 更新索引，不落盘。

文件更换通过 Unix dev/ino 或其他平台创建时间检查；长度缩小或已读前缀变化会重新加载。原 inode 的等长重写在 mtime 改变时也触发重载。该策略服务于 append-only rollout；无法保证识别所有刻意保留元数据的原地改写。

## 安全边界

生产代码不包含对 Codex 文件的写入、改名、删除、truncate 或修复操作。配置仅读取 codex-nav 自有路径，不自动生成文件。诊断不输出原始记录、认证配置、完整环境或内容日志。复制只在明确按 c/C 后发生，不使用 OSC 52；剪贴板不可用时提示并继续。
