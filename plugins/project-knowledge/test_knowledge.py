"""Offline domain and real stdio ABI regression tests; no live Runner required."""

import hashlib
import json
import os
from pathlib import Path
import queue
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

import knowledge
import plugin

SCRIPT = Path(__file__).with_name("plugin.py").resolve()


def rpc(method, request_id=1, **params):
    return {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}


def call(name, **arguments):
    return rpc("tools/call", name=name, arguments=arguments)


def frame(request):
    return json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n"


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / "repo"
        self.root.mkdir()
        self.cache = self.base / "cache"
        self.git("init", "-q")
        self.write("src/model.py", "def parseHTTPRequest(value):\n    return value  # parse request identifier\n")
        self.write("docs/guide.md", "# 项目知识\n项目知识支持中文检索，索引提供来源行号。\n")
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")
        self.provider = knowledge.Knowledge(self.root, self.cache)
        self.provider.initialize()

    def git(self, *arguments):
        result = subprocess.run(["git", *arguments], cwd=self.root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        self.assertEqual(result.returncode, 0, "fixture Git operation failed")
        return result.stdout

    def write(self, path, text):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
        return target


class KnowledgeTests(Fixture):
    def test_sensitive_assignments_preserve_key_and_value_detection(self):
        for text in ["PASSWORD='abcdefghijkl'", '"service.api-key":"abcdefghijkl"',
                     "foo=abcdefghijkl;api_key=abcdefghijkl", "app-private_key=abcdefghijkl"]:
            with self.subTest(text=text):
                self.assertTrue(knowledge.sensitive_text(text))
        for text in ["password='short'", "ordinary_key=abcdefghijkl", "api_key = 'short'"]:
            with self.subTest(text=text):
                self.assertFalse(knowledge.sensitive_text(text))

    def test_actual_fts5_capability_probe_and_failure(self):
        knowledge.probe_fts5()
        with patch.object(knowledge.sqlite3, "connect", side_effect=sqlite3.OperationalError("hidden diagnostic")):
            with self.assertRaisesRegex(knowledge.KnowledgeError, "FTS5 is unavailable"):
                self.provider.initialize()

    def test_unborn_git_root_has_hash_provenance_without_inventing_a_revision(self):
        root = self.base / "unborn"
        root.mkdir()
        subprocess.run(["git", "init", "-q", str(root)], check=True, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        (root / "guide.md").write_text("unbornneedle\n")
        subprocess.run(["git", "add", "guide.md"], cwd=root, check=True, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        provider = knowledge.Knowledge(root, self.base / "unborn-cache")
        provider.initialize()
        self.assertIsNone(provider.index()["coverage"]["revision"])
        hit = provider.search("unbornneedle")["hits"][0]
        self.assertIsNone(hit["index_revision"])
        self.assertIsNone(hit["observed_revision"])
        self.assertEqual(provider.read(hit["path"], hit["source_hash"])["text"], "unbornneedle\n")

    def test_bm25_identifier_and_cjk_search_provenance(self):
        self.assertEqual(self.provider.status(), {"indexed": False, "coverage": None})
        coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["indexed_files"], 2)
        self.assertEqual(coverage["stop_reason"], "complete")
        for query, path in [("parseHTTPRequest", "src/model.py"), ("request", "src/model.py"), ("项目知识", "docs/guide.md"), ("中文检索", "docs/guide.md")]:
            with self.subTest(query=query):
                result = self.provider.search(query)
                self.assertTrue(result["hits"])
                hit = result["hits"][0]
                self.assertEqual(hit["path"], path)
                self.assertEqual(hit["source_hash"], hashlib.sha256((self.root / path).read_bytes()).hexdigest())
                self.assertEqual(hit["index_revision"], self.git("rev-parse", "HEAD").decode().strip())
                self.assertEqual(hit["observed_revision"], hit["index_revision"])
                read = self.provider.read(path, hit["source_hash"], hit["start_line"], hit["end_line"])
                self.assertEqual(read["text"], hit["text"])
        self.assertEqual(self.provider.status()["coverage"], coverage)
        self.assertFalse(list(self.root.glob("**/*sqlite*")))

    def test_stale_ranked_candidates_do_not_hide_later_current_hits(self):
        for index in range(85):
            self.write(f"docs/candidate-{index:03}.md", "needle\n")
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "candidates")
        self.provider.index()
        for index in range(80):
            self.write(f"docs/candidate-{index:03}.md", "changed\n")
        result = self.provider.search("needle", 5)
        self.assertEqual(len(result["hits"]), 5)
        self.assertTrue(all(int(hit["path"][-6:-3]) >= 80 for hit in result["hits"]))
        self.assertEqual(result["stale_candidates_omitted"], 80)

    def test_changed_deleted_and_untracked_hits_are_never_returned_as_current(self):
        self.provider.index()
        old_hit = self.provider.search("parseHTTPRequest")["hits"][0]
        self.write("src/model.py", "def changed():\n    return 'different'\n")
        result = self.provider.search("parseHTTPRequest")
        self.assertEqual(result["hits"], [])
        self.assertGreater(result["stale_candidates_omitted"], 0)
        with self.assertRaisesRegex(knowledge.KnowledgeError, "hash changed"):
            self.provider.read(old_hit["path"], old_hit["source_hash"])
        self.provider.index()
        self.assertTrue(self.provider.search("changed")["hits"])
        (self.root / "src/model.py").unlink()
        self.assertEqual(self.provider.search("changed")["hits"], [])
        (self.root / "docs/guide.md").unlink()
        self.assertEqual(self.provider.search("项目知识")["hits"], [])
        self.write("src/model.py", "parseHTTPRequest untracked replacement\n")
        self.git("rm", "--cached", "src/model.py")
        with self.assertRaisesRegex(knowledge.KnowledgeError, "tracked file"):
            self.provider.read("src/model.py", old_hit["source_hash"])

    def test_cache_text_is_not_authoritative(self):
        self.provider.index()
        with sqlite3.connect(self.provider.database) as connection:
            connection.execute("UPDATE chunks SET text='fabricated stale text' WHERE path='src/model.py'")
        hit = self.provider.search("parseHTTPRequest")["hits"][0]
        self.assertIn("def parseHTTPRequest", hit["text"])
        self.assertNotIn("fabricated", hit["text"])

    def test_corrupt_chunk_ranges_are_rejected_before_source_projection(self):
        self.provider.index()
        for first, last in [("invalid", 2), (1, "invalid"), (0, 2), (-1, 2),
                            (2, 1), (1, 999), (1.5, 2), (1, 1.5)]:
            with self.subTest(first=first, last=last):
                with sqlite3.connect(self.provider.database) as connection:
                    connection.execute("UPDATE chunks SET start_line=?, end_line=? WHERE path='src/model.py'",
                                       (first, last))
                with self.assertRaisesRegex(knowledge.KnowledgeError, "source ranges; rebuild"):
                    self.provider.search("parseHTTPRequest")
        self.provider.index()
        self.assertEqual(self.provider.search("parseHTTPRequest")["hits"][0]["start_line"], 1)

    def test_corrupt_metadata_is_a_known_rebuildable_failure(self):
        coverage = self.provider.index()["coverage"]
        malformed = [None, "[]", "null", "{}", '{"format_version":1}',
                     json.dumps(dict(coverage, indexed_files=True)),
                     json.dumps(dict(coverage, revision="invalid")),
                     json.dumps(dict(coverage, stop_reason="unknown")),
                     json.dumps(dict(coverage, unprocessed_files=-1)),
                     json.dumps(dict(coverage, eligible_files=999)),
                     "[" * 1800 + "0" + "]" * 1800, "x" * 4097]
        for value in malformed:
            with self.subTest(value=str(value)[:60]):
                with sqlite3.connect(self.provider.database) as connection:
                    connection.execute("DELETE FROM metadata")
                    if value is not None:
                        connection.execute("INSERT INTO metadata VALUES (?)", (value,))
                for operation in [self.provider.status, lambda: self.provider.search("request")]:
                    with self.assertRaisesRegex(knowledge.KnowledgeError, "Derived index is unavailable; rebuild"):
                        operation()
        self.provider.index()
        with sqlite3.connect(self.provider.database) as connection:
            connection.execute("INSERT INTO metadata VALUES (?)", (json.dumps(coverage),))
        with self.assertRaises(knowledge.KnowledgeError):
            self.provider.status()
        self.assertTrue(self.provider.index()["indexed"])
        self.assertTrue(self.provider.search("request")["hits"])

    def test_tracked_only_sensitive_binary_oversize_hardlink_and_symlink_guards(self):
        self.write("untracked.md", "hiddenuntracked\n")
        self.write(".env", "hiddenenv\n")
        self.write("credentials.json", '{"value":"hiddencredential"}\n')
        self.write("settings.py", "password = '" + "s" * 32 + "'\n")
        self.write("config.json", '{"api_key":"' + "s" * 32 + '"}\n')
        self.write("key.txt", "-----BEGIN PRIVATE KEY-----\nhiddenkey\n")
        self.write("binary.txt", "hiddenbinary\0\n")
        self.write("huge.txt", "x" * (knowledge.MAX_FILE_BYTES + 1))
        outside = self.base / "outside.md"
        outside.write_text("outsideauthority\n")
        (self.root / "linked.md").symlink_to(outside)
        (self.root / "alias").symlink_to(self.root / "docs", target_is_directory=True)
        os.link(outside, self.root / "hardlink.md")
        self.git("add", ".env", "credentials.json", "settings.py", "config.json", "key.txt", "binary.txt", "huge.txt", "linked.md", "alias", "hardlink.md")
        coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["indexed_files"], 2)
        self.assertGreaterEqual(coverage["excluded_entries"], 4)
        self.assertGreaterEqual(coverage["skipped_files"], 6)
        for query in ["hiddenuntracked", "hiddencredential", "hiddenkey", "hiddenbinary", "outsideauthority"]:
            self.assertEqual(self.provider.search(query)["hits"], [])
        for path in ["../outside.md", "/outside.md", "docs/../guide.md", "docs//guide.md", "docs\\guide.md", ".git/config", ".env", "credentials.json", "alias/guide.md", "linked.md", "hardlink.md"]:
            with self.subTest(path=path):
                with self.assertRaises(knowledge.KnowledgeError):
                    self.provider.read(path, "0" * 64)

    def test_replaced_source_directory_and_root_fail_closed(self):
        self.provider.index()
        old_hit = self.provider.search("项目知识")["hits"][0]
        (self.root / "docs").rename(self.root / "old-docs")
        (self.root / "docs").symlink_to(self.root / "old-docs", target_is_directory=True)
        self.assertEqual(self.provider.search("项目知识")["hits"], [])
        with self.assertRaises(knowledge.KnowledgeError):
            self.provider.read(old_hit["path"], old_hit["source_hash"])
        self.root.rename(self.base / "moved")
        self.root.mkdir()
        with self.assertRaisesRegex(knowledge.KnowledgeError, "root changed"):
            self.provider.status()

    def test_directory_replacement_during_verification_does_not_return_old_bytes(self):
        self.provider.index()
        actual_read = knowledge.os.read
        replaced = []

        def replace_directory(descriptor, size):
            data = actual_read(descriptor, size)
            if data and not replaced:
                replaced.append(True)
                (self.root / "src").rename(self.root / "old-src")
                self.write("src/model.py", "different source\n")
            return data

        with patch.object(knowledge.os, "read", side_effect=replace_directory):
            self.assertEqual(self.provider.search("parseHTTPRequest")["hits"], [])

    def test_root_and_cache_guards(self):
        for root, cache in [(self.root / "src", self.base / "other-cache"), (self.root, self.root / "private-cache")]:
            with self.subTest(root=root, cache=cache):
                with self.assertRaises(knowledge.KnowledgeError):
                    knowledge.Knowledge(root, cache).initialize()
        alias = self.base / "repo-alias"
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(knowledge.KnowledgeError):
            knowledge.Knowledge(alias, self.base / "other-cache").initialize()
        public = self.base / "public-cache"
        public.mkdir(mode=0o755)
        with self.assertRaisesRegex(knowledge.KnowledgeError, "inaccessible"):
            knowledge.Knowledge(self.root, public).initialize()
        linked = self.base / "linked-cache"
        linked.symlink_to(self.cache, target_is_directory=True)
        with self.assertRaises(knowledge.KnowledgeError):
            knowledge.Knowledge(self.root, linked).initialize()

    def test_atomic_rebuild_failure_retains_previous_index(self):
        self.provider.index()
        before = self.provider.database.read_bytes()
        with patch.object(knowledge, "chunk_text", side_effect=sqlite3.OperationalError("private failure")):
            with self.assertRaisesRegex(knowledge.KnowledgeError, "previous complete index"):
                self.provider.index()
        self.assertEqual(self.provider.database.read_bytes(), before)
        self.assertTrue(self.provider.search("parseHTTPRequest")["hits"])
        self.assertFalse(list(self.provider.cache.glob("build-*")))

    def test_mid_file_deadline_publishes_only_complete_sources_with_truthful_coverage(self):
        self.provider.index()
        before = self.provider.database.read_bytes()
        actual_time = knowledge.time.monotonic
        actual_chunks = knowledge.chunk_text
        elapsed = [0]

        def delayed_chunks(text):
            for chunk in actual_chunks(text):
                if "parseHTTPRequest" in text:
                    elapsed[0] = knowledge.MAX_INDEX_SECONDS + 1
                yield chunk

        with patch.object(knowledge.time, "monotonic", side_effect=lambda: actual_time() + elapsed[0]):
            with patch.object(knowledge, "chunk_text", side_effect=delayed_chunks):
                result = self.provider.index()
        coverage = result["coverage"]
        self.assertEqual(coverage["stop_reason"], "time_limit")
        self.assertEqual((coverage["eligible_files"], coverage["indexed_files"],
                          coverage["skipped_files"], coverage["unprocessed_files"]), (2, 1, 0, 1))
        self.assertNotEqual(self.provider.database.read_bytes(), before)
        self.assertEqual(self.provider.status()["coverage"], coverage)
        with sqlite3.connect(self.provider.database) as database:
            self.assertEqual(database.execute("SELECT path FROM sources").fetchall(), [("docs/guide.md",)])
            self.assertEqual(database.execute("SELECT DISTINCT path FROM chunks").fetchall(), [("docs/guide.md",)])
        hit = self.provider.search("中文检索")["hits"][0]
        self.assertEqual(hit["path"], "docs/guide.md")
        self.assertEqual(self.provider.read(hit["path"], hit["source_hash"],
                                            hit["start_line"], hit["end_line"])["text"], hit["text"])
        self.assertEqual(self.provider.search("parseHTTPRequest")["hits"], [])
        self.assertFalse(list(self.provider.cache.glob("build-*")))

    def test_source_deadline_is_not_misreported_as_skipped(self):
        original_source = self.provider.source
        original_time = knowledge.time.monotonic
        elapsed = [0]

        def expire_during_source(path, deadline=None):
            if path == "src/model.py":
                elapsed[0] = knowledge.MAX_INDEX_SECONDS + 1
            return original_source(path, deadline)

        with patch.object(knowledge.time, "monotonic", side_effect=lambda: original_time() + elapsed[0]):
            with patch.object(self.provider, "source", side_effect=expire_during_source):
                coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["stop_reason"], "time_limit")
        self.assertEqual((coverage["indexed_files"], coverage["skipped_files"],
                          coverage["unprocessed_files"]), (1, 0, 1))
        self.assertTrue(self.provider.search("中文检索")["hits"])

    def test_sqlite_progress_interrupt_does_not_lose_prior_committed_files(self):
        self.write("early.py", "def earlyneedle(): pass\n")
        self.write("later.py", "def laterneedle(): pass\n")
        self.write("middle.py", "def middleneedle(): pass\n")
        self.git("add", "early.py", "later.py", "middle.py")
        self.provider.index()
        original_time = knowledge.time.monotonic
        original_terms = knowledge.terms
        original_connect = knowledge.sqlite3.connect
        elapsed = [0]

        class AggressiveProgress(sqlite3.Connection):
            def set_progress_handler(self, callback, instructions):
                return super().set_progress_handler(callback, 1 if callback is not None else 0)

        def trigger_after_time_check(text):
            if "parseHTTPRequest" in text:
                elapsed[0] = knowledge.MAX_INDEX_SECONDS + 1
            return original_terms(text)

        def connect(*args, **kwargs):
            return original_connect(*args, factory=AggressiveProgress, **kwargs)

        with patch.object(knowledge, "INDEX_CHECKPOINT_FILES", 2):
            with patch.object(knowledge.time, "monotonic", side_effect=lambda: original_time() + elapsed[0]):
                with patch.object(knowledge.sqlite3, "connect", side_effect=connect):
                    with patch.object(knowledge, "terms", side_effect=trigger_after_time_check):
                        coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["stop_reason"], "time_limit")
        self.assertEqual((coverage["indexed_files"], coverage["skipped_files"],
                          coverage["unprocessed_files"]), (3, 0, 2))
        with sqlite3.connect(self.provider.database) as database:
            self.assertEqual(database.execute("SELECT path FROM sources ORDER BY path").fetchall(),
                             [("docs/guide.md",), ("early.py",), ("later.py",)])
        self.assertTrue(self.provider.search("中文检索")["hits"])
        self.assertTrue(self.provider.search("earlyneedle")["hits"])
        self.assertTrue(self.provider.search("laterneedle")["hits"])
        self.assertFalse(self.provider.search("middleneedle")["hits"])
        self.assertFalse(self.provider.search("parseHTTPRequest")["hits"])
        self.assertFalse(list(self.provider.cache.glob("build-*")))

    def test_genuine_sqlite_failure_after_first_checkpoint_retains_published_cache(self):
        self.provider.index()
        before = self.provider.database.read_bytes()
        original_chunks = knowledge.chunk_text

        def fail_second_file(text):
            if "parseHTTPRequest" in text:
                raise sqlite3.OperationalError("opaque build failure")
            yield from original_chunks(text)

        with patch.object(knowledge, "chunk_text", side_effect=fail_second_file):
            with self.assertRaisesRegex(knowledge.KnowledgeError, "previous complete index"):
                self.provider.index()
        self.assertEqual(self.provider.database.read_bytes(), before)
        self.assertTrue(self.provider.search("parseHTTPRequest")["hits"])
        self.assertFalse(list(self.provider.cache.glob("build-*")))

    def test_git_output_bound_retain_previous_cache(self):
        self.provider.index()
        before = self.provider.database.read_bytes()
        with patch.object(knowledge, "MAX_GIT_BYTES", 64):
            with self.assertRaisesRegex(knowledge.KnowledgeError, "Git output limit"):
                self.provider.index()
        self.assertEqual(self.provider.database.read_bytes(), before)
        self.assertFalse(list(self.provider.cache.glob("build-*")))

    def test_bounds_coverage_and_current_read_range(self):
        for number in range(8):
            self.write(f"many/{number}.md", f"needle unique{number}\n" * 80)
        self.git("add", "many")
        with patch.object(knowledge, "MAX_FILES", 3):
            coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["indexed_files"], 3)
        self.assertEqual(coverage["stop_reason"], "file_limit")
        self.assertEqual(coverage["unprocessed_files"], 7)
        with patch.object(knowledge, "MAX_CORPUS_BYTES", 10):
            coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["stop_reason"], "corpus_limit")
        self.assertEqual(coverage["indexed_files"], 0)
        self.provider.index()
        default = self.provider.search("needle")
        self.assertEqual(len(default["hits"]), 5)
        maximum = self.provider.search("needle", 20)
        self.assertLessEqual(len(maximum["hits"]), 20)
        self.assertLessEqual(knowledge.encoded_size(maximum), knowledge.MAX_RESULT_BYTES)
        hit = default["hits"][0]
        read = self.provider.read(hit["path"], hit["source_hash"], 2, 3)
        self.assertEqual(read["start_line"], 2)
        self.assertEqual(read["end_line"], 3)
        self.assertEqual(len(read["text"].splitlines()), 2)
        for arguments in [("", 5), ("x" * 513, 5), ("needle", 0), ("needle", 21), ("needle", True)]:
            with self.assertRaises(knowledge.KnowledgeError):
                self.provider.search(*arguments)
        for first, last in [(0, None), (True, None), (3, 2), (10000, None)]:
            with self.assertRaises(knowledge.KnowledgeError):
                self.provider.read(hit["path"], hit["source_hash"], first, last)

    def test_long_lines_report_coverage_and_do_not_claim_complete_excerpts(self):
        self.write("long.md", "needle " * 1000 + "\nshortneedle\n")
        self.git("add", "long.md")
        coverage = self.provider.index()["coverage"]
        self.assertEqual(coverage["oversized_lines_omitted"], 1)
        hit = self.provider.search("shortneedle")["hits"][0]
        self.assertEqual((hit["start_line"], hit["end_line"]), (2, 2))
        # An even larger line cannot be returned under the wire bound.
        self.write("long.md", "x" * 20000 + "\n")
        with self.assertRaisesRegex(knowledge.KnowledgeError, "line exceeds"):
            self.provider.read("long.md", knowledge.digest((self.root / "long.md").read_bytes()), 1, 1)


