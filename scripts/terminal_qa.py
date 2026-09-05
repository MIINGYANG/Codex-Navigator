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


def picker_session(test_root, session_id, source, prompt):
    sessions = test_root / "codex" / "sessions"
    sessions.mkdir(parents=True, exist_ok=True)
    path = sessions / ("rollout-" + session_id + ".jsonl")
    path.write_bytes(
        (json.dumps({"type": "session_meta", "payload": {
            "id": session_id, "source": source, "cwd": str(Path.cwd())}}) + "\n").encode()
        + record({"type": "user_message", "message": prompt})
        + record({"type": "agent_message", "message": "Synthetic session reply"}))
    return path


def verify_main_picker(binary, root):
    picker_root = root / "picker"
    main_prompt = "PRIMARY SESSION CHOICE"
    main_path = picker_session(picker_root, "qa-main", "cli", main_prompt)
    app = Navigator(binary, [], picker_root)
    app.expect("SESSIONS")
    app.expect(main_prompt)
    assert "TIMELINE" not in app.screen.text(), "unique main session was opened automatically"
    app.send("\r")
    app.expect("TIMELINE")
    app.expect(main_prompt)
    app.send("s")
    app.expect("SESSIONS")
    app.expect(main_prompt)
    app.close()

    child_prompt = "ZZZZZZZZZZ CHILD HIDDEN"
    unknown_prompt = "XXXXXXXXXX UNKNOWN HIDDEN"
    child_source = {"subagent": {"thread_spawn": {
        "parent_thread_id": "qa-main", "agent_nickname": "Hidden child"}}}
    child_path = picker_session(picker_root, "qa-child", child_source, child_prompt)
    unknown_path = picker_session(picker_root, "qa-unknown", "future-source", unknown_prompt)
    paths = (main_path, child_path, unknown_path)
    digests = {path: hashlib.sha256(path.read_bytes()).digest() for path in paths}
    for arguments in ([], ["--all"]):
        app = Navigator(binary, arguments, picker_root)
        app.expect("SESSIONS")
        app.expect(main_prompt)
        for query in (child_prompt, unknown_prompt):
            assert query not in app.screen.text(), "non-main session was listed"
            app.send("/" + query)
            app.expect("No matching sessions")
            app.send("\x1b")
            app.expect(main_prompt)
        app.send("r")
        app.expect(main_prompt)
        assert "[SUBAGENT]" not in app.screen.text(), "refresh exposed a child session"
        assert "[UNKNOWN]" not in app.screen.text(), "refresh exposed an unknown session"
        app.close()

    for selector in (str(child_path), "qa-child"):
        app = Navigator(binary, ["--session", selector], picker_root)
        app.expect("TIMELINE")
        app.expect("SUBAGENT")
        app.expect(child_prompt)
        assert main_prompt not in app.screen.text(), "explicit child session redirected to parent"
        app.send("s")
        app.expect("SESSIONS")
        app.expect(main_prompt)
        assert child_prompt not in app.screen.text(), "returning to picker exposed a child session"
        app.close()

    other_root = root / "non-main-picker"
    picker_session(other_root, "qa-child-only", child_source, child_prompt)
    picker_session(other_root, "qa-unknown-only", "future-source", unknown_prompt)
    for arguments in ([], ["--all"]):
        app = Navigator(binary, arguments, other_root)
        app.expect("No main sessions found")
        assert child_prompt not in app.screen.text(), "non-main-only picker exposed a child session"
        assert unknown_prompt not in app.screen.text(), "non-main-only picker exposed an unknown session"
        app.close()
    for path, digest in digests.items():
        assert hashlib.sha256(path.read_bytes()).digest() == digest, "picker modified a session"
    print("PASS terminal: startup-picker/main-only/search/refresh/all/explicit-child-path-and-id/read-only")


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
        running = hasattr(self, "proc")
        if running and (width, height) == (self.screen.width, self.screen.height):
            return
        if running:
            # Drain the previous frame before changing the decoder's geometry.
            self.pump()
            previous_frame = self.screen.frames
        if hasattr(self, "screen"):
            self.screen.resize(width, height)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        if running:
            self.proc.send_signal(signal.SIGWINCH)
            # A resize and a key in the same mio poll batch can lose the TTY
            # readiness edge in crossterm 0.28. Wait for the redraw acknowledgement
            # before sending the next user action, rather than adding a blind sleep.
            deadline = time.monotonic() + 4
            while self.screen.frames == previous_frame and time.monotonic() < deadline:
                self.pump()
            assert self.screen.frames > previous_frame, "terminal resize was not rendered"

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
        self.frames = 0
        self.resize(width, height)

    def resize(self, width, height):
        if (width, height) == (getattr(self, "width", None), getattr(self, "height", None)):
            return
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
                    elif code in ("h", "l") and args == "?25":
                        # ratatui finalizes frames by restoring cursor visibility;
                        # search input uses a visible cursor, reading uses a hidden one.
                        self.frames += 1
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
    # The decoder must preserve same-size contents and acknowledge fragmented frames.
    screen = Screen(20, 4)
    screen.feed(b"USER\x1b[?2")
    assert screen.frames == 0
    screen.feed(b"5l")
    assert screen.frames == 1
    screen.feed(b"\x1b[?25h")
    assert screen.frames == 2
    screen.resize(20, 4)
    assert "USER" in screen.text()
    with tempfile.TemporaryDirectory(prefix="codex-nav-terminal-qa-") as directory:
        root = Path(directory)
        session = root / "rollout-synthetic.jsonl"
        session.write_bytes(record({"type": "user_message", "message": "第一轮 多行问题\n检查项目"})
                            + record({"type": "agent_message", "message": "可见回复"})
                            + record({"type": "task_complete"})
                            + record({"type": "user_message", "message": "第二轮 检查 authentication"})
                            + record({"type": "agent_message", "message": "\n".join(
                                ["Synthetic long viewer line " + str(n) for n in range(100)]
                                + ["TURN TWO END"])}))
        app = Navigator(binary, ["--session", str(session)], root)
        app.expect("WATCHING")
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
        for width in (70, 122, 70, 120):
            app.resize(width, 32)
            app.send("g")
            app.expect("USER")
            app.expect("TURN 02")
            app.send("G")
            app.expect("TURN TWO END")
            app.expect("TURN 02")
            app.expect("+1 new turn")
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
        app.send("\t")
        app.send("G")
        app.output.clear()
        app.resize(121, 32)
        app.expect("TURN 03")
        app.send("f")
        app.expect("No retained final answer")
        app.pump(3.2)
        assert "No retained final answer" not in app.screen.text(), "toast did not expire"
        digest = hashlib.sha256(session.read_bytes()).digest()
        app.close()
        assert hashlib.sha256(session.read_bytes()).digest() == digest
        print("PASS terminal: navigation/search/history/live/resize/viewer-gG/help/clipboard/q/restore/read-only")

        app = Navigator(binary, ["--session", str(session), "--no-watch"], root)
        app.expect("STATIC")
        app.close("\x03")
        print("PASS terminal: Ctrl+C restores termios and alternate screen")

        app = Navigator(binary, [], root)
        app.expect("No main sessions found")
        app.close()
        print("PASS terminal: empty-session picker")

        verify_main_picker(binary, root)

        final_session = root / "rollout-final.jsonl"
        final_session.write_bytes(
            (json.dumps({"type": "session_meta", "payload": {
                "id": "synthetic-main", "source": "cli", "cwd": str(root)}}) + "\n").encode()
            + record({"type": "user_message", "message": "Find the final answer"})
            + record({"type": "agent_message", "message": "\n".join(
                "Working line " + str(n) for n in range(100))})
            + record({"type": "agent_message", "message": "FINAL RESULT VERIFIED"})
            + record({"type": "task_complete", "last_agent_message": "FINAL RESULT VERIFIED"}))
        final_digest = hashlib.sha256(final_session.read_bytes()).digest()
        final_app = Navigator(binary, ["--session", str(final_session)], root)
        final_app.expect("WATCHING")
        final_app.expect("MAIN")
        for width in (120, 70, 122):
            final_app.resize(width, 32)
            final_app.send("f")
            final_app.expect("FINAL ANSWER")
            final_app.expect("FINAL RESULT VERIFIED")
            final_app.expect("TURN 01")
            final_app.send("g")
            final_app.expect("USER")
        final_app.close()
        assert hashlib.sha256(final_session.read_bytes()).digest() == final_digest
        print("PASS terminal: final-answer/phase-promotion/wide-narrow/main-identity/watching/read-only")

        status_session = root / "rollout-status.jsonl"
        failed_output = (json.dumps({"type": "response_item", "payload": {
            "type": "function_call_output", "output": {"exit_code": 1, "output": "retry needed"}}}) + "\n").encode()
        successful_output = (json.dumps({"type": "response_item", "payload": {
            "type": "function_call_output", "output": {"exit_code": 0, "output": "retry finished"}}}) + "\n").encode()
        status_session.write_bytes(
            record({"type": "user_message", "message": "Retry then complete"})
            + failed_output + successful_output
            + record({"type": "task_complete", "last_agent_message": "USER REVIEWS THE OUTCOME"})
            + record({"type": "user_message", "message": "No completion yet"}) + failed_output
            + record({"type": "user_message", "message": "Interrupted turn"})
            + record({"type": "turn_aborted"})
            + record({"type": "user_message", "message": "Execution error turn"})
            + record({"type": "turn_error"}))
        status_digest = hashlib.sha256(status_session.read_bytes()).digest()
        status_app = Navigator(binary, ["--session", str(status_session)], root)
        status_app.expect("TIMELINE")
        status_app.expect("01 ✓ !1")
        status_app.expect("02 … !1")
        status_app.expect("03 ⊘")
        status_app.expect("04 ✕")
        status_app.send("g")
        status_app.send("f")
        status_app.expect("USER REVIEWS THE OUTCOME")
        for width in (70, 120):
            status_app.resize(width, 32)
            status_app.send("G")
            status_app.expect("TURN STATUS · completed")
            status_app.expect("1 activity errors")
            status_app.expect("Completion is not a correctness verdict.")
        for outcome in ("incomplete", "interrupted", "execution error"):
            status_app.send("]")
            status_app.send("G")
            status_app.expect("TURN STATUS · " + outcome)
        status_app.send("?")
        status_app.expect("!N activity errors")
        status_app.send("\x1b")
        status_app.close()
        assert hashlib.sha256(status_session.read_bytes()).digest() == status_digest
        print("PASS terminal: lifecycle/activity-warning/retry/completion/abort/error/final-review/read-only")

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
