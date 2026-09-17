#!/usr/bin/env python3
"""诊断脚本的隔离合成测试；临时样本保留在 /tmp，不操作真实会话。"""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import quote

import diagnose_lineage as diagnostic

SOURCE = "11111111-1111-4111-8111-111111111111"
CHILD = "22222222-2222-4222-8222-222222222222"


class DiagnoseTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="codex-lineage-test-"))
        self.home = self.root / "codex"
        self.home.mkdir()
        (self.home / "sessions").mkdir()
        self.data = self.root / "data"
        self.secret = "SYNTHETIC_BODY_MUST_NOT_APPEAR"

    def session(self, path, ident=SOURCE, base=None):
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {"id": ident, "history_mode": "paginated"}
        if base is not None:
            payload["history_base"] = {"thread_id": base}
        path.write_text(json.dumps({"type": "session_meta", "payload": payload}) +
                        "\n" + self.secret + "\n")
        return path

    def trash(self, ident=SOURCE, name=None):
        name = name or "rollout-" + SOURCE + ".jsonl"
        original = str(self.home / "sessions/中文 空格" / name)
        info = self.data / "Trash/info" / (name + ".trashinfo")
        info.parent.mkdir(parents=True, exist_ok=True)
        info.write_text("[Trash Info]\nPath=" + quote(original) +
                        "\nDeletionDate=2026-09-17T20:00:00\n")
        stored = self.session(self.data / "Trash/files" / name, ident)
        return original, info, stored

    def check(self):
        return diagnostic.diagnose(self.home, SOURCE, self.data)

    def test_archived_source_and_direct_branch(self):
        source = self.session(self.home / "archived_sessions/source.jsonl")
        child = self.session(self.home / "sessions/2026/09/17/child.jsonl", CHILD, SOURCE)
        report = self.check()
        self.assertTrue(report["complete"])
        self.assertEqual(report["source_files"][0]["path"], str(source))
        self.assertEqual(report["direct_dependents"][0]["path"], str(child))
        self.assertNotIn(self.secret, json.dumps(report))

    def test_url_encoded_trash_and_read_only_hashes(self):
        original, info, stored = self.trash()
        files = [p for p in self.root.rglob("*") if p.is_file()]
        before = {p: hashlib.sha256(p.read_bytes()).digest() for p in files}
        report = self.check()
        self.assertTrue(report["complete"])
        candidate = report["trash_candidates"][0]
        self.assertEqual(candidate["original_path"], original)
        self.assertEqual(candidate["path"], str(stored))
        self.assertEqual(candidate["deletion_date"], "2026-09-17T20:00:00")
        self.assertIn("不代表可以安全恢复", candidate["verification"])
        self.assertNotIn(self.secret, json.dumps(report))
        self.assertEqual(before, {p: hashlib.sha256(p.read_bytes()).digest() for p in files})
        self.assertEqual(set(files), {p for p in self.root.rglob("*") if p.is_file()})

    def test_same_filename_wrong_metadata_id(self):
        self.trash(ident=CHILD)
        report = self.check()
        self.assertFalse(report["complete"])
        self.assertEqual(report["trash_candidates"], [])
        self.assertIn("ID 不符", report["anomalies"][0]["message"])

    def test_broken_header_never_exposes_body(self):
        path = self.home / "sessions/broken.jsonl"
        path.parent.mkdir(exist_ok=True)
        path.write_text(self.secret + "\n")
        report = self.check()
        self.assertFalse(report["complete"])
        self.assertNotIn(self.secret, json.dumps(report))

    def test_symbolic_links_are_not_followed(self):
        external = self.session(self.root / "external/source.jsonl")
        sessions = self.home / "sessions"
        sessions.mkdir(exist_ok=True)
        (sessions / "link.jsonl").symlink_to(external)
        (sessions / "directory").symlink_to(external.parent, target_is_directory=True)
        report = self.check()
        self.assertFalse(report["complete"])
        self.assertEqual(report["source_files"], [])

    def test_parent_directory_symlink_is_not_followed(self):
        self.session(self.root / "external/sessions/source.jsonl")
        alias = self.root / "alias"
        alias.symlink_to(self.root / "external", target_is_directory=True)
        report = diagnostic.diagnose(alias, SOURCE, self.data)
        self.assertFalse(report["complete"])
        self.assertEqual(report["source_files"], [])

    def test_limits_are_explicit(self):
        self.session(self.home / "sessions/source.jsonl")
        with patch.object(diagnostic, "MAX_HEADER", 5):
            self.assertFalse(self.check()["complete"])
        with patch.object(diagnostic, "MAX_ENTRIES", 0):
            report = self.check()
            self.assertFalse(report["complete"])
            self.assertIn("上限", report["anomalies"][0]["message"])

    def test_fork_metadata_without_history_base_is_not_dependency(self):
        path = self.session(self.home / "sessions/child.jsonl", CHILD)
        record = json.loads(path.read_text().splitlines()[0])
        record["payload"]["forked_from_id"] = SOURCE
        path.write_text(json.dumps(record) + "\n")
        self.assertEqual(self.check()["direct_dependents"], [])

    def test_bad_history_base_and_trash_path_are_incomplete(self):
        path = self.session(self.home / "sessions/bad.jsonl", CHILD)
        path.write_text(json.dumps({"type": "session_meta", "payload":
                                   {"id": CHILD, "history_base": []}}) + "\n")
        _, info, _ = self.trash()
        info.write_text("[Trash Info]\nPath=/bad%GG\nDeletionDate=now\n")
        report = self.check()
        self.assertFalse(report["complete"])
        self.assertEqual(report["anomaly_count"], 2)

    def test_duplicate_metadata_keys_are_incomplete(self):
        path = self.home / "sessions/duplicate.jsonl"
        path.write_text('{"type":"session_meta","payload":{"id":"' + SOURCE +
                        '","id":"' + CHILD + '"}}\n')
        self.assertFalse(self.check()["complete"])
        path.write_text('{"type":"session_meta","payload":{"id":"' + CHILD +
                        '","history_base":null,"history_base":{"thread_id":"' +
                        SOURCE + '"}}}\n')
        self.assertFalse(self.check()["complete"])

    def test_unterminated_header_and_unknown_history_mode(self):
        path = self.home / "sessions/partial.jsonl"
        value = {"type": "session_meta", "payload": {"id": SOURCE}}
        path.write_text(json.dumps(value))
        self.assertFalse(self.check()["complete"])
        value["payload"]["history_mode"] = "future"
        path.write_text(json.dumps(value) + "\n")
        self.assertFalse(self.check()["complete"])

    def test_multiple_sources_are_ambiguous(self):
        self.session(self.home / "sessions/source.jsonl")
        self.session(self.home / "archived_sessions/source.jsonl")
        report = self.check()
        self.assertFalse(report["complete"])
        self.assertEqual(len(report["source_files"]), 2)
        self.assertIn("歧义", report["anomalies"][0]["message"])

    def test_missing_sessions_is_incomplete(self):
        empty_home = self.root / "empty"
        empty_home.mkdir()
        report = diagnostic.diagnose(empty_home, SOURCE, self.data)
        self.assertFalse(report["complete"])
        self.assertIn("会话目录不存在", report["anomalies"][0]["message"])

    def test_total_byte_and_time_budgets(self):
        self.session(self.home / "sessions/source.jsonl")
        with patch.object(diagnostic, "MAX_SCAN_BYTES", 10):
            report = self.check()
            self.assertFalse(report["complete"])
            self.assertIn("累计读取", report["anomalies"][0]["message"])
            self.assertLessEqual(report["bytes_checked"], 11)
        with patch.object(diagnostic, "MAX_SCAN_SECONDS", -1):
            report = self.check()
            self.assertFalse(report["complete"])
            self.assertIn("耗时", report["anomalies"][0]["message"])


if __name__ == "__main__":
    unittest.main()
