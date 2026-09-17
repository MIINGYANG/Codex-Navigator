#!/usr/bin/env python3
"""只读检查 Codex 分支来源与系统回收站；只检查首行，不输出正文，不执行恢复。

可独立复制到 Linux 设备运行，仅依赖 Python 3 标准库。
"""
import argparse
import configparser
import json
import os
from pathlib import Path
import re
import stat
import time
from urllib.parse import unquote
import uuid

MAX_ENTRIES = 100000
MAX_DEPTH = 32
MAX_HEADER = 1024 * 1024
MAX_TRASHINFO = 65536
MAX_ERRORS = 100
MAX_SCAN_BYTES = 64 * 1024 * 1024
MAX_SCAN_SECONDS = 30


def canonical_id(value):
    if not isinstance(value, str):
        raise ValueError("会话 ID 不是 UUID")
    parsed = str(uuid.UUID(value))
    if parsed != value.lower():
        raise ValueError("会话 ID 不是标准 UUID")
    return parsed


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("JSON 存在重复键")
        result[key] = value
    return result


def open_directory(path):
    """逐层使用 O_NOFOLLOW，避免父目录符号链接绕过检查。"""
    path = Path(os.path.abspath(path))
    descriptor = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except Exception:
        os.close(descriptor)
        raise


class Check:
    def __init__(self, source_id):
        self.source_id = canonical_id(source_id)
        self.entries = 0
        self.errors = 0
        self.bytes_read = 0
        self.started = time.monotonic()
        self.report = {
            "source_id": self.source_id,
            "source_files": [],
            "direct_dependents": [],
            "trash_candidates": [],
            "anomalies": [],
        }

    def problem(self, path, message):
        self.errors += 1
        if len(self.report["anomalies"]) < MAX_ERRORS:
            self.report["anomalies"].append({"path": str(path), "message": message})

    def read(self, directory, name, path, limit, first_line=False):
        if self.exhausted(path):
            return None
        try:
            descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                                 dir_fd=directory)
            with os.fdopen(descriptor, "rb") as handle:
                if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):
                    raise ValueError("不是普通文件，已跳过")
                size = min(limit + 1, MAX_SCAN_BYTES - self.bytes_read + 1)
                data = handle.readline(size) if first_line else handle.read(size)
                self.bytes_read += len(data)
                if self.bytes_read > MAX_SCAN_BYTES:
                    self.problem(path, "累计读取字节超过检查上限")
                    return None
                if len(data) > limit:
                    self.problem(path, "检查内容超过读取上限")
                    return None
                if first_line and not data.endswith(b"\n"):
                    self.problem(path, "首行未以换行符结束，可能正在写入或已截断")
                    return None
                return data.decode("utf-8")
        except (OSError, UnicodeError, ValueError) as error:
            self.problem(path, "读取失败：" + type(error).__name__)
            return None

    def header(self, directory, name, path):
        text = self.read(directory, name, path, MAX_HEADER, first_line=True)
        if text is None:
            return None
        try:
            record = json.loads(text, object_pairs_hook=unique_object)
            if not isinstance(record, dict) or record.get("type") != "session_meta":
                raise ValueError("首行不是 session_meta")
            payload = record.get("payload")
            if not isinstance(payload, dict):
                raise ValueError("缺少元数据对象")
            ident = canonical_id(payload.get("id"))
            if payload.get("history_mode") not in (None, "legacy", "paginated"):
                raise ValueError("未知 history_mode")
            base = payload.get("history_base")
            source = None
            if base is not None:
                if not isinstance(base, dict):
                    raise ValueError("history_base 不是对象")
                source = canonical_id(base.get("thread_id"))
            if self.source_id in name.lower() and ident != self.source_id:
                self.problem(path, "文件名包含目标 ID，但元数据 ID 不符；不能作为来源文件")
            return {"id": ident, "history_base_thread_id": source}
        except (ValueError, TypeError, AttributeError, RecursionError):
            # JSON 错误正文可能含会话内容；只输出固定说明。
            self.problem(path, "首行元数据损坏、ID 无效或 history_base 格式未知")
            return None

    def exhausted(self, path):
        if self.bytes_read > MAX_SCAN_BYTES:
            self.problem(path, "累计读取字节超过检查上限")
            return True
        if time.monotonic() - self.started > MAX_SCAN_SECONDS:
            self.problem(path, "检查耗时超过上限")
            return True
        return False

    def walk(self, root, visit, required=False):
        root = Path(os.path.abspath(root))
        try:
            descriptor = open_directory(root)
        except FileNotFoundError:
            if required:
                self.problem(root, "会话目录不存在，检查不完整")
            return
        except OSError as error:
            self.problem(root, "目录无法安全读取：" + type(error).__name__)
            return
        try:
            self.walk_fd(descriptor, root, visit, 0)
        finally:
            os.close(descriptor)

    def walk_fd(self, descriptor, root, visit, depth):
        if depth > MAX_DEPTH:
            self.problem(root, "目录深度超过检查上限")
            return
        try:
            with os.scandir(descriptor) as entries:
                for entry in entries:
                    if self.exhausted(root):
                        return
                    self.entries += 1
                    if self.entries > MAX_ENTRIES:
                        self.problem(root, "目录项数量超过检查上限")
                        return
                    path = root / entry.name
                    try:
                        kind = entry.stat(follow_symlinks=False).st_mode
                        if stat.S_ISLNK(kind):
                            self.problem(path, "符号链接已跳过")
                        elif stat.S_ISDIR(kind):
                            child = os.open(entry.name, os.O_RDONLY | os.O_DIRECTORY |
                                            os.O_NOFOLLOW, dir_fd=descriptor)
                            try:
                                self.walk_fd(child, path, visit, depth + 1)
                            finally:
                                os.close(child)
                        elif stat.S_ISREG(kind):
                            visit(descriptor, entry.name, path)
                        else:
                            self.problem(path, "非普通文件已跳过")
                    except OSError as error:
                        self.problem(path, "目录项读取失败：" + type(error).__name__)
        except OSError as error:
            self.problem(root, "目录遍历失败：" + type(error).__name__)

    def session(self, directory, name, path):
        if name.endswith(".jsonl.zst"):
            self.problem(path, "压缩记录暂不支持检查")
        if not name.endswith(".jsonl"):
            return
        header = self.header(directory, name, path)
        if header is None:
            return
        if header["id"] == self.source_id:
            self.report["source_files"].append({"path": str(path), **header})
        if header["history_base_thread_id"] == self.source_id:
            self.report["direct_dependents"].append({"path": str(path), **header})

    def trash_info(self, directory, name, path, trash_files):
        if not name.endswith(".trashinfo"):
            return
        text = self.read(directory, name, path, MAX_TRASHINFO)
        if text is None:
            return
        try:
            config = configparser.ConfigParser(interpolation=None)
            config.read_string(text)
            section = config["Trash Info"]
            encoded = section["Path"]
            if re.search(r"%(?![0-9a-fA-F]{2})", encoded):
                raise ValueError("非法 URL 编码")
            original = unquote(encoded, encoding="utf-8", errors="strict")
            if not Path(original).is_absolute() or "\x00" in original:
                raise ValueError("回收站原路径不是绝对路径")
            deleted = section.get("DeletionDate")
            if not deleted:
                raise ValueError("缺少删除时间")
        except (configparser.Error, KeyError, ValueError, UnicodeError):
            self.problem(path, "回收站元数据损坏或路径格式未知")
            return
        filename = name[:-len(".trashinfo")]
        stored = trash_files / filename
        try:
            descriptor = open_directory(trash_files)
        except OSError as error:
            self.problem(stored, "回收站文件目录无法读取：" + type(error).__name__)
            return
        try:
            # 普通用户回收站还包含其他类型文件，不读取无关项目。
            if not (filename.endswith(".jsonl") or original.endswith(".jsonl") or
                    self.source_id in filename.lower()):
                return
            header = self.header(descriptor, filename, stored)
            if header is not None and header["id"] == self.source_id:
                self.report["trash_candidates"].append({
                    "path": str(stored), "trashinfo": str(path),
                    "original_path": original, "deletion_date": deleted, **header,
                    "verification": "仅核对首行 ID；未验证完整历史，不代表可以安全恢复",
                })
        finally:
            os.close(descriptor)


