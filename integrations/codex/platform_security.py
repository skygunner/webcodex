"""Cross-platform private local state primitives for the optional Codex adapter.

Unix preserves the existing owner/mode/no-follow contract. Windows mirrors the
repository's native private-path policy: no reparse points, current-user owner,
and no effective access for Everyone, Authenticated Users, or local Users.
"""

import contextlib
import ctypes
import functools
import json
import os
from pathlib import Path
import stat
import tempfile

if os.name == "nt":
    import msvcrt
else:
    import fcntl


class SecurityError(Exception):
    pass


_REPARSE_POINT = 0x400
_WINDOWS_ALLOWED_SIDS = ("S-1-5-18", "S-1-5-32-544")


def _is_reparse_or_link(path):
    metadata = os.lstat(path)
    return stat.S_ISLNK(metadata.st_mode) or (
        os.name == "nt"
        and getattr(metadata, "st_file_attributes", 0) & _REPARSE_POINT != 0
    )


def _unix_private_file_metadata(metadata):
    return (
        stat.S_ISREG(metadata.st_mode)
        and metadata.st_uid == os.getuid()
        and metadata.st_mode & 0o077 == 0
        and metadata.st_nlink == 1
    )


def _windows_api():
    if os.name != "nt":
        raise RuntimeError("Windows security API requested on non-Windows host")
    from ctypes import wintypes

    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)

    advapi.OpenProcessToken.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.HANDLE),
    ]
    advapi.OpenProcessToken.restype = wintypes.BOOL
    advapi.GetTokenInformation.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi.GetTokenInformation.restype = wintypes.BOOL
    advapi.ConvertSidToStringSidW.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    advapi.ConvertSidToStringSidW.restype = wintypes.BOOL
    advapi.ConvertStringSidToSidW.argtypes = [
        wintypes.LPCWSTR,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    advapi.ConvertStringSidToSidW.restype = wintypes.BOOL
    advapi.GetNamedSecurityInfoW.argtypes = [
        wintypes.LPWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_void_p),
    ]
    advapi.GetNamedSecurityInfoW.restype = wintypes.DWORD
    advapi.GetEffectiveRightsFromAclW.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi.GetEffectiveRightsFromAclW.restype = wintypes.DWORD
    advapi.GetAclInformation.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        wintypes.DWORD,
        wintypes.DWORD,
    ]
    advapi.GetAclInformation.restype = wintypes.BOOL
    advapi.GetAce.argtypes = [
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    advapi.GetAce.restype = wintypes.BOOL
    advapi.IsValidSid.argtypes = [ctypes.c_void_p]
    advapi.IsValidSid.restype = wintypes.BOOL
    advapi.EqualSid.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    advapi.EqualSid.restype = wintypes.BOOL
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.restype = wintypes.BOOL
    advapi.GetSecurityDescriptorDacl.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(wintypes.BOOL),
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(wintypes.BOOL),
    ]
    advapi.GetSecurityDescriptorDacl.restype = wintypes.BOOL
    advapi.GetSecurityDescriptorOwner.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(wintypes.BOOL),
    ]
    advapi.GetSecurityDescriptorOwner.restype = wintypes.BOOL
    advapi.SetNamedSecurityInfoW.argtypes = [
        wintypes.LPWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
    ]
    advapi.SetNamedSecurityInfoW.restype = wintypes.DWORD

    kernel.GetCurrentProcess.argtypes = []
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    kernel.LocalFree.argtypes = [ctypes.c_void_p]
    kernel.LocalFree.restype = ctypes.c_void_p
    kernel.MoveFileExW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD]
    kernel.MoveFileExW.restype = wintypes.BOOL

    return advapi, kernel, wintypes


