import ctypes
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import platform_security

from platform_security import (
    SecurityError,
    atomic_private_write,
    locked_state_directory,
    read_private_file,
    secure_created_path,
)


class FakeWindowsSecurityApi:
    """Exercise real ACL traversal while detecting use-after-free deterministically."""

    def __init__(self, owner_matches=True, fail_allocation_at=None, acl_available=True):
        self.buffers = []
        self.labels = {}
        self.allocations = set()
        self.freed = []
        self.owner_matches = owner_matches
        self.fail_allocation_at = fail_allocation_at
        self.acl_available = acl_available
        self.allocation_calls = 0
        self.owner = self.pointer("current" if owner_matches else "other-owner")
        self.acl = self.pointer("acl")
        self.descriptor = self.pointer("descriptor")
        ace = ctypes.create_string_buffer(16)
        self.buffers.append(ace)
        self.ace = ctypes.addressof(ace)
        ctypes.c_ushort.from_buffer(ace, 2).value = 16
        self.labels[self.ace + 8] = "current"

    def pointer(self, label):
        buffer = ctypes.create_string_buffer(16)
        self.buffers.append(buffer)
        address = ctypes.addressof(buffer)
        self.labels[address] = label
        return address

    def sid(self, label):
        self.allocation_calls += 1
        if self.allocation_calls == self.fail_allocation_at:
            raise SecurityError("windows_security_unavailable")
        address = self.pointer(label)
        self.allocations.add(address)
        return ctypes.c_void_p(address), self

    def GetNamedSecurityInfoW(self, name, kind, flags, owner, group, acl, sacl, descriptor):
        owner._obj.value = self.owner
        acl._obj.value = self.acl
        descriptor._obj.value = self.descriptor
        return 0

    def GetAclInformation(self, acl, info, size, kind):
        info._obj.AceCount = 1
        return self.acl_available

    def GetAce(self, acl, index, ace):
        ace._obj.value = self.ace
        return True

    def IsValidSid(self, sid):
        return sid.value in self.labels

    def EqualSid(self, left, right):
        for sid in (left, right):
            if sid.value in self.freed:
                raise AssertionError("SID freed before ACL comparison")
        return self.labels[left.value] == self.labels[right.value]

    def LocalFree(self, pointer):
        if pointer.value in self.freed:
            raise AssertionError("allocation freed more than once")
        self.freed.append(pointer.value)

    def check(self):
        from ctypes import wintypes
        with patch.object(platform_security, "_windows_api", return_value=(self, self, wintypes)), \
                patch.object(platform_security, "_windows_current_sid_string", return_value="current"), \
                patch.object(platform_security, "_windows_sid", side_effect=self.sid):
            return platform_security._windows_private_acl(Path("private-fixture"))


class PlatformSecurityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.state = self.root / "state"
        self.state.mkdir()
        secure_created_path(self.state)

    def test_windows_sid_lifetime_and_exact_cleanup_with_fake_api(self):
        for owner_matches, acl_available in [(True, True), (False, True), (True, False)]:
            with self.subTest(owner_matches=owner_matches, acl_available=acl_available):
                api = FakeWindowsSecurityApi(owner_matches=owner_matches, acl_available=acl_available)
                self.assertEqual(api.check(), owner_matches and acl_available)
                self.assertEqual(set(api.freed), api.allocations | {api.descriptor})
                self.assertEqual(len(api.freed), len(api.allocations) + 1)

    def test_windows_sid_allocation_failure_releases_prior_allocations(self):
        for failure_at in (1, 2, 3):
            with self.subTest(failure_at=failure_at):
                api = FakeWindowsSecurityApi(fail_allocation_at=failure_at)
                with self.assertRaisesRegex(SecurityError, "windows_security_unavailable"):
                    api.check()
                self.assertEqual(set(api.freed), api.allocations | {api.descriptor})
                self.assertEqual(len(api.freed), len(api.allocations) + 1)

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