class NativeProtocolTests(Fixture):
    def environment(self):
        environment = dict(os.environ)
        environment["XDG_CACHE_HOME"] = str(self.base / "protocol-cache")
        environment["PYTHONIOENCODING"] = "ascii"
        environment.pop("PYTHONUNBUFFERED", None)
        return environment

    def run_wire(self, data, command=None, environment=None, timeout=30):
        result = subprocess.run(command or [sys.executable, "-B", str(SCRIPT)], input=data,
                                cwd=self.root, env=environment or self.environment(),
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b"")
        responses = []
        for line in result.stdout.splitlines():
            self.assertLessEqual(len(line), plugin.MAX_OUTPUT_BYTES)
            response = json.loads(line)
            if "result" in response and "content" in response["result"]:
                self.assertLessEqual(knowledge.encoded_size(response["result"]), knowledge.MAX_RESULT_BYTES)
            responses.append(response)
        return responses

    def test_long_punctuation_source_completes_without_quadratic_sensitive_scan(self):
        text = "-." * ((knowledge.MAX_FILE_BYTES - 32) // 2) + "\nlongpunctuationneedle\n"
        self.write("punctuation.md", text)
        self.git("add", "punctuation.md")
        messages = [rpc("initialize", protocolVersion="webcodex-plugin-v1"), call("index"),
                    call("search", query="longpunctuationneedle"),
                    call("read", path="punctuation.md", source_hash=knowledge.digest(text.encode()),
                         start_line=2, end_line=2)]
        responses = self.run_wire(b"".join(frame(request) for request in messages), timeout=5)
        self.assertEqual(responses[1]["result"]["structuredContent"]["coverage"]["indexed_files"], 3)
        self.assertEqual(responses[2]["result"]["structuredContent"]["hits"][0]["path"], "punctuation.md")
        self.assertEqual(responses[3]["result"]["structuredContent"]["text"], "longpunctuationneedle\n")

    def test_corrupt_metadata_does_not_terminate_protocol(self):
        provider = knowledge.Knowledge(self.root, self.base / "protocol-cache" / "webcodex-project-knowledge")
        provider.initialize()
        provider.index()
        for value in ["[]", '{"format_version":1}']:
            with self.subTest(value=value):
                with sqlite3.connect(provider.database) as connection:
                    connection.execute("UPDATE metadata SET value=?", (value,))
                messages = [rpc("initialize", protocolVersion="webcodex-plugin-v1"),
                            call("status"), call("search", query="request"), call("index"), call("status")]
                responses = self.run_wire(b"".join(frame(request) for request in messages))
                for response in responses[1:3]:
                    self.assertTrue(response["result"]["isError"])
                    self.assertEqual(response["result"]["content"][0]["text"],
                                     "Derived index is unavailable; rebuild the index.")
                self.assertTrue(responses[-1]["result"]["structuredContent"]["indexed"])

    def test_corrupt_chunk_ranges_do_not_terminate_protocol(self):
        provider = knowledge.Knowledge(self.root, self.base / "protocol-cache" / "webcodex-project-knowledge")
        provider.initialize()
        provider.index()
        for first, last in [("invalid", 2), (0, 2), (1, 999)]:
            with self.subTest(first=first, last=last):
                with sqlite3.connect(provider.database) as connection:
                    connection.execute("UPDATE chunks SET start_line=?, end_line=? WHERE path='src/model.py'",
                                       (first, last))
                messages = [rpc("initialize", protocolVersion="webcodex-plugin-v1"),
                            call("search", query="parseHTTPRequest"), call("index"),
                            call("search", query="parseHTTPRequest")]
                responses = self.run_wire(b"".join(frame(request) for request in messages))
                self.assertTrue(responses[1]["result"]["isError"])
                self.assertIn("source ranges; rebuild", responses[1]["result"]["content"][0]["text"])
                hit = responses[-1]["result"]["structuredContent"]["hits"][0]
                self.assertEqual((hit["start_line"], hit["end_line"]), (1, 2))

    def test_initialize_catalog_index_search_and_read_real_protocol(self):
        messages = [rpc("initialize", protocolVersion="webcodex-plugin-v1"), rpc("tools/list"), call("status"), call("index"), call("search", query="项目知识"), call("search", query="parseHTTPRequest")]
        responses = self.run_wire(b"".join(frame(request) for request in messages))
        self.assertEqual(responses[0]["result"], {"protocolVersion": "webcodex-plugin-v1"})
        tools = responses[1]["result"]["tools"]
        self.assertEqual({tool["name"] for tool in tools}, {"index", "status", "search", "read"})
        for tool in tools:
            self.assertTrue(tool["projectBound"])
            self.assertFalse(tool["inputSchema"]["additionalProperties"])
            self.assertNotIn("root", tool["inputSchema"]["properties"])
            self.assertEqual(tool["annotations"]["readOnlyHint"], tool["name"] != "index")
        self.assertFalse(responses[2]["result"]["structuredContent"]["indexed"])
        self.assertEqual(responses[3]["result"]["structuredContent"]["coverage"]["indexed_files"], 2)
        self.assertEqual(responses[4]["result"]["structuredContent"]["hits"][0]["path"], "docs/guide.md")
        hit = responses[5]["result"]["structuredContent"]["hits"][0]
        responses = self.run_wire(frame(messages[0]) + frame(call("read", path=hit["path"], source_hash=hit["source_hash"], start_line=1, end_line=1)))
        self.assertEqual(responses[1]["result"]["structuredContent"]["text"], "def parseHTTPRequest(value):\n")
        self.assertEqual(self.git("status", "--porcelain"), b"")

    def test_wire_failures_recover_without_reflecting_private_inputs(self):
        messages = [rpc("initialize", protocolVersion="webcodex-plugin-v1"), call("index", root="PRIVATE_INPUT"), call("search", query="x", limit=True), call("read", path="../outside.md", source_hash="0" * 64), call("index"), call("search", query="request", limit=21), call("status")]
        responses = self.run_wire(b"{\n" + b"".join(frame(request) for request in messages))
        self.assertEqual(responses[0]["error"]["code"], -32700)
        self.assertEqual(responses[2]["error"]["code"], -32602)
        self.assertEqual(responses[3]["error"]["code"], -32602)
        self.assertTrue(responses[4]["result"]["isError"])
        self.assertTrue(responses[6]["result"]["isError"])
        self.assertFalse(responses[-1]["result"]["isError"])
        self.assertNotIn("PRIVATE_INPUT", json.dumps(responses))
        oversized = b"x" * (plugin.MAX_INPUT_BYTES + 1) + b"\n"
        responses = self.run_wire(oversized + frame(messages[0]))
        self.assertEqual(responses[0]["error"]["code"], -32600)
        self.assertIn("result", responses[1])

    def test_wire_invalid_numbers_structure_and_unicode_fail_closed(self):
        frames = [b'null\n', b'{"jsonrpc":"2.0","id":true,"method":"tools/list"}\n',
                  b'{"jsonrpc":"2.0","id":NaN,"method":"tools/list"}\n',
                  b'{"jsonrpc":"2.0","id":1e9999,"method":"tools/list"}\n',
                  b'{"jsonrpc":"2.0","id":"\\ud800","method":"tools/list"}\n',
                  b'{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search","arguments":{"query":"\\ud800"}}}\n']
        for bad in frames:
            with self.subTest(frame=bad[:60]):
                responses = self.run_wire(bad + frame(rpc("tools/list")))
                self.assertIn("error", responses[0])
                self.assertIn("result", responses[1])

    def test_initialize_checks_fts5_not_just_catalog(self):
        wrapper = "import sys, sqlite3; sys.path.insert(0, sys.argv[1]); import plugin; sqlite3.connect = lambda *a, **k: (_ for _ in ()).throw(sqlite3.OperationalError('PRIVATE_SQLITE_ERROR')); plugin.serve(sys.stdin.buffer, sys.stdout.buffer)"
        responses = self.run_wire(frame(rpc("initialize", protocolVersion="webcodex-plugin-v1")), [sys.executable, "-B", "-c", wrapper, str(SCRIPT.parent)])
        self.assertEqual(responses[0]["error"]["code"], -32603)
        self.assertIn("FTS5", responses[0]["error"]["message"])
        self.assertNotIn("PRIVATE_SQLITE_ERROR", json.dumps(responses))

    def test_git_environment_cannot_select_another_repository(self):
        environment = self.environment()
        environment["GIT_DIR"] = str(self.base / "nonexistent")
        environment["GIT_WORK_TREE"] = str(self.base)
        responses = self.run_wire(frame(rpc("initialize", protocolVersion="webcodex-plugin-v1")) + frame(call("index")), environment=environment)
        self.assertIn("result", responses[0])
        self.assertEqual(responses[1]["result"]["structuredContent"]["coverage"]["indexed_files"], 2)

    def test_interactive_flush_before_eof(self):
        process = subprocess.Popen([sys.executable, "-B", str(SCRIPT)], cwd=self.root,
                                   env=self.environment(), stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        output = queue.Queue()
        errors = []

        def drain():
            for line in iter(process.stdout.readline, b""):
                output.put(line)

        threads = [threading.Thread(target=drain), threading.Thread(target=lambda: errors.append(process.stderr.read()))]
        for thread in threads:
            thread.start()
        try:
            for request in [rpc("initialize", protocolVersion="webcodex-plugin-v1"), call("index"), call("search", query="中文检索")]:
                data = frame(request)
                process.stdin.write(data[:3])
                process.stdin.flush()
                process.stdin.write(data[3:])
                process.stdin.flush()
                response = json.loads(output.get(timeout=30))
                self.assertIn("result", response)
            self.assertTrue(response["result"]["structuredContent"]["hits"])
            self.assertIsNone(process.poll())
            process.stdin.close()
            self.assertEqual(process.wait(timeout=10), 0)
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
            for thread in threads:
                thread.join(timeout=10)
            for stream in (process.stdin, process.stdout, process.stderr):
                stream.close()
        self.assertEqual(errors, [b""])


if __name__ == "__main__":
    unittest.main()
