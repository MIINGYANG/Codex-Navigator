# /btw 侧聊留存：接入验证

2026-09-17，Codex CLI 0.153.2。此文记录隔离合成环境的结果，**不是已实现的采集功能**。正式启动方式需要用户选择；当前版本只提供已有落盘会话的阅读与收藏。

## 已确认的限制

普通 Navigator 监听文件，无法获取只存在于 Codex 内存里的临时分支。独立启动一个 app-server 也不会连接到现有 CLI 的内存。

即使连到承载 CLI 的同一个 app-server，单独初始化的旁观连接也不能读取临时侧聊正文：

| 隔离测试 | 结果 |
| --- | --- |
| 第二连接 initialize 后监听 | 能收到 thread/started，含 ephemeral、forkedFromId，turns 为空 |
| thread/loaded/list | 包含临时线程 |
| thread/read 元数据 | 可读取 |
| thread/read includeTurns:true | ephemeral threads do not support includeTurns |
| thread/turns/list | ephemeral threads do not support thread/turns/list |
| thread/resume，包括创建者和 excludeTurns:true | no rollout found |
| thread/subscribe | 没有该方法 |
| 临时线程执行合成 shellCommand | 创建者收到 turn/item 正文事件，旁观者只收到状态变化 |

隔离 TUI 恢复人工构造的两轮会话后，输入裸 `/btw`，没有发送 `turn/start` 或模型请求。实际 `thread/fork` 使用 `ephemeral:true`、`threadSource:"user"`、`excludeTurns:true`，但 `lastTurnId` 和 `beforeTurnId` 都是 null。之后的 `thread/inject_items` 是内部侧聊边界提示，尽管标记为 user，也不能当作用户侧聊内容展示。

## 可选实现路线

通过官方 `codex --remote unix://…` 让原生 CLI 显式使用 Navigator 本地连接入口；Navigator 双向转发原协议，并将侧聊内容另存为自身副本。不是 PTY 包装，不更改 Codex 源码，不自行生成或提交模型 turn，但需要用户调整启动方式，不能宣称打开 Navigator 就能捕获任意现有进程。

本机 Unix 传输实测为 HTTP Upgrade 后的 WebSocket frames。实现须覆盖 masked/unmasked、分片、控制帧、消息上限，副本解析/写盘失败不得吞掉或修改原请求与审批。连接变为中间环节也意味着它的生命周期会影响客户端连接，正式实现需明确断开恢复策略。

按连接与 JSON-RPC 请求 ID 关联 fork 请求/响应，获得父子线程身份；父问题位置只在有明确证据时记录。默认 /btw 无精确父轮次字段，应区分显式锚点、发起位置快照与待确认位置，不能按时间猜测。

若保留现有启动方式，可使用手动导入并指定挂靠问题，或通过 `/fork` 建立可恢复的正式 Codex 分支；这些是不同的操作体验，不自动替用户选择。

## 验证边界

全部 PoC 使用隔离 CODEX_HOME、合成会话和自建服务；没有访问真实会话、修改真实配置或请求模型。验证用服务和终端均已退出。功能实现还需要协议、持久化、断线缺口和界面的自动回归，不能以预览通过替代。

官方说明：[命令](https://learn.chatgpt.com/docs/developer-commands?surface=cli)、[App Server](https://learn.chatgpt.com/docs/app-server)、[Hooks](https://learn.chatgpt.com/docs/hooks)。接口和行为以支持的 Codex 版本为准。
