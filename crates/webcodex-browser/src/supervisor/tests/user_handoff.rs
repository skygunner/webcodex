use super::*;

fn managed() -> (BrowserSupervisor, String, String) {
    let supervisor = fixture();
    let browser = supervisor.launch().unwrap().browser_id;
    let mut backend = FakeBackend::new();
    backend.managed = true;
    supervisor
        .state()
        .browsers
        .get_mut(&browser)
        .unwrap()
        .backend = Box::new(backend);
    let page = supervisor.pages(&browser, 1).unwrap()[0].page_id.clone();
    (supervisor, browser, page)
}

fn begin(supervisor: &BrowserSupervisor, browser: &str) -> serde_json::Value {
    supervisor
        .begin_user_handoff(
            browser,
            "session_owner",
            "message_question",
            |hint, _| {
                assert_eq!(hint.process_id, 42);
                Ok("surface_exact".into())
            },
            Box::new(|_| {}),
        )
        .unwrap()
}

fn finish(supervisor: &BrowserSupervisor, browser: &str, fence: &str) -> BrowserResult<()> {
    supervisor.finish_user_handoff(browser, "session_owner", fence, |_| Ok(()))
}

#[test]
fn handoff_blocks_domain_effects_and_requires_new_snapshot_after_exact_finish() {
    let (supervisor, browser, page) = managed();
    let snapshot = supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    let element = snapshot
        .nodes
        .iter()
        .find_map(|n| n.element_id.clone())
        .unwrap();
    let hold = begin(&supervisor, &browser);
    assert_eq!(
        supervisor
            .navigate(&browser, &page, "https://example.com")
            .unwrap_err()
            .kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor.reload(&browser, &page).unwrap_err().kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor.new_page(&browser).unwrap_err().kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor
            .click(&browser, &page, &element)
            .unwrap_err()
            .kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor
            .key(&browser, &page, BrowserKey::Enter)
            .unwrap_err()
            .kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor
            .batch(
                &browser,
                &page,
                &[BatchOperation::Click {
                    element_id: element.clone()
                }]
            )
            .unwrap_err()
            .kind,
        "browser_user_handoff_active"
    );
    supervisor.pages(&browser, 1).unwrap();
    supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    assert_eq!(
        supervisor.new_page(&browser).unwrap_err().kind,
        "browser_user_handoff_active"
    );
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        supervisor
            .navigate(&browser, &page, "https://example.com")
            .unwrap_err()
            .kind,
        "browser_snapshot_required"
    );
    assert_eq!(
        supervisor
            .click(&browser, &page, &element)
            .unwrap_err()
            .kind,
        "browser_snapshot_required"
    );
    supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    assert_eq!(
        supervisor
            .click(&browser, &page, &element)
            .unwrap_err()
            .kind,
        "stale_element"
    );
    supervisor
        .navigate(&browser, &page, "https://example.com")
        .unwrap();
}

#[test]
fn handoff_fences_page_close_until_fresh_snapshot_without_blocking_browser_close() {
    let (supervisor, browser, page) = managed();
    let hold = begin(&supervisor, &browser);
    // Closing the page is a Browser effect, not the explicit whole-Browser shutdown.
    assert_eq!(
        supervisor.close_page(&browser, &page).unwrap_err().kind,
        "browser_user_handoff_active"
    );
    assert_eq!(supervisor.pages(&browser, 1).unwrap().len(), 1);
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        supervisor.close_page(&browser, &page).unwrap_err().kind,
        "browser_snapshot_required"
    );
    supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    supervisor.close_page(&browser, &page).unwrap();
    assert!(supervisor.pages(&browser, 1).unwrap().is_empty());
    supervisor.close_browser(&browser).unwrap();
}

#[test]
fn handoff_exact_duplicates_never_renew_and_wrong_identity_never_releases() {
    let (supervisor, browser, _) = managed();
    let hold = begin(&supervisor, &browser);
    let deadline = supervisor.state().browsers[&browser]
        .user_handoff
        .as_ref()
        .unwrap()
        .deadline;
    let duplicate = supervisor
        .begin_user_handoff(
            &browser,
            "session_owner",
            "message_question",
            |_, _| panic!("duplicate must not touch native gate"),
            Box::new(|_| {}),
        )
        .unwrap();
    assert_eq!(hold["state_changed"], true);
    assert_eq!(duplicate["state_changed"], false);
    assert_eq!(hold["handoff_fence"], duplicate["handoff_fence"]);
    assert_eq!(
        supervisor.state().browsers[&browser]
            .user_handoff
            .as_ref()
            .unwrap()
            .deadline,
        deadline
    );
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "other_owner",
                "message_question",
                |_, _| panic!(),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "browser_user_handoff_conflict"
    );
    let fence = hold["handoff_fence"].as_str().unwrap();
    assert_eq!(
        supervisor
            .finish_user_handoff(&browser, "other_owner", fence, |_| panic!())
            .unwrap_err()
            .kind,
        "stale_user_handoff"
    );
    assert_eq!(
        finish(&supervisor, &browser, "wrong_fence")
            .unwrap_err()
            .kind,
        "stale_user_handoff"
    );
    let other = supervisor.launch().unwrap().browser_id;
    assert_eq!(
        finish(&supervisor, &other, fence).unwrap_err().kind,
        "stale_user_handoff"
    );
    finish(&supervisor, &browser, fence).unwrap();
    assert_eq!(
        finish(&supervisor, &browser, fence).unwrap_err().kind,
        "stale_user_handoff"
    );
}