@functools.lru_cache(maxsize=1)
def _windows_current_sid_string():
    advapi, kernel, wintypes = _windows_api()
    TOKEN_QUERY = 0x0008
    TOKEN_USER = 1

    class SidAndAttributes(ctypes.Structure):
        _fields_ = [("Sid", ctypes.c_void_p), ("Attributes", wintypes.DWORD)]

    class TokenUser(ctypes.Structure):
        _fields_ = [("User", SidAndAttributes)]

    token = wintypes.HANDLE()
    if not advapi.OpenProcessToken(kernel.GetCurrentProcess(), TOKEN_QUERY, ctypes.byref(token)):
        raise SecurityError("windows_security_unavailable")
    try:
        needed = wintypes.DWORD()
        advapi.GetTokenInformation(token, TOKEN_USER, None, 0, ctypes.byref(needed))
        if needed.value == 0:
            raise SecurityError("windows_security_unavailable")
        buffer = ctypes.create_string_buffer(needed.value)
        if not advapi.GetTokenInformation(
            token, TOKEN_USER, buffer, needed.value, ctypes.byref(needed)
        ):
            raise SecurityError("windows_security_unavailable")
        token_user = ctypes.cast(buffer, ctypes.POINTER(TokenUser)).contents
        text = ctypes.c_void_p()
        if not advapi.ConvertSidToStringSidW(token_user.User.Sid, ctypes.byref(text)):
            raise SecurityError("windows_security_unavailable")
        try:
            return ctypes.wstring_at(text)
        finally:
            kernel.LocalFree(text)
    finally:
        kernel.CloseHandle(token)


def _windows_sid(sid_text):
    advapi, kernel, _ = _windows_api()
    sid = ctypes.c_void_p()
    if not advapi.ConvertStringSidToSidW(sid_text, ctypes.byref(sid)):
        raise SecurityError("windows_security_unavailable")
    return sid, kernel


def _windows_private_acl(path):
    advapi, kernel, wintypes = _windows_api()
    OWNER_SECURITY_INFORMATION = 0x00000001
    DACL_SECURITY_INFORMATION = 0x00000004
    SE_FILE_OBJECT = 1
    ACL_SIZE_INFORMATION = 2
    ACCESS_ALLOWED_ACE_TYPE = 0
    ACCESS_DENIED_ACE_TYPE = 1

    class AclSizeInformation(ctypes.Structure):
        _fields_ = [
            ("AceCount", wintypes.DWORD),
            ("AclBytesInUse", wintypes.DWORD),
            ("AclBytesFree", wintypes.DWORD),
        ]

    class AceHeader(ctypes.Structure):
        _fields_ = [
            ("AceType", ctypes.c_ubyte),
            ("AceFlags", ctypes.c_ubyte),
            ("AceSize", ctypes.c_ushort),
        ]

    owner = ctypes.c_void_p()
    acl = ctypes.c_void_p()
    descriptor = ctypes.c_void_p()
    name = ctypes.create_unicode_buffer(str(path))
    result = advapi.GetNamedSecurityInfoW(
        name,
        SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        ctypes.byref(owner),
        None,
        ctypes.byref(acl),
        None,
        ctypes.byref(descriptor),
    )
    if result != 0 or not owner.value or not acl.value:
        if descriptor.value:
            kernel.LocalFree(descriptor)
        return False
    try:
        current_sid, current_kernel = _windows_sid(_windows_current_sid_string())
        try:
            if not advapi.EqualSid(owner, current_sid):
                return False
        finally:
            current_kernel.LocalFree(current_sid)

        allowed = [current_sid]
        allocated = []
        for sid_text in _WINDOWS_ALLOWED_SIDS:
            sid, sid_kernel = _windows_sid(sid_text)
            allowed.append(sid)
            allocated.append((sid, sid_kernel))
        try:
            info = AclSizeInformation()
            if not advapi.GetAclInformation(
                acl,
                ctypes.byref(info),
                ctypes.sizeof(info),
                ACL_SIZE_INFORMATION,
            ):
                return False
            for index in range(info.AceCount):
                ace = ctypes.c_void_p()
                if not advapi.GetAce(acl, index, ctypes.byref(ace)) or not ace.value:
                    return False
                header = ctypes.cast(ace, ctypes.POINTER(AceHeader)).contents
                if header.AceType == ACCESS_DENIED_ACE_TYPE:
                    continue
                if header.AceType != ACCESS_ALLOWED_ACE_TYPE or header.AceSize < 12:
                    return False
                # ACCESS_ALLOWED_ACE is ACE_HEADER + ACCESS_MASK + SID.
                trustee_sid = ctypes.c_void_p(ace.value + 8)
                if not advapi.IsValidSid(trustee_sid):
                    return False
                if not any(advapi.EqualSid(trustee_sid, sid) for sid in allowed):
                    return False
            return True
        finally:
            for sid, sid_kernel in allocated:
                sid_kernel.LocalFree(sid)
    finally:
        kernel.LocalFree(descriptor)


