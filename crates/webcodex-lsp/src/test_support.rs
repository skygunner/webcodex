use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct EnvGuard {
    restored: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    pub(crate) fn new() -> Self {
        Self {
            restored: Vec::new(),
        }
    }

    pub(crate) fn set(mut self, name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        self.capture(name);
        std::env::set_var(name, value.as_ref());
        self
    }

    pub(crate) fn remove(mut self, name: &'static str) -> Self {
        self.capture(name);
        std::env::remove_var(name);
        self
    }

    fn capture(&mut self, name: &'static str) {
        self.restored.push((name, std::env::var_os(name)));
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in self.restored.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

struct FakeServerBinary {
    _temp: tempfile::TempDir,
    path: PathBuf,
}

/// Create executable fixtures next to this *running test binary*. This
/// follows Cargo's actual target directory (including CARGO_TARGET_DIR and
/// --target-dir) rather than guessing a path from the source checkout.
/// The directory containing this executable is already known to allow exec,
/// even if system TMPDIR or the source checkout is mounted noexec.
pub(super) fn executable_tempdir() -> tempfile::TempDir {
    let test_binary = env::current_exe().expect("locate executing LSP test binary");
    let executable_dir = test_binary.parent().expect("test binary has a parent directory");
    tempfile::Builder::new()
        .prefix("webcodex-lsp-exec-")
        .tempdir_in(executable_dir)
        .expect("create temporary executable fixture next to the test binary")
}

pub(super) fn fake_server_path() -> &'static Path {
    static BINARY: OnceLock<FakeServerBinary> = OnceLock::new();
    &BINARY
        .get_or_init(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let source = manifest.join("src/fake_server.rs");
            // The fake server is a compiled binary that test processes must
            // execute, so it is built through `executable_tempdir` rather
            // than the system temp directory. The OnceLock keeps one binary
            // per test process; the unique tempdir keeps concurrent runs
            // isolated and cleans up automatically.
            let temp = executable_tempdir();
            let path = temp
                .path()
                .join(format!("webcodex-lsp-fake{}", env::consts::EXE_SUFFIX));
            let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));

            for attempt in 1..=2 {
                let output = Command::new(&rustc)
                    .arg("--edition=2021")
                    .arg("--crate-name=webcodex_lsp_fake")
                    .arg(&source)
                    .arg("-o")
                    .arg(&path)
                    .output()
                    .expect("run rustc for fake LSP server");
                if output.status.success() {
                    // Finalize the local executable before concurrent tests can
                    // launch it. On macOS the linker's signature alone can race
                    // first-execution policy assessment and get a fresh helper
                    // killed before main, masquerading as an LSP startup crash.
                    #[cfg(target_os = "macos")]
                    {
                        let output = Command::new("/usr/bin/codesign")
                            .args(["--force", "--sign", "-"])
                            .arg(&path)
                            .output()
                            .expect("ad-hoc sign fake LSP server");
                        assert!(
                            output.status.success(),
                            "fake LSP server signing failed ({}):\n{}",
                            output.status,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    return FakeServerBinary { _temp: temp, path };
                }
                if attempt == 2 {
                    panic!(
                        "fake LSP server compilation failed after {attempt} attempts ({}):\n{}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
            unreachable!()
        })
        .path
}

pub(super) fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(5)),
        );
    }
}
