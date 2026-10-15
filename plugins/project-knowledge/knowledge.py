"""Bounded, local, derived project knowledge. Python 3.10+, standard library only."""

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import sqlite3
import stat
import subprocess
import tempfile
import threading
import time
import unicodedata
from urllib.parse import quote

MAX_FILES = 4096
MAX_FILE_BYTES = 256 * 1024
MAX_CORPUS_BYTES = 64 * 1024 * 1024
MAX_INDEX_SECONDS = 20
INDEX_CHECKPOINT_FILES = 512  # At most this many committed-work candidates per batch.
INDEX_FINAL_CHECKPOINT_MARGIN = 1.0  # Publish a checkpoint shortly before expiry.
MAX_GIT_BYTES = 8 * 1024 * 1024
MAX_RESULT_BYTES = 16 * 1024
MAX_PATH_CHARS = 1024
MAX_RESULTS = 20
CHUNK_LINES = 48
CHUNK_BYTES = 4096
FORMAT_VERSION = 1
EXTENSIONS = frozenset(".md .markdown .txt .rst .adoc .rs .py .js .jsx .ts .tsx .mjs .cjs .c .h .cpp .hpp .cc .cs .go .java .kt .kts .swift .rb .php .sh .bash .zsh .fish .ps1 .sql .html .css .scss .vue .svelte .lua .r .jl .ex .exs .erl .hrl .hs .scala .dart .toml .yaml .yml .json .xml .proto .graphql".split())
NAMES = frozenset({"Dockerfile", "Makefile", "CMakeLists.txt", "LICENSE"})
SENSITIVE_PART = re.compile(r"(?:^|[._-])(?:secrets?|credentials?|tokens?|passwords?|private[-_]?keys?)(?:$|[._-])", re.I)
SENSITIVE_TEXT = re.compile(r"-----BEGIN (?:[A-Z ]*PRIVATE KEY|OPENSSH PRIVATE KEY)-----|\bAKIA[0-9A-Z]{16}\b|\b(?:ghp_|github_pat_)[A-Za-z0-9_]{20,}|\b(?:sk-live-|sk-proj-)[A-Za-z0-9_-]{16,}")
# Start only at the beginning of a complete key token. Retrying a greedy key
# prefix at every '-' or '.' makes otherwise bounded source scans quadratic.
ASSIGNMENT = re.compile(r"(?<![\w.-])[\"']?([\w.-]+)[\"']?\s*[:=]")
SENSITIVE_KEY = re.compile(r"secret|password|api[_-]?key|access[_-]?token|auth[_-]?token|private[_-]?key", re.I)
SENSITIVE_VALUE = re.compile(r"\s*[\"']?[^\s\"'<>]{12}")


def sensitive_text(text):
    return bool(SENSITIVE_TEXT.search(text)) or any(
        SENSITIVE_KEY.search(match[1]) and SENSITIVE_VALUE.match(text, match.end())
        for match in ASSIGNMENT.finditer(text)
    )


