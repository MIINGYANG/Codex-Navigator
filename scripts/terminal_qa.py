#!/usr/bin/env python3
"""Linux/macOS terminal integration QA; launches Navigator only, never Codex.

All writes target a generated test directory. Real sessions (optional) are read-only.
No terminal transcript containing private session content is printed or saved.
"""
import argparse
import codecs
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import unicodedata


def record(payload):
    return (json.dumps({"type": "event_msg", "payload": payload}, ensure_ascii=False) + "\n").encode()


class Navigator:
    def __init__(self, binary, args, test_root):
        self.master, self.slave = pty.openpty()
        self.screen = Screen(120, 32)
        self.resize(120, 32)
        self.before = termios.tcgetattr(self.slave)
        env = dict(os.environ, TERM="xterm-256color", CODEX_HOME=str(test_root / "codex"),
                   XDG_CONFIG_HOME=str(test_root / "config"), DISPLAY="", WAYLAND_DISPLAY="")
        self.proc = subprocess.Popen([str(binary), *args], stdin=self.slave,
                                     stdout=self.slave, stderr=self.slave, env=env)
        self.output = bytearray()

    def resize(self, width, height):
        if hasattr(self, "screen"):
            self.screen.resize(width, height)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        if hasattr(self, "proc"):
            self.proc.send_signal(signal.SIGWINCH)

    def pump(self, duration=0.1):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            readable, _, _ = select.select([self.master], [], [], min(0.05, max(0, deadline-time.monotonic())))
            if readable:
                try:
                    data = os.read(self.master, 65536)
                    self.output.extend(data)
                    self.screen.feed(data)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break

    def expect(self, text, timeout=4):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if text in self.screen.text():
                return
            self.pump()
        raise AssertionError("terminal expectation was not rendered: " + text)

    def send(self, text):
        os.write(self.master, text.encode())
        self.pump()

    def close(self, key="q"):
        self.send(key)
        deadline = time.monotonic() + 4
        while self.proc.poll() is None and time.monotonic() < deadline:
            self.pump()
        if self.proc.poll() is None:
            self.proc.terminate()
            self.proc.wait(timeout=2)
            raise AssertionError("Navigator did not exit using its keyboard handler")
        self.pump()
        assert self.proc.returncode == 0, "Navigator returned a failure exit status"
        assert termios.tcgetattr(self.slave) == self.before, "terminal termios was not restored"
        assert b"\x1b[?1049l" in self.output, "alternate screen was not restored"
        os.close(self.master)
        os.close(self.slave)


class Screen:
    """Small CSI screen decoder for ratatui output; avoids matching stale raw bytes."""
    def __init__(self, width, height):
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.resize(width, height)

    def resize(self, width, height):
        self.width, self.height = width, height
        self.rows = [[" "] * width for _ in range(height)]
        self.x = self.y = 0

    def text(self):
        return "\n".join("".join(row) for row in self.rows)

    def feed(self, data):
        source = self.pending + self.decoder.decode(data)
        self.pending = ""
        i = 0
        while i < len(source):
            ch = source[i]
            if ch == "\x1b":
                if i + 1 >= len(source):
                    self.pending = source[i:]
                    return
                if source[i+1] == "[":
                    match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", source[i:])
                    if not match:
                        self.pending = source[i:]
                        return
                    args, _, code = match.groups()
                    values = [int(v or 0) for v in args.lstrip("?<>").split(";")]
                    n = values[0] or 1
                    if code in "Hf":
                        self.y = max(0, n-1)
                        self.x = max(0, (values[1] if len(values)>1 else 1)-1)
                    elif code == "G": self.x = n-1
                    elif code == "A": self.y = max(0, self.y-n)
                    elif code == "B": self.y += n
                    elif code == "C": self.x += n
                    elif code == "D": self.x = max(0, self.x-n)
                    elif code == "J" and values[0] in (2, 3):
                        self.rows = [[" "] * self.width for _ in range(self.height)]
                    elif code == "K" and self.y < self.height:
                        for x in range(self.x, self.width): self.rows[self.y][x] = " "
                    i += len(match.group())
                    continue
                i += 2
                continue
            if ch == "\r": self.x = 0
            elif ch == "\n": self.y += 1
            elif not unicodedata.category(ch).startswith("C"):
                width = 0 if unicodedata.combining(ch) else (2 if unicodedata.east_asian_width(ch) in "WF" else 1)
                if self.y < self.height and self.x < self.width:
                    self.rows[self.y][self.x] = ch
                    if width == 2 and self.x+1 < self.width: self.rows[self.y][self.x+1] = ""
                self.x += width
            i += 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--real-session", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="codex-nav-terminal-qa-") as directory:
        root = Path(directory)
        session = root / "rollout-synthetic.jsonl"
        session.write_bytes(record({"type": "user_message", "message": "第一轮 多行问题\n检查项目"})
                            + record({"type": "agent_message", "message": "可见回复"})
                            + record({"type": "task_complete"})
                            + record({"type": "user_message", "message": "第二轮 检查 authentication"}))
        app = Navigator(binary, ["--session", str(session)], root)
        app.expect("第二轮")
        app.send("g")
        app.output.clear()
        with session.open("ab") as file:
            file.write(record({"type": "user_message", "message": "第三轮 新输入"}))
        app.expect("+1 new turn")
        app.resize(121, 32)
        app.expect("TURN 01")
        app.send("/authentication")
        app.expect("Search prompts")
        app.send("\r")
        app.output.clear()
        app.resize(122, 32)
        app.expect("TURN 02")
        app.send("?")
        app.expect("KEYBOARD HELP")
        app.send("\x1b")
        app.send("c")
        app.expect("Clipboard unavailable")
        app.resize(70, 22)
        app.send("\t")
        app.expect("TIMELINE")
        app.send("\t")
        app.resize(1, 1)
        app.pump()
        app.resize(120, 32)
        app.send("G")
        app.output.clear()
        app.resize(121, 32)
        app.expect("TURN 03")
        digest = hashlib.sha256(session.read_bytes()).digest()
        app.close()
        assert hashlib.sha256(session.read_bytes()).digest() == digest
        print("PASS terminal: navigation/search/history/live/resize/help/clipboard/q/restore/read-only")

        app = Navigator(binary, ["--session", str(session), "--no-watch"], root)
        app.expect("STATIC")
        app.close("\x03")
        print("PASS terminal: Ctrl+C restores termios and alternate screen")

        app = Navigator(binary, [], root)
        app.expect("No Codex sessions found")
        app.close()
        print("PASS terminal: empty-session picker")

        if args.real_session:
            path = args.real_session.resolve()
            app = Navigator(binary, ["--session", str(path)], root)
            app.expect("TIMELINE", timeout=15)
            app.pump(1)
            app.send("G")
            app.resize(75, 25)
            app.send("\t")
            app.resize(120, 32)
            app.close("\x03")
            print("PASS terminal: real local session open/resize/navigate/Ctrl+C (content not logged)")


if __name__ == "__main__":
    main()
