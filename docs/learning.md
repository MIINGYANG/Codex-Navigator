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

## 2026-09-06 — 一句话入口与可验证的 Agent 安装交付
**Question:** 如何让用户把一句话交给自己的 Agent，就能安装、自检并打开 Navigator？
**Key insight:** README 应提供短入口，把环境检查、安装与验收细节放进可引用的 Agent 指南。交付必须区分构建成功、HTTP 可用、浏览器交互验收和服务仍在运行；没有会话是正常空态，后台进程不能保留时需明确交回用户终端启动。
**Details / snippet:** README 精简为产品介绍、一句话指令、手动安装和必要限制；新增 docs/agent-setup.md。相对链接、CLI 参数、157 项 Rust 测试、18 项前端测试与 release build 复验通过；仅文档变更，不修改版本或发布远程资源。
**Tags:** #readme #installation #agent #verification #ux

## 2026-09-06 — 终端与网页是同一产品的两种入口
**Question:** 如何避免简化安装文档后，用户误以为 Navigator 只能在网页使用？
**Key insight:** 仅在命令清单中提到终端版不够，首屏和 Agent 安装指令都应保留模式选择。终端版需要交互式终端，交付启动/退出命令；网页版才需要服务地址、令牌和后台生命周期说明。
**Details / snippet:** README 首屏并列 codex-nav 与 codex-nav --web --port 0；Agent 指南按模式分支验收。链接、围栏、CLI 与源码核对及 7 项相关测试通过，仅修改文档。
**Tags:** #readme #tui #web #onboarding #verification

## 2026-09-06 — 独立命令安装与 PATH 验收
**Question:** 用户是否可以安装后在任意目录运行 Navigator，而不进入源码目录？
**Key insight:** cargo install 安装的是独立二进制，Web 资源已内嵌；日常使用不依赖源码工作目录。命令能否直接输入取决于 PATH，应区分当前进程的临时配置与用户新终端的持久配置，后者需要针对实际 shell 授权设置和验证。
**Details / snippet:** 已在 /tmp 使用临时安装命令通过 --version / --help；README 增加任意目录启动与 PATH 说明，Agent 指南增加新终端验收要求。未修改用户 shell，未进行全局安装。
**Tags:** #installation #path #shell #readme #verification

## 2026-09-06 — 安装排错与面向用户的文档入口
**Question:** 如何在 README 防止旧 Cargo 安装失败，并减少开发过程文档对用户的干扰？
**Key insight:** 工具链报错应先确认命令路径和版本，不能将 PATH 缺失误判为必须重新安装；旧 Cargo 的 v4 锁文件错误也不应通过改锁文件规避。精简 README 导航可降低用户阅读负担，但取消链接不等于从公开仓库隐藏文件，保留开发历史与整理用户入口是不同操作。
**Details / snippet:** 新增 FAQ 和 Agent 工具链选择检查，本机 source ~/.cargo/env 后 Rust/Cargo 1.98.1 验证通过；文档链接与差异检查通过。保留设计和验收文件，不改 ignore、不删除、不重写历史、不推送。
**Tags:** #readme #rust #installation #documentation #verification

## 2026-09-06 — 精确路径历史清理与远程验收
**Question:** 如何在保留本地副本和开发时间线的同时，从公开历史移除两份指定文档？
**Key insight:** 清理需要覆盖所有相关提交和标签，仅从当前索引取消跟踪不够。先在仓库外保存完整 bundle 和旧远程哈希，逐提交保留非目标文件与元数据，再以精确 lease 原子推送，并用新克隆验证目标路径及其全部历史 blob 都不再可达。
**Details / snippet:** 12 个提交、6 个注释标签完成映射与验证；7 个文档历史 blob 不再可达，新 SSH clone 和 fsck 通过。本地 Markdown 保留且精确忽略，未永久删除文件。Git 2.25.1 应使用 --stdin 隐式批量事务，而非较新的 start/commit 指令；旧克隆与托管缓存需要另行处理，不能承诺自动消失。
**Tags:** #git #history #backup #privacy #verification

## 2026-09-06 — 从干净初始 PATH 验证持久安装
**Question:** 如何让安装后的 Navigator 在新终端中无需手动 source 就能启动？
**Key insight:** 临时 source 只能证明当前进程可用；持久安装必须在用户实际 shell 的启动配置中加载正确路径。用户明确授权后，备份并幂等追加，再从不含安装目录的初始 PATH 启动新 shell，才能验证不是继承了 Agent 的临时环境。
**Details / snippet:** 本机 Bash 配置仅新增一个存在检查加载块，原配置逐字保留；普通/登录/嵌套新 Bash 验证成功，源码外直接启动 TUI 并 q 退出通过。README 与 Agent 指令包含用户级配置授权、备份、幂等性和新终端验收；不上传私有 shell 备份。
**Tags:** #installation #path #bash #agent #verification

## 2026-09-06 — 用问题与真实界面制作可验收的宣传样片
**Question:** 如何交付三种有吸引力、可比较且不泄露会话的短视频宣传方向？
**Key insight:** 钩子分别围绕找不到答案、快捷键定位和隔天遗忘展开，必须让后续演示兑现同一个问题。真实应用加载合成会话后采集截图，可同时验证产品事实与保护隐私；截图剪辑、人声缺失和未投放测试均需明确说明。
**Details / snippet:** 独立素材包包含三版 28/24/30 秒竖屏 MP4、双比例封面、字幕、原创声音、旁白及平台文案和制作源码。通过全片解码、字幕/链接/hash、动画像素回归、浏览器播放与桌面/窄屏布局检查。局部动画完成后再做整帧位移可避免清理坐标错位残影；Chrome 后台静音播放可能被暂停，播放验收通过真实手势启动并检查时间推进。素材留仓库外，不改应用、不上传。
**Tags:** #promotion #video #privacy #design #verification

## 2026-09-06 — 在痛点剧情中交代产品灵感
**Question:** 用户选中 C 宣传片后，如何自然加入来自 GPT 侧边导航栏历史跳转的灵感？
**Key insight:** 保留已获认可的开场，把灵感放在痛点与产品演示之间，才能同时解释为什么做和怎么解决。以用户提供的创作经历表述，使用自行绘制的导航概念图，不暗示官方关联、真实界面截图或账号历史互通。
**Details / snippet:** C 更新为 32 秒，第 8–14 秒加入 ChatGPT 侧边栏灵感段落；原 30 秒成片与源码另存，A/B 不动。字幕/旁白/发布文案同步，完整解码、时长、声音、时间轴、动效回归、原稿及链接检查通过；实际新增场景与成片解码画面已检查。
**Tags:** #promotion #storytelling #versioning #verification

## 2026-09-08 — 问题脉络必须区分记录顺序与推断关系
**Question:** 如何让用户交互查看具体问题、过程及递进，同时保持只读、快速与可信？
**Key insight:** 以真实问题序号和已有活动记录建立两层路径，缺口和省略要显式标注，不能把相邻记录或工具日志包装成内部思维或因果。搜索排名不是时间顺序，脉络必须在服务端分页前按记录顺序排序，保留阅读页原来的相关性排序。
**Details / snippet:** v1.3.0 采用问题轨道与有界步骤/原文面板，新增 order=chronological 与12项纯投影测试；空轮询不重绘，无关问题更新保留完整工作区 DOM。跨页按焦点而非旧selected导航，局部 g/G 滚动局部面板，v 切回阅读页恢复可见焦点。158 Rust +30前端测试、格式/lint、release、宽窄浏览器和终端验收通过；未写真实会话、不调用 AI。
**Tags:** #web #navigation #visualization #read-only #verification
