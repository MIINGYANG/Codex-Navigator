# 来源会话缺失时的恢复排查

`invalid paginated history lineage for <ID>: missing source rollout` 表示 Codex 恢复分页历史时找不到 `<ID>` 对应的来源会话。报错中的 ID 可能与正在恢复的会话不同：分支的部分历史保存在来源文件中。

在 Codex 0.153.2 的隔离合成数据中已验证：通过官方 `thread/name/set` 重命名来源和分支后，rollout 字节完全不变，分支仍可恢复；移走来源文件后出现相同报错，将原文件放回后恢复正常。官方归档目录 `archived_sessions/` 中的来源仍可被恢复流程读取。确定文件依赖的字段是 `history_base.thread_id`，仅有 `forked_from_id` 不代表必须读取原文件。

Navigator 3.2.1 之前的删除流程缺少这项依赖检查。3.2.1 会扫描活动和归档会话的首条元数据，发现依赖、重复身份、损坏或无法完整检查时拒绝删除；它不会自动修复已缺失的文件。删除前仍需结束正在使用或创建相关分支的 Codex 进程，避免检查期间产生新的依赖。

## 为什么 Codex 终端删除与 Navigator 回收站不同

Codex 0.153.2 同时提供 `codex delete`（永久删除）和 `codex archive`（归档），不能仅凭会话从列表消失就判定执行了哪一个。官方 `thread/delete` 使用线程存储管理流程；本机原生二进制中可查到 `failed to scan fork history references` 和 `forked history still references it` 的拒绝删除错误。旧 Navigator 单独移动 rollout，没有走该流程。

[官方 App Server 文档](https://learn.chatgpt.com/docs/app-server) 说明 `thread/delete` 永久删除线程、rollout 和元数据，还涉及 spawned descendants；这不等于替所有用户 fork 分支复制完整历史。Navigator 的回收站操作也不等价于此 API。3.2.1 保留可恢复回收站的约定，独立检查历史引用；不会擅自改为官方永久删除。

如果只是整理列表，可使用 Codex 官方归档保留 transcript，参见[官方命令说明](https://learn.chatgpt.com/docs/developer-commands?surface=cli)。Navigator 目前没有 Web 归档入口。

核查修复是否生效时，先运行 `codex-nav --version`，并确认网页服务已经停止旧进程、使用新命令重启。本地源码提交、GitHub 上的版本、安装的命令与已运行的网页进程可能不同；安装新文件本身不会更新旧进程。

## 在出错的设备上定位文件

进入包含此脚本的 Navigator 源码目录执行（把路径、ID 换成报错中的值）：

```bash
python3 scripts/diagnose_lineage.py \
  --codex-home /home/lmy/.codex \
  --source-id 01a0a3a1-3fe5-7d93-b44d-0f3aba2595cd
```

脚本只读取会话首条元数据与当前用户的回收站索引，不输出问题或回复正文、不修改文件、不启动 Codex。可将单个脚本复制到目标设备运行，无第三方 Python 依赖。输出路径与会话 ID 仍属于个人信息，请只向协助排查的人提供。

脚本检查 `sessions/`、`archived_sessions/` 以及 `$XDG_DATA_HOME/Trash`（默认 `~/.local/share/Trash`）。它不遍历其他磁盘挂载点的回收站，也不查询远程备份；未找到候选不等于文件永久丢失。扫描遇到上限或异常会提示检查不完整。

## 核对并恢复

1. 在目标设备退出正在使用这些会话的 Codex 进程，保留现有会话和回收站文件。
2. 找到元数据 ID 与报错中的来源 ID 一致的候选；核对回收站记录的原位置。若有多个候选、原路径已有文件或检查异常，先核对备份，不覆盖、不盲目选择最新文件。
3. 使用系统文件管理器的回收站“恢复”将匹配的原文件还原到原位置。回收站没有文件时，检查其他设备或备份中的同一来源文件；不要用空文件或相似会话代替。
4. 重新运行诊断确认来源可见，再使用原来的 Codex 恢复命令。如果接着报告另一个来源 ID 缺失，对该 ID 重复核对，直到依赖链完整。
5. 重启 Navigator 网页服务重新读取会话。安装新二进制不会自动替换已运行服务。

不要删除或置空 `history_base`、改写 `forked_from_id`、修改 ordinal/字节偏移或重建数据库来压过此报错。即使这样能打开部分会话，也不能恢复缺失的历史前缀。