#[test]
fn handoff_deadline_does_not_unpause_surviving_browser_and_caps_lifetime() {
    let (supervisor, browser, page) = managed();
    supervisor
        .state()
        .browsers
        .get_mut(&browser)
        .unwrap()
        .created_at = Instant::now() - MAX_BROWSER_LIFETIME + Duration::from_secs(30);
    let hold = begin(&supervisor, &browser);
    assert!(hold["expires_in_ms"].as_u64().unwrap() <= 30_000);
    supervisor
        .state()
        .browsers
        .get_mut(&browser)
        .unwrap()
        .user_handoff
        .as_mut()
        .unwrap()
        .deadline = Instant::now() - Duration::from_secs(1);
    assert_eq!(
        supervisor.reload(&browser, &page).unwrap_err().kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor.list_browsers()[0]
            .user_handoff
            .as_ref()
            .unwrap()
            .expires_in_ms,
        0
    );
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
}

#[test]
fn handoff_idle_reaper_protection_and_close_revoke_exact_resource() {
    let (supervisor, browser, _) = managed();
    let cancellations = Arc::new(AtomicUsize::new(0));
    let counter = cancellations.clone();
    let hold = supervisor
        .begin_user_handoff(
            &browser,
            "session_owner",
            "message_question",
            |_, _| Ok("surface_exact".into()),
            Box::new(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    supervisor
        .state()
        .browsers
        .get_mut(&browser)
        .unwrap()
        .last_activity_at = Instant::now() - BROWSER_IDLE_TIMEOUT;
    lifecycle::check(&supervisor);
    assert!(supervisor.state().browsers.contains_key(&browser));
    supervisor.close_browser(&browser).unwrap();
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
    assert_eq!(
        finish(
            &supervisor,
            &browser,
            hold["handoff_fence"].as_str().unwrap()
        )
        .unwrap_err()
        .kind,
        "stale_browser"
    );
    let replacement = supervisor.launch().unwrap().browser_id;
    assert_ne!(replacement, browser);
    assert_eq!(
        finish(
            &supervisor,
            &replacement,
            hold["handoff_fence"].as_str().unwrap()
        )
        .unwrap_err()
        .kind,
        "stale_user_handoff"
    );
}

#[test]
fn handoff_rejects_unsupported_ownership_and_failed_exact_surface_without_hold() {
    let supervisor = fixture();
    let browser = supervisor.launch().unwrap().browser_id;
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "message_question",
                |_, _| panic!(),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "browser_user_handoff_unavailable"
    );
    let (supervisor, browser, _) = managed();
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "message_question",
                |_, _| Err(BrowserError::not_started(
                    "browser_surface_unavailable",
                    "ambiguous fixture"
                )),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "browser_surface_unavailable"
    );
    assert!(supervisor.state().browsers[&browser].user_handoff.is_none());
    supervisor.new_page(&browser).unwrap();
}

#[test]
fn handoff_fresh_snapshot_is_required_for_each_original_page() {
    let (supervisor, browser, page) = managed();
    let second = supervisor.new_page(&browser).unwrap().page_id;
    let hold = begin(&supervisor, &browser);
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
    supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    assert_eq!(
        supervisor.reload(&browser, &second).unwrap_err().kind,
        "browser_snapshot_required"
    );
    supervisor
        .snapshot(&browser, &second, SnapshotMode::Full, 16, 8)
        .unwrap();
    supervisor.reload(&browser, &second).unwrap();
}