def diagnose(codex_home, source_id, data_home=None):
    check = Check(source_id)
    home = Path(os.path.abspath(codex_home))
    check.report["codex_home"] = str(home)
    if not home.exists():
        check.problem(home, "CODEX_HOME 不存在，请确认设备与路径")
    check.walk(home / "sessions", check.session, required=True)
    check.walk(home / "archived_sessions", check.session)
    data = Path(data_home or os.environ.get("XDG_DATA_HOME") or
                str(Path.home() / ".local/share"))
    trash = Path(os.path.abspath(data)) / "Trash"
    check.report["trash_root"] = str(trash)
    check.walk(trash / "info", lambda directory, name, path:
               check.trash_info(directory, name, path, trash / "files"))
    if len(check.report["source_files"]) > 1:
        check.problem(home, "多个来源文件使用同一 ID，存在歧义，不能确定 Codex 实际读取哪一个")
    check.report["complete"] = check.errors == 0
    check.report["check_status"] = "指定范围检查完成" if not check.errors else "检查不完整"
    check.report["anomaly_count"] = check.errors
    check.report["entries_checked"] = check.entries
    check.report["bytes_checked"] = check.bytes_read
    check.report["source_status"] = (
        "找到来源文件" if check.report["source_files"] else
        "已检查目录中未找到来源；检查不完整" if check.errors else
        "已检查目录中未找到来源文件")
    check.report["scope"] = (
        "只检查 sessions、archived_sessions 与当前用户 XDG 系统回收站；"
        "不含其他磁盘的回收站、备份或其他设备。会话只检查首行，不输出正文；"
        "不检查完整历史或完整依赖链，不执行恢复。")
    return check.report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-home", default=os.environ.get("CODEX_HOME") or
                        str(Path.home() / ".codex"))
    parser.add_argument("--source-id", required=True, type=canonical_id)
    args = parser.parse_args()
    report = diagnose(args.codex_home, args.source_id)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["complete"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
