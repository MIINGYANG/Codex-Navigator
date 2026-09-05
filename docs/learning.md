## 2026-09-06 — 本机会话驱动的只读归一化
**Question:** 如何让独立 Sidecar 兼容实际 Codex rollout，而不污染时间线或改写源文件？
**Key insight:** 本机 0.153.2 的人类输入主要来自带 content_item_kinds 的 response_item 和 item_completed，不能只实现旧 user_message。应复用同一解析器处理发现与 Viewer，并通过来源、邻近距离、Turn ID 去重；completion 也是去重边界。
**Details / snippet:** Phase 0–4 已通过 parser、discovery、state、incremental、worker 自动测试；真实历史文件只读检查前后 SHA-256 一致。补充回归确保结构化 exit_code 优先于正文字符串，迟到 item 按 envelope turn_id 归属。
**Tags:** #codex #parser #readonly #verification

## 2026-09-06 — 有界实时读取与交付验收
**Question:** 如何在处理超大会话和实时搜索时同时保证响应性与可验证的只读行为？
**Key insight:** 记录大小、保留文本、索引身份字段和后台交付队列都需要明确上限；加载进度不能只依赖已提交记录的 revision，因为超大半行到 EOF 时可能没有新记录。搜索选择必须按 Turn 身份保留，文件截断或 rollback 后取消搜索也必须回到有效选择。
**Details / snippet:** Phase 5–7 验收通过：91 tests、fmt、clippy、release、终端恢复、真实会话只读跟随。合成约 54 MiB / 4096 Turn 解析 173.28 ms；256 MiB 超大单行跳过成功，基准峰值 RSS 62,384 KiB。原始历史文件 hash 不变。
**Tags:** #tui #streaming #memory #release #verification
