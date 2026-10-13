//! Per-Job exclusive report ownership. Never consult project-local report files
//! or stdout; retaining only aggregate evidence keeps recovery bounded/private.
use std::io::Read;
use std::path::PathBuf;
use webcodex_core::runner_protocol::ShellJobTestCountEvidence;
use webcodex_core::validation_evidence::CargoTestCountEvidenceStatus;

pub(super) const REPORT_MAX_BYTES: u64 = 1024 * 1024;
pub(super) struct NodeReport {
    directory: tempfile::TempDir,
    path: PathBuf,
}
impl NodeReport {
    pub(super) fn new() -> std::io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("webcodex-node-report-")
            .tempdir()?;
        let path = directory.path().join("results.json");
        // The framework must create the file in this new private directory.
        // An earlier Job's report can never occupy this path.
        Ok(Self { directory, path })
    }
    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub(super) fn evidence(
        &self,
        status: &str,
        exit_code: Option<i32>,
    ) -> ShellJobTestCountEvidence {
        let summary = if matches!(
            (status, exit_code),
            ("completed", Some(0)) | ("failed", Some(1))
        ) {
            self.read()
                .and_then(|bytes| parse_report(&bytes, exit_code == Some(0)))
        } else {
            None
        };
        match summary {
            Some((passed, failed)) => ShellJobTestCountEvidence {
                tests_detected: true,
                tests_run_count: passed.checked_add(failed),
                tests_passed: Some(passed),
                tests_failed: Some(failed),
                status: CargoTestCountEvidenceStatus::CompleteSummary,
            },
            None => ShellJobTestCountEvidence {
                tests_detected: false,
                tests_run_count: None,
                tests_passed: None,
                tests_failed: None,
                status: CargoTestCountEvidenceStatus::NoCompleteSummary,
            },
        }
    }
    fn read(&self) -> Option<Vec<u8>> {
        let metadata = std::fs::symlink_metadata(&self.path).ok()?;
        if !metadata.file_type().is_file() || metadata.len() > REPORT_MAX_BYTES {
            return None;
        }
        let canonical = self.path.canonicalize().ok()?;
        if canonical.parent()? != self.directory.path().canonicalize().ok()? {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return None;
            }
        }
        // Reuse the Runner's pinned-directory, no-follow/nonblocking reader.
        let file = crate::webcodex_runner::file_access::open_regular_file(&canonical).ok()?;
        if crate::webcodex_runner::file_access::file_link_count(&file).ok()? != 1 {
            return None;
        }
        let opened = file.metadata().ok()?;
        if !opened.is_file() || opened.len() > REPORT_MAX_BYTES {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if opened.dev() != metadata.dev()
                || opened.ino() != metadata.ino()
                || opened.nlink() != 1
            {
                return None;
            }
        }
        let mut bytes = Vec::new();
        file.take(REPORT_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() as u64 <= REPORT_MAX_BYTES).then_some(bytes)
    }
}
fn parse_report(bytes: &[u8], process_passed: bool) -> Option<(u64, u64)> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    let count = |key| object.get(key)?.as_u64().filter(|n| *n <= 1_000_000);
    let passed = count("numPassedTests")?;
    let failed = count("numFailedTests")?;
    let pending = count("numPendingTests")?;
    let todo = count("numTodoTests")?;
    if object.get("success")?.as_bool()? != process_passed
        || count("numTotalTests")?
            != passed
                .checked_add(failed)?
                .checked_add(pending)?
                .checked_add(todo)?
        || (process_passed && failed != 0)
    {
        return None;
    }
    let mut observed = [0u64; 4];
    for suite in object.get("testResults")?.as_array()? {
        for assertion in suite.get("assertionResults")?.as_array()? {
            let index = match assertion.get("status")?.as_str()? {
                "passed" => 0,
                "failed" => 1,
                "pending" | "skipped" | "disabled" => 2,
                "todo" => 3,
                _ => return None,
            };
            observed[index] = observed[index].checked_add(1)?;
        }
    }
    if observed != [passed, failed, pending, todo] {
        return None;
    }
    Some((passed, failed))
}

#[cfg(test)]
mod tests;
