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

## 2026-09-06 — 按焦点导航与本地版本迭代
**Question:** 如何修复宽窄终端切换后的正文 g/G，并保留后续开发的版本时间线？
**Key insight:** 布局宽度和导航焦点是两个独立状态；g/G 应与 j/k 一样按焦点分发，正文首尾跳转不能触发 Turn 选择或清除历史提示。以现有版本建立基线 commit/tag，每次完成验收的改动保存独立提交，并同步语义版本号与 CHANGELOG。
**Details / snippet:** v1.0.0 基线 e9fd58a；v1.0.1 修复新增 4 个导航回归测试，先复现后修复；全量 95 tests、fmt、clippy、release、伪终端宽窄切换及正文首尾验收通过。查看版本：git log --graph --decorate --oneline --all。
**Tags:** #keyboard #focus #git #versioning #verification

## 2026-09-06 — 最新输入保留与可靠最终回复定位
**Question:** 如何解决旧输出耗尽内存后最新 Prompt 消失、主子会话混淆、最终回复难定位？
**Key insight:** 全局只增不减的正文预算会优先牺牲最新信息；Prompt 与活动应分开使用滚动预算，淘汰旧正文时同步 dirty Turn、搜索和省略提示。会话身份必须来自首条结构化 metadata；最终回复必须依据 phase 或 completion，并在镜像去重时保留后补的语义标记。
**Details / snippet:** v1.1.0：16 MiB Prompt / 48 MiB 活动预算，f 使用结构化折行锚点。121 tests、fmt、clippy、release、实际终端验收通过；真实 MAIN/SUBAGENT 检查正确，历史文件 hash 不变。模糊搜索测试应核对精确项排名首位，不能假定其他子序列候选不存在。
**Tags:** #parser #memory #session #navigation #testing

## 2026-09-06 — 主会话优先的显式选择入口
**Question:** 为什么默认先选 Session、隐藏子代理更符合日常使用？
**Key insight:** 底层支持的日志类型不必全部进入产品默认列表；用户通常想找自己的主任务，而非内部代理。将主会话过滤放在 Picker 发现入口，既统一启动/返回/刷新行为，又保留显式 ID/path 打开特殊来源的诊断能力。
**Details / snippet:** v1.1.1 移除自动打开，列表严格只含 MAIN；--all 仅扩展日期。122 tests、fmt、clippy、release 与 5 组终端验收通过；旧版已复现唯一主会话自动打开的问题，新版 Enter 确认及显式子会话不重定向均通过。
**Tags:** #ux #session #picker #regression #verification

## 2026-09-06 — 生命周期、过程告警与答案正确性分层
**Question:** 如何避免已结束的轮次被中途工具错误标成最终失败？
**Key insight:** 轮次生命周期只依据明确结束、执行错误或中断事件；活动错误是独立事实，不能覆盖生命周期，也不能作为答案正确性的代理指标。失败后重试并结束应显示 ✓ !N，用户通过最终回复判断结果；晚到记录和 rollback 也必须保持这一分离。
**Details / snippet:** v1.1.2 新增 10 项回归，全量 132 tests、fmt、clippy、release 和 6 组终端验收通过。当前核心可复用于未来 Web，但 HTTP/API、序列化适配和网页界面均尚未实现。
**Tags:** #state #parser #ux #verification #architecture

## 2026-09-06 — 先比较阅读路径，再实现 Web
**Question:** 如何在正式实现 --web 前，让用户选择清晰且可比较的设计？
**Key insight:** 设计预选应提供相同核心操作下的不同信息布局，而非同一模板换色。用合成数据制作独立 HTML，让用户实际体验会话入口、时间线、最终回复和手机排版；明确预览与正式数据接入的边界，选择确认前不推进后端。
**Details / snippet:** A 三栏工作台、B 长文阅读、C 深色双区控制台，统一 gallery 切换。六组桌面/手机浏览器交互各 18 断言通过；原有 132 tests 与 release 检查通过。正式版本保持 v1.1.2，用户尚未选定 Web 方案。
**Tags:** #design #web #prototype #interaction #verification

## 2026-09-06 — 将选定的阅读设计接到真实只读状态
**Question:** 如何将 B 阅读样稿升级为安全、实时、不中断历史阅读的正式 --web？
**Key insight:** 公开页面与受保护会话 API 必须分层：HTML 不含秘密，应允许合法导航；数据请求仍需令牌与来源校验。会话 revision 只提示变化，当前轮 turn revision 才决定正文是否重绘，失败资源也必须单独重试；不能因元数据成功而把旧正文当作已同步。
**Details / snippet:** v1.2.0：157 Rust tests、18 前端 tests、四组宽窄/监控浏览器验收、fmt/clippy/Prettier/ESLint/release 通过。活动窗口最多64条并保留前后组，其他轮更新不清空历史 DOM。目录 G 使用新 metadata 而非滞后分页；“距最新”与“新增”分别表示历史位置和新到达事件。真实主会话加载与历史文件 hash 核对通过。
**Tags:** #web #readonly #security #state #pagination #verification

## 2026-09-06 — 终端验收必须等待重排完成
**Question:** 为什么未修改 TUI 业务代码，却在重排后的 g/f 验收出现不稳定失败？
**Key insight:** 模拟终端不能在同尺寸重排时自行清空屏幕，因为真实应用可能只增量重绘。重排信号与紧随其后的按键也不应被验收脚本塞入同一事件批次；等待实际帧结束比任意延时可靠，并保留原功能断言。
**Details / snippet:** scripts/terminal_qa.py 加入分片光标帧结束与同尺寸保留断言，重排完成后才继续发键；六组终端集成连续五次及最终复验通过，不改变 Rust/TUI 逻辑。
**Tags:** #testing #terminal #resize #race #verification