def secure_created_path(path):
    """Make a newly created Windows file/directory private before sensitive writes.

    Unix callers already create with restrictive modes; chmod is retained here so
    tests and temporary paths have the same canonical permissions.
    """
    path = Path(path)
    if os.name != "nt":
        os.chmod(path, 0o700 if path.is_dir() else 0o600)
        return

    advapi, kernel, wintypes = _windows_api()
    SE_FILE_OBJECT = 1
    DACL_SECURITY_INFORMATION = 0x00000004
    OWNER_SECURITY_INFORMATION = 0x00000001
    PROTECTED_DACL_SECURITY_INFORMATION = 0x80000000
    sid = _windows_current_sid_string()
    sddl = (
        f"O:{sid}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{sid})"
    )
    descriptor = ctypes.c_void_p()
    if not advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl, 1, ctypes.byref(descriptor), None
    ):
        raise SecurityError("windows_security_unavailable")
    try:
        present = wintypes.BOOL()
        defaulted = wintypes.BOOL()
        dacl = ctypes.c_void_p()
        owner = ctypes.c_void_p()
        if not advapi.GetSecurityDescriptorDacl(
            descriptor, ctypes.byref(present), ctypes.byref(dacl), ctypes.byref(defaulted)
        ):
            raise SecurityError("windows_security_unavailable")
        if not present.value or not dacl.value:
            raise SecurityError("windows_security_unavailable")
        if not advapi.GetSecurityDescriptorOwner(
            descriptor, ctypes.byref(owner), ctypes.byref(defaulted)
        ):
            raise SecurityError("windows_security_unavailable")
        if not owner.value:
            raise SecurityError("windows_security_unavailable")
        name = ctypes.create_unicode_buffer(str(path))
        result = advapi.SetNamedSecurityInfoW(
            name,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION
            | OWNER_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            None,
            dacl,
            None,
        )
        if result != 0:
            raise SecurityError("windows_security_unavailable")
    finally:
        kernel.LocalFree(descriptor)
    if not _windows_private_acl(path):
        raise SecurityError("windows_private_path_required")


def _validate_private_file(path, invalid_code):
    path = Path(path)
    try:
        metadata = os.lstat(path)
    except OSError:
        raise
    if _is_reparse_or_link(path) or not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
        raise SecurityError(invalid_code)
    if os.name == "nt":
        if not _windows_private_acl(path):
            raise SecurityError(invalid_code)
    elif not _unix_private_file_metadata(metadata):
        raise SecurityError(invalid_code)
    return metadata


def read_private_file(path, max_bytes, invalid_code, oversized_code):
    path = Path(path)
    before = _validate_private_file(path, invalid_code)
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    if os.name != "nt":
        flags |= os.O_NOFOLLOW | os.O_NONBLOCK
    fd = os.open(path, flags)
    with os.fdopen(fd, "rb") as stream:
        after = os.fstat(stream.fileno())
        if not stat.S_ISREG(after.st_mode) or after.st_nlink != 1:
            raise SecurityError(invalid_code)
        if os.name == "nt":
            if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                raise SecurityError(invalid_code)
        elif not _unix_private_file_metadata(after):
            raise SecurityError(invalid_code)
        raw = stream.read(max_bytes + 1)
    if len(raw) > max_bytes:
        raise SecurityError(oversized_code)
    return raw


def _validate_private_directory(path):
    path = Path(path)
    metadata = os.lstat(path)
    if _is_reparse_or_link(path) or not stat.S_ISDIR(metadata.st_mode):
        raise SecurityError("private_state_directory_required")
    if os.name == "nt":
        if not _windows_private_acl(path):
            raise SecurityError("private_state_directory_required")
    elif metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
        raise SecurityError("private_state_directory_required")


