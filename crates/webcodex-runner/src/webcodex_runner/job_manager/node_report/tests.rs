use super::*;
use serde_json::json;
fn report(passed: u64, failed: u64, pending: u64) -> Vec<u8> {
    let statuses = std::iter::repeat_n("passed", passed as usize)
        .chain(std::iter::repeat_n("failed", failed as usize))
        .chain(std::iter::repeat_n("pending", pending as usize))
        .map(|status| json!({"status":status}))
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({"numTotalTests":passed+failed+pending,"numPassedTests":passed,"numFailedTests":failed,"numPendingTests":pending,"numTodoTests":0,"success":failed==0,"testResults":[{"assertionResults":statuses}]})).unwrap()
}
#[test]
fn node_json_report_is_exclusive_bounded_and_cleaned() {
    let old = NodeReport::new().unwrap();
    std::fs::write(old.path(), report(2, 0, 1)).unwrap();
    let fresh = NodeReport::new().unwrap();
    assert_ne!(fresh.path(), old.path());
    assert_eq!(fresh.evidence("completed", Some(0)).tests_run_count, None);
    assert_eq!(old.evidence("completed", Some(0)).tests_run_count, Some(2));
    let path = old.path().to_path_buf();
    drop(old);
    assert!(!path.exists());
    std::fs::write(fresh.path(), vec![b' '; REPORT_MAX_BYTES as usize + 1]).unwrap();
    assert_eq!(fresh.evidence("completed", Some(0)).tests_run_count, None);
    std::fs::write(fresh.path(), &report(1, 0, 0)[..20]).unwrap();
    assert_eq!(fresh.evidence("completed", Some(0)).tests_run_count, None);
}
#[test]
fn node_json_report_counts_require_complete_consistent_outcome() {
    let report_file = NodeReport::new().unwrap();
    for (passed, failed, pending, status, code, expected) in [
        (2, 0, 1, "completed", 0, Some(2)),
        (1, 1, 1, "failed", 1, Some(2)),
        (0, 0, 1, "completed", 0, Some(0)),
        (1, 0, 0, "failed", 1, None),
        (1, 0, 0, "timeout", 0, None),
        (1, 0, 0, "stopped", 0, None),
        (1, 0, 0, "lost", 0, None),
        (1, 1, 0, "completed", 0, None),
    ] {
        std::fs::write(report_file.path(), report(passed, failed, pending)).unwrap();
        let evidence = report_file.evidence(status, Some(code));
        assert!(evidence.is_valid());
        assert_eq!(evidence.tests_run_count, expected, "{status}/{code}");
    }
    let mut value: serde_json::Value = serde_json::from_slice(&report(2, 0, 0)).unwrap();
    value["numPassedTests"] = json!(10);
    assert!(parse_report(&serde_json::to_vec(&value).unwrap(), true).is_none());
    value["numTotalTests"] = json!(10);
    assert!(parse_report(&serde_json::to_vec(&value).unwrap(), true).is_none());
    assert!(parse_report(b"test passed 9999", true).is_none());
}
#[cfg(unix)]
#[test]
fn node_json_report_rejects_links_and_special_files() {
    let old = NodeReport::new().unwrap();
    std::fs::write(old.path(), report(1, 0, 0)).unwrap();
    let new = NodeReport::new().unwrap();
    std::os::unix::fs::symlink(old.path(), new.path()).unwrap();
    assert_eq!(new.evidence("completed", Some(0)).tests_run_count, None);
    std::fs::remove_file(new.path()).unwrap();
    std::fs::hard_link(old.path(), new.path()).unwrap();
    assert_eq!(new.evidence("completed", Some(0)).tests_run_count, None);
    std::fs::remove_file(new.path()).unwrap();
    std::fs::create_dir(new.path()).unwrap();
    assert_eq!(new.evidence("completed", Some(0)).tests_run_count, None);
}