#[test]
fn handoff_expiry_reaper_and_shutdown_revoke_native_gate_once() {
    for shutdown in [false, true] {
        let (supervisor, browser, _) = managed();
        let cancellations = Arc::new(AtomicUsize::new(0));
        let counter = cancellations.clone();
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "message_question",
                |_, _| Ok("surface_exact".into()),
                Box::new(move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        if shutdown {
            supervisor.shutdown_until(Instant::now() + Duration::from_secs(2));
        } else {
            let mut state = supervisor.state();
            let runtime = state.browsers.get_mut(&browser).unwrap();
            runtime.last_activity_at = Instant::now() - BROWSER_IDLE_TIMEOUT;
            runtime.user_handoff.as_mut().unwrap().deadline =
                Instant::now() - Duration::from_secs(1);
            drop(state);
            lifecycle::check(&supervisor);
        }
        assert!(supervisor.state().browsers.is_empty());
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn handoff_native_finish_failure_preserves_original_block_and_fence() {
    let (supervisor, browser, page) = managed();
    let hold = begin(&supervisor, &browser);
    let fence = hold["handoff_fence"].as_str().unwrap();
    assert_eq!(
        supervisor
            .finish_user_handoff(&browser, "session_owner", fence, |_| Err(
                BrowserError::not_started("fixture_native_failure", "Original reservation stays")
            ))
            .unwrap_err()
            .kind,
        "fixture_native_failure"
    );
    assert_eq!(
        supervisor.reload(&browser, &page).unwrap_err().kind,
        "browser_user_handoff_active"
    );
    assert_eq!(
        supervisor.list_browsers()[0]
            .user_handoff
            .as_ref()
            .unwrap()
            .handoff_fence,
        fence
    );
    finish(&supervisor, &browser, fence).unwrap();
}

#[test]
fn handoff_keeps_native_input_reserved_between_finish_and_fresh_snapshot() {
    let (supervisor, browser, page) = managed();
    let cancellations = Arc::new(AtomicUsize::new(0));
    let counter = cancellations.clone();
    let hold = supervisor
        .begin_user_handoff(
            &browser,
            "session_owner",
            "message_question",
            |_, _| Ok("surface_exact".into()),
            Box::new(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(cancellations.load(Ordering::SeqCst), 0);
    let summary = supervisor.list_browsers().pop().unwrap();
    assert!(summary.user_handoff.is_none());
    assert!(summary.needs_snapshot);
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "new_question",
                |_, _| panic!(),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "browser_snapshot_required"
    );
    supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
    supervisor.close_browser(&browser).unwrap();
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
}

#[test]
fn handoff_close_before_recovery_snapshot_releases_original_native_reservation() {
    let (supervisor, browser, _) = managed();
    let cancellations = Arc::new(AtomicUsize::new(0));
    let counter = cancellations.clone();
    let hold = supervisor
        .begin_user_handoff(
            &browser,
            "session_owner",
            "message_question",
            |_, _| Ok("surface_exact".into()),
            Box::new(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(cancellations.load(Ordering::SeqCst), 0);
    supervisor.close_browser(&browser).unwrap();
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
}

#[test]
fn handoff_terminal_begin_replay_never_rearms_after_finish_and_snapshot() {
    let (supervisor, browser, page) = managed();
    let hold = begin(&supervisor, &browser);
    finish(
        &supervisor,
        &browser,
        hold["handoff_fence"].as_str().unwrap(),
    )
    .unwrap();
    supervisor
        .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
        .unwrap();
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "message_question",
                |_, _| panic!("terminal begin cannot acquire a new native reservation"),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "stale_user_handoff"
    );
    assert!(supervisor.state().browsers[&browser].user_handoff.is_none());
    let next = supervisor
        .begin_user_handoff(
            &browser,
            "session_owner",
            "message_new_step",
            |_, _| Ok("surface_exact".into()),
            Box::new(|_| {}),
        )
        .unwrap();
    assert_ne!(next["handoff_fence"], hold["handoff_fence"]);
    assert_eq!(
        finish(
            &supervisor,
            &browser,
            hold["handoff_fence"].as_str().unwrap()
        )
        .unwrap_err()
        .kind,
        "stale_user_handoff"
    );
}

#[test]
fn handoff_terminal_associations_are_bounded_without_evicting_replay_fences() {
    let (supervisor, browser, page) = managed();
    for index in 0..super::super::user_handoff::MAX_HANDOFF_ASSOCIATIONS {
        let message = format!("message_{index}");
        let hold = supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                &message,
                |_, _| Ok("surface_exact".into()),
                Box::new(|_| {}),
            )
            .unwrap();
        finish(
            &supervisor,
            &browser,
            hold["handoff_fence"].as_str().unwrap(),
        )
        .unwrap();
        supervisor
            .snapshot(&browser, &page, SnapshotMode::Full, 16, 8)
            .unwrap();
    }
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "message_new_step",
                |_, _| panic!(),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "browser_user_handoff_limit"
    );
    assert_eq!(
        supervisor
            .begin_user_handoff(
                &browser,
                "session_owner",
                "message_0",
                |_, _| panic!(),
                Box::new(|_| {})
            )
            .unwrap_err()
            .kind,
        "stale_user_handoff"
    );
}

#[test]
fn handoff_runner_restart_discards_resource_and_never_adopts_old_identity() {
    let (supervisor, browser, _) = managed();
    let cancellations = Arc::new(AtomicUsize::new(0));
    let counter = cancellations.clone();
    let hold = supervisor
        .begin_user_handoff(
            &browser,
            "session_owner",
            "message_question",
            |_, _| Ok("surface_exact".into()),
            Box::new(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    drop(supervisor);
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
    let (replacement, new_browser, _) = managed();
    assert_ne!(new_browser, browser);
    assert_eq!(
        finish(
            &replacement,
            &browser,
            hold["handoff_fence"].as_str().unwrap()
        )
        .unwrap_err()
        .kind,
        "stale_browser"
    );
    assert_eq!(
        finish(
            &replacement,
            &new_browser,
            hold["handoff_fence"].as_str().unwrap()
        )
        .unwrap_err()
        .kind,
        "stale_user_handoff"
    );
    assert!(replacement.list_browsers()[0].user_handoff.is_none());
}
