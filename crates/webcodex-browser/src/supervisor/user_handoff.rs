//! A bounded, process-local, exact Browser hold. Session attention remains external.
use super::*;

const DEFAULT_HOLD: Duration = Duration::from_secs(10 * 60);
pub(super) const MAX_HANDOFF_ASSOCIATIONS: usize = 64;

pub(super) struct UserHandoff {
    owner_session_id: String,
    message_id: String,
    fence: String,
    pub(super) deadline: Instant,
    surface_id: String,
    cancel: Box<dyn Fn(&str) + Send>,
}

impl BrowserSupervisor {
    /// Resolve the exact live surface while holding the Browser operation lease,
    /// then atomically fence automatic effects. An exact duplicate never renews.
    pub fn begin_user_handoff(
        &self,
        browser_id: &str,
        session_id: &str,
        message_id: &str,
        resolve: impl FnOnce(crate::BrowserWindowHint, &str) -> BrowserResult<String>,
        cancel: Box<dyn Fn(&str) + Send>,
    ) -> BrowserResult<serde_json::Value> {
        if session_id.is_empty()
            || session_id.len() > 128
            || message_id.is_empty()
            || message_id.len() > 128
        {
            return Err(BrowserError::not_started(
                "invalid_request",
                "Exact bounded Session and attention identities are required",
            ));
        }
        self.touch_current(browser_id)?;
        let mut state = self.operation_state()?;
        let runtime = state
            .browsers
            .get_mut(browser_id)
            .ok_or_else(|| stale_browser(browser_id))?;
        if runtime.backend.ownership() != crate::BrowserOwnership::OwnedManagedPersistent {
            return Err(BrowserError::not_started(
                "browser_user_handoff_unavailable",
                "User handoff requires an owned visible managed persistent Browser",
            ));
        }
        if let Some(hold) = &runtime.user_handoff {
            if hold.owner_session_id != session_id || hold.message_id != message_id {
                return Err(BrowserError::not_started(
                    "browser_user_handoff_conflict",
                    "This exact Browser already has a different user handoff",
                ));
            }
            return Ok(hold.receipt(browser_id, false));
        }
        if runtime
            .completed_user_handoffs
            .contains(&(session_id.to_owned(), message_id.to_owned()))
        {
            return Err(BrowserError::not_started("stale_user_handoff", "This Browser/Session/attention association already finished; observe recovery, never rearm a delayed begin"));
        }
        if runtime.completed_user_handoffs.len() >= MAX_HANDOFF_ASSOCIATIONS {
            return Err(BrowserError::not_started("browser_user_handoff_limit", "This Browser reached its bounded handoff history; close it before explicitly launching a replacement"));
        }
        runtime.admit_automatic_effect(None)?;
        let fence = opaque_id("handoff");
        let surface_id = resolve(runtime.backend.live_window_hint()?, &fence)?;
        if surface_id.is_empty() || surface_id.len() > 128 {
            cancel(&fence);
            return Err(BrowserError::not_started(
                "browser_surface_unavailable",
                "Exact live Browser surface is unavailable",
            ));
        }
        let now = Instant::now();
        let deadline = (now + DEFAULT_HOLD).min(runtime.created_at + MAX_BROWSER_LIFETIME);
        let hold = UserHandoff {
            owner_session_id: session_id.to_owned(),
            message_id: message_id.to_owned(),
            fence,
            deadline,
            surface_id,
            cancel,
        };
        runtime.elements.clear();
        runtime.needs_handoff_snapshot = true;
        runtime.handoff_snapshot_pages = runtime.pages.keys().cloned().collect();
        let receipt = hold.receipt(browser_id, true);
        runtime.user_handoff = Some(hold);
        Ok(receipt)
    }

    /// Releasing needs the original owner and fence even after the deadline. ACK,
    /// reply, elapsed time and snapshots never release a hold.
    pub fn finish_user_handoff(
        &self,
        browser_id: &str,
        session_id: &str,
        fence: &str,
        finish: impl FnOnce(&str) -> BrowserResult<()>,
    ) -> BrowserResult<()> {
        self.touch_current(browser_id)?;
        let mut state = self.operation_state()?;
        let runtime = state
            .browsers
            .get_mut(browser_id)
            .ok_or_else(|| stale_browser(browser_id))?;
        let hold = runtime.user_handoff.as_ref().ok_or_else(|| {
            BrowserError::not_started(
                "stale_user_handoff",
                "No matching live handoff; observe the Browser and take a fresh snapshot",
            )
        })?;
        if hold.owner_session_id != session_id || hold.fence != fence {
            return Err(BrowserError::not_started(
                "stale_user_handoff",
                "Original Session and exact handoff fence are required",
            ));
        }
        finish(fence)?;
        runtime
            .completed_user_handoffs
            .insert((hold.owner_session_id.clone(), hold.message_id.clone()));
        runtime.pending_handoff_snapshot = runtime.user_handoff.take();
        runtime.elements.clear();
        runtime.needs_handoff_snapshot = true;
        runtime.handoff_snapshot_pages = runtime.pages.keys().cloned().collect();
        Ok(())
    }
}

impl UserHandoff {
    pub(super) fn summary(&self) -> crate::types::UserHandoffSummary {
        crate::types::UserHandoffSummary {
            session_id: self.owner_session_id.clone(),
            message_id: self.message_id.clone(),
            handoff_fence: self.fence.clone(),
            surface_id: self.surface_id.clone(),
            expires_in_ms: self
                .deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64,
        }
    }
    fn receipt(&self, browser_id: &str, state_changed: bool) -> serde_json::Value {
        serde_json::json!({"browser_id": browser_id, "session_id": self.owner_session_id,
            "message_id": self.message_id, "handoff_fence": self.fence, "surface_id": self.surface_id,
            "expires_in_ms": self.deadline.saturating_duration_since(Instant::now()).as_millis() as u64,
            "needs_snapshot": true, "state_changed": state_changed})
    }
}

impl BrowserRuntime {
    pub(super) fn admit_automatic_effect(&self, page_id: Option<&str>) -> BrowserResult<()> {
        if self.user_handoff.is_some() {
            let mut error = BrowserError::not_started("browser_user_handoff_active", "Automatic page effects remain blocked until exact finish_user_handoff; deadline expiry and user replies do not release the hold");
            error.recovery_action = Some("browsers");
            return Err(error);
        }
        if self.needs_handoff_snapshot
            || page_id.is_some_and(|id| self.handoff_snapshot_pages.contains(id))
        {
            let mut error = BrowserError::not_started(
                "browser_snapshot_required",
                "Take a fresh Browser snapshot after finishing the user handoff",
            );
            error.recovery_action = Some("snapshot");
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for UserHandoff {
    fn drop(&mut self) {
        (self.cancel)(&self.fence);
    }
}
