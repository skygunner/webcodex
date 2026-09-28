import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from platform_security import (
    SecurityError,
    atomic_private_write,
    locked_state_directory,
    read_private_file,
    secure_created_path,
)


class PlatformSecurityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.state = self.root / "state"
        self.state.mkdir()
        secure_created_path(self.state)

    def test_private_file_roundtrip_and_broad_access_rejected(self):
        path = self.root / "secret"
        path.write_bytes(b"secret")
        secure_created_path(path)
        self.assertEqual(
            read_private_file(path, 32, "not_private", "too_large"),
            b"secret",
        )

        if os.name == "nt":
            subprocess.run(
                ["icacls", str(path), "/grant", "*S-1-1-0:(R)"],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        else:
            path.chmod(0o644)

        with self.assertRaisesRegex(SecurityError, "not_private"):
            read_private_file(path, 32, "not_private", "too_large")

    def test_state_directory_broad_access_is_rejected(self):
        if os.name == "nt":
            subprocess.run(
                ["icacls", str(self.state), "/grant", "*S-1-1-0:(OI)(CI)(R)"],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        else:
            self.state.chmod(0o755)
        with self.assertRaisesRegex(SecurityError, "private_state_directory_required"):
            with locked_state_directory(self.state):
                self.fail("broad state directory must not be opened")

    @unittest.skipUnless(os.name == "nt", "Windows ACL contract")
    def test_windows_broad_write_access_is_rejected(self):
        path = self.root / "writeable-secret"
        path.write_bytes(b"secret")
        secure_created_path(path)
        subprocess.run(
            ["icacls", str(path), "/grant", "*S-1-1-0:(W)"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        with self.assertRaisesRegex(SecurityError, "not_private"):
            read_private_file(path, 32, "not_private", "too_large")

        writable_state = self.root / "writeable-state"
        writable_state.mkdir()
        secure_created_path(writable_state)
        subprocess.run(
            ["icacls", str(writable_state), "/grant", "*S-1-1-0:(OI)(CI)(W)"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        with self.assertRaisesRegex(SecurityError, "private_state_directory_required"):
            with locked_state_directory(writable_state):
                self.fail("broadly writable state directory must not be opened")

    @unittest.skipUnless(os.name == "nt", "Windows ACL contract")
    def test_windows_other_principal_access_is_rejected(self):
        path = self.root / "other-principal-secret"
        path.write_bytes(b"secret")
        secure_created_path(path)
        subprocess.run(
            ["icacls", str(path), "/grant", "*S-1-5-32-546:(R)"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        with self.assertRaisesRegex(SecurityError, "not_private"):
            read_private_file(path, 32, "not_private", "too_large")

    def test_state_lock_is_nonblocking_and_json_is_private(self):
        with locked_state_directory(self.state) as state:
            state.create_json("event.json", {"status": "pending"})
            self.assertEqual(state.read_json("event.json", 1024), {"status": "pending"})
            with self.assertRaisesRegex(SecurityError, "adapter_busy_event_not_saved"):
                with locked_state_directory(self.state):
                    self.fail("nested lock must not be acquired")
            state.unlink("event.json")
        self.assertFalse((self.state / "event.json").exists())

    def test_atomic_private_write_replaces_exact_file(self):
        path = self.state / "snapshot.json"
        atomic_private_write(path, b'{"revision":1}\n')
        self.assertEqual(
            json.loads(read_private_file(path, 1024, "not_private", "too_large")),
            {"revision": 1},
        )
        atomic_private_write(path, b'{"revision":2}\n')
        self.assertEqual(
            json.loads(read_private_file(path, 1024, "not_private", "too_large")),
            {"revision": 2},
        )
        self.assertEqual(list(self.state.glob(".recovery-*")), [])

    @unittest.skipUnless(os.name == "nt", "Windows reparse-point contract")
    def test_windows_state_directory_reparse_point_is_rejected(self):
        real = self.root / "real-state"
        real.mkdir()
        secure_created_path(real)
        junction = self.root / "junction-state"
        subprocess.run(
            ["cmd", "/c", "mklink", "/J", str(junction), str(real)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        with self.assertRaisesRegex(SecurityError, "private_state_directory_required"):
            with locked_state_directory(junction):
                self.fail("reparse-point state directory must not be opened")


if __name__ == "__main__":
    unittest.main()
