# Web v1.2.0 实施约定

用户已选择 B「专注阅读」。浅紫底 #f8f7fc、纸面 #ffffff、正文 #302a43、强调 #7156ab、边线 #e5e0ee、警告 #916a32；标题本地衬线，正文系统中文无衬线，路径等宽。不加载网络字体。签名元素是最终回复书签。布局为主会话入口 → 左轻目录 / 右宽正文；窄屏压缩目录，保留全部导航。不引入仪表盘。

## 安全与运行

`codex-nav --web [--port 8765] [--no-open]`；保留 --session、--cwd、--all、--no-watch。仅监听 127.0.0.1，端口可为 0。页面资源内嵌。随机令牌通过启动 URL fragment 传入，API 使用 `X-Codex-Nav-Token`。不开放 CORS，校验 Host / Origin，禁止任意路径读取，Codex 数据完全只读。无云、遥测、AI API。

## 前后端约定

所有 API 为 GET，JSON 错误 `{error: string}`；除静态资源外必须认证。下列字段固定，允许添加。key 为服务端登记的不透明会话标识，不能是浏览器指定文件路径。index 从 0 开始。

- `/api/info` → `{version, watch, default_all, initial_session: key|null, refresh_ms: 750}`。
- `/api/sessions?all=0|1&refresh=1` → `{loading, error: string|null, sessions: [{key,id,title,cwd,updated_at,turn_count,first_prompt}]}`。仅主会话；前端标题/目录过滤。扫描异步，loading 时重试。
- `/api/session/{key}` → `{key, meta:{id,cwd}, generation, revision, loading, offset, total_bytes, turn_count, latest_active: index|null, stats, error: string|null, watch}`。首次启动后台解析；重新加载 generation 改变。前端轮询元数据，变化才获取目录/正文。
- `/api/session/{key}/turns?q=...&offset=0&limit=100` → `{generation,revision,total,offset,turns:[{index,ordinal,preview,status,errors,revision,started_at,has_final,omitted_bytes}]}`。复用 Prompt 搜索索引；最大分页 100，空查询时间顺序。
- `/api/session/{key}/turn/{index}?offset=0&limit=8` → `{generation,revision,turn:{index,ordinal,id,prompt:{text,preview,images_count,omitted_bytes},status,activity,started_at,completed_at},items:[{index,type,text?,phase?,name?,summary?,is_error?,path?,kind?}],items_total,next_offset:number|null,final_answer:{index,text,phase}|null}`。type 为 agent_message/tool_call/tool_output/file_activity/notice/omitted。最终回复独立提供，仅可靠 final 标记；limit 最大 8。
- `/api/session/{key}/turn/{index}/text` → 纯文本，复制当前轮完整已保留内容（包括省略标记）。
- `/api/session/{key}?refresh=1` → 手动刷新，无 watch 时也可用。

status 值 in_progress/completed/failed/interrupted/unknown/rolled_back，仅描述生命周期；errors 为活动警告数，不代表最终结果正确或错误。

客户端保留历史选择、展开状态和阅读位置；先加载当前轮，活动分页展开。支持 / 搜索、Esc、j/k、g/G 按焦点、f 最终回复、s 会话列表、r 刷新、c/C 复制、[/] 前后轮、? 帮助。不劫持输入框或浏览器组合键。使用 textContent / 安全 Markdown 子集，禁止原始 HTML 注入和外部图片自动加载。显示断线、解析省略、空态和无最终回复提示。

实施补充：响应包含 turn.revision，用于避免其他轮更新重绘历史正文；客户端可见活动窗口最多 64 条，前后组均可访问。目录 G 在无搜索时以最新 metadata.latest_active 为准，不能依赖可能尚在加载的旧目录分页。失败资源独立 dirty 重试，不因元数据读取成功就认为正文已同步。

服务端限定打开会话缓存与响应分页，沿用 SessionWorker 增量读取与内存预算；测试只用合成 fixture，真实只读验证不复制用户内容。