class StateDirectory:
    def __init__(self, root, dir_fd=None):
        self.root = Path(root)
        self.dir_fd = dir_fd

    def names(self):
        return os.listdir(self.dir_fd if self.dir_fd is not None else self.root)

    def read_json(self, name, max_bytes):
        if self.dir_fd is not None:
            child = os.open(
                name,
                os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                dir_fd=self.dir_fd,
            )
            with os.fdopen(child, "rb") as stream:
                metadata = os.fstat(stream.fileno())
                if not _unix_private_file_metadata(metadata):
                    raise SecurityError("invalid_pending_file")
                raw = stream.read(max_bytes + 1)
            if len(raw) > max_bytes:
                raise SecurityError("oversized_pending_file")
            return json.loads(raw)

        raw = read_private_file(
            self.root / name,
            max_bytes,
            "invalid_pending_file",
            "oversized_pending_file",
        )
        return json.loads(raw)

    def create_json(self, name, value):
        if self.dir_fd is not None:
            child = os.open(
                name,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                0o600,
                dir_fd=self.dir_fd,
            )
            with os.fdopen(child, "w") as stream:
                json.dump(value, stream, sort_keys=True)
                stream.flush()
                os.fsync(stream.fileno())
            os.fsync(self.dir_fd)
            return

        path = self.root / name
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
        child = os.open(path, flags, 0o600)
        try:
            secure_created_path(path)
            with os.fdopen(child, "w") as stream:
                child = None
                json.dump(value, stream, sort_keys=True)
                stream.flush()
                os.fsync(stream.fileno())
        except Exception:
            if child is not None:
                os.close(child)
            with contextlib.suppress(OSError):
                os.unlink(path)
            raise

    def unlink(self, name):
        if self.dir_fd is not None:
            os.unlink(name, dir_fd=self.dir_fd)
            os.fsync(self.dir_fd)
        else:
            os.unlink(self.root / name)


@contextlib.contextmanager
def locked_state_directory(path):
    root = Path(path)
    _validate_private_directory(root)
    if os.name != "nt":
        directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        lock = None
        try:
            metadata = os.fstat(directory)
            if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
                raise SecurityError("private_state_directory_required")
            lock = os.open(
                ".lock",
                os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW,
                0o600,
                dir_fd=directory,
            )
            lock_metadata = os.fstat(lock)
            if not _unix_private_file_metadata(lock_metadata):
                raise SecurityError("invalid_lock")
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise SecurityError("adapter_busy_event_not_saved") from None
            yield StateDirectory(root, directory)
        finally:
            if lock is not None:
                os.close(lock)
            os.close(directory)
        return

    lock_path = root / ".lock"
    created = False
    try:
        lock = os.open(
            lock_path,
            os.O_RDWR | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0),
            0o600,
        )
        created = True
    except FileExistsError:
        lock = os.open(lock_path, os.O_RDWR | getattr(os, "O_BINARY", 0))
    try:
        if created:
            secure_created_path(lock_path)
        _validate_private_file(lock_path, "invalid_lock")
        if os.fstat(lock).st_size == 0:
            os.write(lock, b"\0")
            os.fsync(lock)
        os.lseek(lock, 0, os.SEEK_SET)
        try:
            msvcrt.locking(lock, msvcrt.LK_NBLCK, 1)
        except OSError:
            raise SecurityError("adapter_busy_event_not_saved") from None
        try:
            yield StateDirectory(root)
        finally:
            os.lseek(lock, 0, os.SEEK_SET)
            with contextlib.suppress(OSError):
                msvcrt.locking(lock, msvcrt.LK_UNLCK, 1)
    finally:
        os.close(lock)


def _windows_replace_write_through(source, target):
    _, kernel, _ = _windows_api()
    MOVEFILE_REPLACE_EXISTING = 0x1
    MOVEFILE_WRITE_THROUGH = 0x8
    if not kernel.MoveFileExW(
        str(source),
        str(target),
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    ):
        raise OSError(ctypes.get_last_error(), "MoveFileExW failed")


def atomic_private_write(path, raw):
    path = Path(path)
    fd, temporary = tempfile.mkstemp(prefix=".recovery-", dir=path.parent)
    temporary = Path(temporary)
    try:
        secure_created_path(temporary)
        with os.fdopen(fd, "wb") as stream:
            fd = None
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        if os.name == "nt":
            _windows_replace_write_through(temporary, path)
        else:
            os.replace(temporary, path)
            directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        if fd is not None:
            os.close(fd)
        if temporary.exists():
            with contextlib.suppress(OSError):
                temporary.unlink()