def valid_metadata(value):
    counts = {"indexed_at_unix", "eligible_files", "indexed_files", "indexed_bytes",
              "chunks", "excluded_entries", "skipped_files", "oversized_lines_omitted",
              "unprocessed_files"}
    if not isinstance(value, dict) or set(value) != counts | {"format_version", "revision", "stop_reason"}:
        return False
    if type(value["format_version"]) is not int or value["format_version"] != FORMAT_VERSION:
        return False
    if any(type(value[key]) is not int or value[key] < 0 for key in counts):
        return False
    revision = value["revision"]
    return (
        (revision is None or isinstance(revision, str) and re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", revision) is not None)
        and value["stop_reason"] in ("complete", "time_limit", "file_limit", "corpus_limit")
        and value["indexed_files"] <= MAX_FILES
        and value["indexed_bytes"] <= MAX_CORPUS_BYTES
        and value["eligible_files"] == value["indexed_files"] + value["skipped_files"] + value["unprocessed_files"]
    )


class KnowledgeError(Exception):
    """Known local application failure with a fixed, safe message."""


class IndexBudgetExpired(Exception):
    """Internal signal: stop the current file, publish prior complete files."""


def digest(data):
    return hashlib.sha256(data).hexdigest()


def eligible_path(path):
    if not isinstance(path, str) or not path or len(path) > MAX_PATH_CHARS:
        return False
    if "\\" in path or any(ord(char) < 32 or ord(char) == 127 for char in path):
        return False
    parts = PurePosixPath(path).parts
    if not parts or path.startswith("/") or any(part in (".", "..") or part.startswith(".") for part in parts):
        return False
    if "/".join(parts) != path or any(SENSITIVE_PART.search(part) for part in parts):
        return False
    leaf = parts[-1]
    return Path(leaf).suffix.lower() in EXTENSIONS or leaf in NAMES


def terms(text):
    """Preserve identifiers, add snake/camel components, and segment CJK bigrams."""
    normalized = unicodedata.normalize("NFKC", text)
    output = []
    for token in re.findall(r"[A-Za-z_][A-Za-z_0-9]*|[0-9]+|[\u3400-\u9fff]+", normalized):
        if "\u3400" <= token[0] <= "\u9fff":
            output.extend(token[index:index + 2] for index in range(max(1, len(token) - 1)))
        else:
            output.append(token.lower())
            output.extend(piece.lower() for piece in re.findall(r"[A-Z]+(?=[A-Z][a-z]|\d|_|$)|[A-Z]?[a-z]+|[0-9]+", token))
    return output


def git(root, arguments, deadline):
    """Read bounded subprocess output; no shell, terminal, network, or raw errors."""
    timeout = deadline - time.monotonic()
    if timeout <= 0:
        raise KnowledgeError("Local Git observation exceeded its deadline.")
    environment = dict(os.environ)
    # Prepared provider cwd is the sole repository selector, including when the
    # Runner's inherited environment contains Git overrides.
    for key in list(environment):
        if key.startswith("GIT_"):
            environment.pop(key)
    environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_OPTIONAL_LOCKS="0")
    try:
        process = subprocess.Popen(
            ["git", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", *arguments],
            cwd=root, env=environment, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
    except OSError:
        raise KnowledgeError("Local Git is unavailable.") from None
    output = bytearray()
    oversized = []

    def drain():
        while True:
            block = process.stdout.read(min(65536, MAX_GIT_BYTES + 1 - len(output)))
            if not block:
                return
            output.extend(block)
            if len(output) > MAX_GIT_BYTES:
                oversized.append(True)
                process.kill()
                return

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    try:
        process.wait(timeout=timeout)
        reader.join(timeout=max(0, deadline - time.monotonic()))
        if reader.is_alive():
            raise subprocess.TimeoutExpired("git", timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        reader.join(timeout=1)
        raise KnowledgeError("Local Git observation exceeded its deadline.") from None
    finally:
        if not reader.is_alive():
            process.stdout.close()
    if oversized:
        raise KnowledgeError("Tracked-file observation exceeds the bounded Git output limit.")
    if process.returncode:
        raise KnowledgeError("Configured cwd must be an accessible Git working-tree root.")
    return bytes(output)


def probe_fts5():
    try:
        with sqlite3.connect(":memory:") as connection:
            connection.execute("CREATE VIRTUAL TABLE capability_probe USING fts5(tokens)")
            connection.execute("INSERT INTO capability_probe VALUES ('probe')")
            if not connection.execute("SELECT rowid FROM capability_probe WHERE capability_probe MATCH 'probe'").fetchone():
                raise sqlite3.OperationalError("FTS5 probe did not match")
    except sqlite3.Error:
        raise KnowledgeError("Python SQLite FTS5 is unavailable; install a Python build with FTS5 and check this provider again.") from None


def private_directory(path):
    # Cache directories are operator-local, never supplied by a tools/call.
    if path.is_symlink():
        raise KnowledgeError("Cache directory must be an ordinary private directory.")
    try:
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = path.stat()
    except OSError:
        raise KnowledgeError("Private cache directory is unavailable.") from None
    if not stat.S_ISDIR(info.st_mode) or (os.name == "posix" and (info.st_uid != os.getuid() or info.st_mode & 0o077)):
        raise KnowledgeError("Cache directory must be owned by this user and inaccessible to other users.")


class Knowledge:
    def __init__(self, root=None, cache_home=None):
        self.root = Path(root if root is not None else os.getcwd()).absolute()
        self.cache_home = Path(cache_home) if cache_home is not None else Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "webcodex-project-knowledge"
        self.cache = None
        self.database = None
        self.root_identity = None

    def initialize(self):
        probe_fts5()  # Exercise actual CREATE/MATCH; compile-option strings are insufficient.
        deadline = time.monotonic() + 5
        try:
            resolved = self.root.resolve(strict=True)
            observed = Path(os.fsdecode(git(self.root, ["rev-parse", "--show-toplevel"], deadline)).strip()).resolve(strict=True)
        except OSError:
            raise KnowledgeError("Configured cwd is unavailable.") from None
        if self.root != resolved or observed != resolved or resolved == Path(resolved.anchor) or resolved == Path.home().resolve():
            raise KnowledgeError("Configure one ordinary Git working-tree root as provider cwd; filesystem and home roots are excluded.")
        # Fail closed rather than silently using race-prone path traversal on a
        # Python/OS lacking descriptor-relative no-follow reads.
        if not hasattr(os, "O_NOFOLLOW") or os.open not in os.supports_dir_fd:
            raise KnowledgeError("This provider requires descriptor-relative no-follow file access on this host.")
        self.root = resolved
        root_stat = self.root.stat()
        self.root_identity = (root_stat.st_dev, root_stat.st_ino)
        cache_home = self.cache_home.absolute()
        resolved_cache = cache_home.resolve()
        if resolved_cache == self.root or self.root in resolved_cache.parents or cache_home != resolved_cache:
            raise KnowledgeError("Cache must be outside the Project and must not traverse symlinks.")
        private_directory(cache_home)
        self.cache = cache_home / digest(os.fsencode(self.root))
        private_directory(self.cache)
        self.database = self.cache / "index.sqlite3"

    def require_initialized(self):
        if self.database is None:
            raise KnowledgeError("Initialize this provider before calling tools.")
        try:
            info = self.root.lstat()
            if not stat.S_ISDIR(info.st_mode) or (info.st_dev, info.st_ino) != self.root_identity:
                raise OSError("root identity changed")
        except OSError:
            raise KnowledgeError("Configured Project root changed; check and reload this provider before proceeding.") from None

    def revision(self, deadline):
        # --revs-only is empty for an unborn HEAD without treating an otherwise
        # valid new working tree as a repository-access failure.
        return git(self.root, ["rev-parse", "--revs-only", "HEAD"], deadline).decode("ascii").strip() or None

    def tracked(self, deadline):
        raw = git(self.root, ["ls-files", "--cached", "--stage", "-z"], deadline)
        paths = set()
        skipped = 0
        for record in raw.split(b"\0"):
            if not record:
                continue
            try:
                header, raw_path = record.split(b"\t", 1)
                mode, _object_hash, stage = header.split(b" ")
                path = raw_path.decode("utf-8")
            except (ValueError, UnicodeError):
                skipped += 1
                continue
            if mode not in (b"100644", b"100755") or stage != b"0" or not eligible_path(path):
                skipped += 1
                continue
            paths.add(path)
        return sorted(paths), skipped

    def open_source(self, path):
        descriptors = []
        try:
            flags = os.O_RDONLY | os.O_NOFOLLOW | getattr(os, "O_NONBLOCK", 0)
            directory = os.open(self.root, flags | os.O_DIRECTORY)
            descriptors.append(directory)
            root_stat = os.fstat(directory)
            if (root_stat.st_dev, root_stat.st_ino) != self.root_identity:
                raise KnowledgeError("Configured Project root identity changed.")
            parts = PurePosixPath(path).parts
            for part in parts[:-1]:
                directory = os.open(part, flags | os.O_DIRECTORY, dir_fd=directory)
                descriptors.append(directory)
            file_descriptor = os.open(parts[-1], flags, dir_fd=directory)
            descriptors.append(file_descriptor)
            return descriptors
        except BaseException:
            for descriptor in reversed(descriptors):
                os.close(descriptor)
            raise

    def source(self, path, deadline=None):
        if not eligible_path(path):
            raise KnowledgeError("Source path is not eligible for project knowledge.")
        descriptors = []
        try:
            descriptors = self.open_source(path)
            file_descriptor = descriptors[-1]
            before = os.fstat(file_descriptor)
            if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_FILE_BYTES or before.st_nlink != 1:
                raise KnowledgeError("Source is not a bounded ordinary file.")
            data = bytearray()
            while len(data) <= MAX_FILE_BYTES:
                if deadline is not None and time.monotonic() >= deadline:
                    raise KnowledgeError("Source verification exceeded the local work budget.")
                block = os.read(file_descriptor, min(65536, MAX_FILE_BYTES + 1 - len(data)))
                if not block:
                    break
                data.extend(block)
            after = os.fstat(file_descriptor)
            if len(data) > MAX_FILE_BYTES or after.st_nlink != 1 or (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
                raise KnowledgeError("Source changed or exceeded its bound during reading.")
            # Reopen the complete relative path after reading: a renamed parent
            # must not leave old descriptor bytes represented as the current path.
            current = self.open_source(path)
            try:
                fresh = os.fstat(current[-1])
                if (fresh.st_dev, fresh.st_ino, fresh.st_size, fresh.st_mtime_ns, fresh.st_ctime_ns, fresh.st_nlink) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns, after.st_nlink):
                    raise KnowledgeError("Source path changed during verification.")
            finally:
                for descriptor in reversed(current):
                    os.close(descriptor)
            text = data.decode("utf-8")
            if "\0" in text or any(ord(char) < 32 and char not in "\t\n\r" for char in text) or sensitive_text(text):
                raise KnowledgeError("Binary or potentially sensitive source is excluded.")
            return bytes(data), text
        except (OSError, UnicodeError):
            raise KnowledgeError("Source is unavailable or is not ordinary UTF-8 text.") from None
        finally:
            for descriptor in reversed(descriptors):
                os.close(descriptor)

    def connect(self):
        self.require_initialized()
        connection = None
        try:
            info = self.database.lstat()
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or (os.name == "posix" and (info.st_uid != os.getuid() or info.st_mode & 0o077)):
                raise KnowledgeError("Derived cache is not an ordinary private file; rebuild it locally.")
            connection = sqlite3.connect("file:" + quote(str(self.database)) + "?mode=ro", uri=True, timeout=1)
            connection.row_factory = sqlite3.Row
            rows = connection.execute("SELECT substr(value, 1, 4097) FROM metadata LIMIT 2").fetchall()
            if len(rows) != 1 or not isinstance(rows[0][0], str) or len(rows[0][0]) > 4096:
                raise ValueError("invalid metadata record")
            metadata = json.loads(rows[0][0])
            if not valid_metadata(metadata):
                raise ValueError("invalid metadata structure")
            return connection, metadata
        except FileNotFoundError:
            raise KnowledgeError("No derived index exists; call index first.") from None
        except (OSError, sqlite3.Error, ValueError, TypeError, RecursionError):
            if connection is not None:
                connection.close()
            raise KnowledgeError("Derived index is unavailable; rebuild the index.") from None

    def status(self):
        self.require_initialized()
        if not self.database.exists():
            return {"indexed": False, "coverage": None}
        connection, metadata = self.connect()
        connection.close()
        return {"indexed": True, "coverage": metadata}

    def index(self):
        self.require_initialized()
        started = time.monotonic()
        deadline = started + MAX_INDEX_SECONDS
        paths, excluded = self.tracked(deadline)
        revision = self.revision(deadline)
        descriptor, temporary = tempfile.mkstemp(prefix="build-", suffix=".sqlite3", dir=self.cache)
        os.close(descriptor)
        connection = None
        try:
            connection = sqlite3.connect(temporary)
            # The build database is private and disposable until fsync + atomic
            # publication. Checkpoint commits retain complete-file progress if
            # SQLite interrupts and rolls back an entire in-flight transaction.
            connection.execute("PRAGMA journal_mode=MEMORY")
            connection.execute("PRAGMA synchronous=OFF")
            progress = lambda: int(time.monotonic() >= deadline)
            connection.set_progress_handler(progress, 10000)
            connection.execute("CREATE TABLE metadata(value TEXT NOT NULL)")
            connection.execute("CREATE TABLE sources(path TEXT PRIMARY KEY, source_hash TEXT NOT NULL)")
            connection.execute("CREATE TABLE chunks(id INTEGER PRIMARY KEY, path TEXT NOT NULL, start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, text TEXT NOT NULL)")
            connection.execute("CREATE VIRTUAL TABLE search USING fts5(tokens)")
            count = size = skipped = chunks = oversized_lines = 0
            checkpoint = (count, size, chunks, oversized_lines)
            final_checkpointed = False
            connection.execute("BEGIN")
            stop_reason = "complete"
            for path in paths:
                if time.monotonic() >= deadline:
                    stop_reason = "time_limit"
                    break
                if count >= MAX_FILES:
                    stop_reason = "file_limit"
                    break
                try:
                    data, text = self.source(path, deadline)
                except KnowledgeError:
                    self.require_initialized()
                    if time.monotonic() >= deadline:
                        stop_reason = "time_limit"
                        break  # An unfinished read is unprocessed, not skipped.
                    skipped += 1
                    continue
                if size + len(data) > MAX_CORPUS_BYTES:
                    stop_reason = "corpus_limit"
                    break
                file_chunks = 0
                file_oversized = sum(len(line.encode("utf-8")) > CHUNK_BYTES for line in text.splitlines(keepends=True))
                try:
                    # Batch inserts avoid per-source FTS5 SAVEPOINT flushes.
                    # Only full-file checkpoint commits are publishable if the
                    # current batch must be rolled back at its time limit.
                    connection.execute("INSERT INTO sources VALUES (?, ?)", (path, digest(data)))
                    for first, last, chunk in chunk_text(text):
                        if time.monotonic() >= deadline:
                            raise IndexBudgetExpired()
                        tokens = " ".join(terms(path + "\n" + chunk))
                        if not tokens:
                            continue
                        file_chunks += 1
                        rowid = chunks + file_chunks
                        connection.execute("INSERT INTO chunks VALUES (?, ?, ?, ?, ?)", (rowid, path, first, last, chunk))
                        connection.execute("INSERT INTO search(rowid, tokens) VALUES (?, ?)", (rowid, tokens))
                    if time.monotonic() >= deadline:
                        raise IndexBudgetExpired()
                except (IndexBudgetExpired, sqlite3.Error) as error:
                    connection.set_progress_handler(None, 0)
                    # SQLITE_INTERRUPT can roll back the entire in-flight
                    # transaction. Never report its uncommitted full files.
                    connection.rollback()
                    if isinstance(error, IndexBudgetExpired) or (str(error) == "interrupted" and time.monotonic() >= deadline):
                        count, size, chunks, oversized_lines = checkpoint
                        stop_reason = "time_limit"
                        break
                    raise  # Genuine DB errors retain the old published index.
                finally:
                    connection.set_progress_handler(progress, 10000)
                count += 1
                size += len(data)
                chunks += file_chunks
                oversized_lines += file_oversized
                near_deadline = time.monotonic() >= deadline - INDEX_FINAL_CHECKPOINT_MARGIN
                if count == 1 or count - checkpoint[0] >= INDEX_CHECKPOINT_FILES or (near_deadline and not final_checkpointed):
                    connection.set_progress_handler(None, 0)
                    connection.commit()
                    checkpoint = (count, size, chunks, oversized_lines)
                    final_checkpointed = final_checkpointed or near_deadline
                    connection.execute("BEGIN")
                    connection.set_progress_handler(progress, 10000)
            connection.set_progress_handler(None, 0)
            metadata = {
                "format_version": FORMAT_VERSION, "revision": revision,
                "indexed_at_unix": int(time.time()), "eligible_files": len(paths),
                "indexed_files": count, "indexed_bytes": size, "chunks": chunks,
                "excluded_entries": excluded, "skipped_files": skipped,
                "oversized_lines_omitted": oversized_lines,
                "unprocessed_files": len(paths) - count - skipped,
                "stop_reason": stop_reason,
            }
            connection.execute("INSERT INTO metadata VALUES (?)", (json.dumps(metadata),))
            connection.commit()
            connection.close()
            connection = None
            with open(temporary, "rb") as output:
                os.fsync(output.fileno())
            os.replace(temporary, self.database)
            return {"indexed": True, "coverage": metadata}
        except (sqlite3.Error, OSError):
            raise KnowledgeError("Index rebuild failed; any previous complete index is retained.") from None
        finally:
            if connection is not None:
                connection.close()
            for suffix in ("", "-journal"):
                try:
                    os.unlink(temporary + suffix)
                except FileNotFoundError:
                    pass

    def search(self, query, limit=5):
        if not isinstance(query, str) or not 1 <= len(query) <= 512 or type(limit) is not int or not 1 <= limit <= MAX_RESULTS:
            raise KnowledgeError("Search requires a 1–512 character query and a result limit from 1 to 20.")
        query_terms = list(dict.fromkeys(terms(query)))[:64]
        if not query_terms:
            raise KnowledgeError("Search query has no supported identifier or CJK terms.")
        connection, coverage = self.connect()
        deadline = time.monotonic() + 5
        try:
            tracked = set(self.tracked(deadline)[0])
            revision = self.revision(deadline)
            connection.set_progress_handler(lambda: int(time.monotonic() >= deadline), 10000)
            match = " OR ".join('"' + term.replace('"', '""') + '"' for term in query_terms)
            cursor = connection.execute(
                "SELECT c.path, c.start_line, c.end_line, s.source_hash, bm25(search) AS rank FROM search JOIN chunks c ON c.id = search.rowid JOIN sources s ON s.path = c.path WHERE search MATCH ? ORDER BY rank, c.path, c.start_line",
                (match,),
            )
            result = {"query": query, "hits": [], "stale_candidates_omitted": 0, "results_truncated": False, "coverage": coverage}
            verified = {}
            scanned = 0
            while scanned < MAX_FILES * 4:
                row = cursor.fetchone()
                if row is None:
                    break
                scanned += 1
                if time.monotonic() >= deadline:
                    result["results_truncated"] = True
                    break
                path = row["path"]
                if path not in verified:
                    try:
                        data, text = self.source(path, deadline) if path in tracked else (b"", "")
                        verified[path] = text if path in tracked and digest(data) == row["source_hash"] else None
                    except KnowledgeError:
                        verified[path] = None
                if verified[path] is None:
                    result["stale_candidates_omitted"] += 1
                    continue
                # SQLite affinity does not guarantee integer coordinates. A
                # corrupt hint must not fabricate provenance or stop the provider.
                lines = verified[path].splitlines(keepends=True)
                first, last = row["start_line"], row["end_line"]
                if (type(first) is not int or type(last) is not int
                        or not 1 <= first <= last <= len(lines)
                        or last - first + 1 > CHUNK_LINES):
                    raise KnowledgeError("Derived index has invalid source ranges; rebuild the index.")
                excerpt = "".join(lines[first - 1:last])
                if len(excerpt.encode("utf-8")) > CHUNK_BYTES:
                    raise KnowledgeError("Derived index has invalid source ranges; rebuild the index.")
                # Return only verified current bytes, never the cached text column.
                hit = {key: row[key] for key in ("path", "start_line", "end_line", "source_hash")}
                hit["text"] = excerpt
                hit.update(index_revision=coverage["revision"], observed_revision=revision)
                result["hits"].append(hit)
                if len(result["hits"]) > limit or encoded_size(result) > MAX_RESULT_BYTES - 512:
                    result["hits"].pop()
                    result["results_truncated"] = True
                    break
            else:
                result["results_truncated"] = cursor.fetchone() is not None
            return result
        except sqlite3.Error:
            raise KnowledgeError("Derived search index is unavailable; rebuild the index.") from None
        finally:
            connection.close()

    def read(self, path, source_hash, start_line=1, end_line=None):
        self.require_initialized()
        if not eligible_path(path) or not isinstance(source_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", source_hash):
            raise KnowledgeError("Read requires an eligible relative path and its search source_hash.")
        if type(start_line) is not int or start_line < 1 or (end_line is not None and (type(end_line) is not int or end_line < start_line)):
            raise KnowledgeError("Read requires a positive ordered line range.")
        deadline = time.monotonic() + 5
        if path not in set(self.tracked(deadline)[0]):
            raise KnowledgeError("Source is no longer an eligible tracked file; rebuild and search again.")
        data, text = self.source(path, deadline)
        if digest(data) != source_hash:
            raise KnowledgeError("Source hash changed; rebuild and search again before reading.")
        lines = text.splitlines(keepends=True)
        if start_line > len(lines):
            raise KnowledgeError("Requested line range is outside this source.")
        last = min(end_line if end_line is not None else start_line + CHUNK_LINES - 1, len(lines), start_line + 199)
        result = {"path": path, "source_hash": source_hash, "observed_revision": self.revision(deadline), "start_line": start_line, "end_line": last, "text": "".join(lines[start_line - 1:last]), "truncated": end_line is not None and last < end_line}
        while encoded_size(result) > MAX_RESULT_BYTES - 512 and result["end_line"] > start_line:
            result["end_line"] -= 1
            result["text"] = "".join(lines[start_line - 1:result["end_line"]])
            result["truncated"] = True
        if encoded_size(result) > MAX_RESULT_BYTES - 512:
            raise KnowledgeError("One source line exceeds the result bound; narrow the source locally.")
        return result


def encoded_size(value):
    return len(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def chunk_text(text):
    # Oversized individual lines are omitted rather than silently cutting a line
    # while claiming complete line provenance. read reports the same condition.
    current = []
    size = 0
    first = 1
    for number, line in enumerate(text.splitlines(keepends=True), 1):
        line_size = len(line.encode("utf-8"))
        if current and (len(current) >= CHUNK_LINES or size + line_size > CHUNK_BYTES):
            yield first, number - 1, "".join(current)
            current, size = [], 0
        if line_size > CHUNK_BYTES:
            first = number + 1
            continue
        if not current:
            first = number
        current.append(line)
        size += line_size
    if current:
        yield first, first + len(current) - 1, "".join(current)
