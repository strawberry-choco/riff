//! OS notifications for background work: the egui
//! `ui/notifications.rs` behavior, through nsh's notification service.
//!
//! The notice log is the source: the pump appends every typed notice, and
//! each new entry raises one OS notification while the persisted "System
//! notifications" flag is on. A silent notification is used so a finished
//! scan does not interrupt.
//!
//! `show_notification` is a no-op where the platform cannot show one, so no
//! OS check is needed.

use nsh_core::notifications::Notification;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;

/// Watch the notice log and raise one OS notification per new entry.
pub fn install(model: Stored<UiModel>, state: UiState) {
    let seen = stored(0_usize);
    effect(move || {
        let notices = state.notices.get();
        let count = notices.len();
        if seen.replace(count) >= count {
            return;
        }
        let enabled = model.with(|m| {
            m.rt.library
                .lock_or_recover()
                .ui_flags
                .system_notifications
        });
        if !enabled {
            return;
        }
        let Some(notice) = notices.last() else {
            return;
        };
        let _ = show_notification(
            Notification::new(
                format!("riff-{count}"),
                crate::APP_NAME,
                notice.message.clone(),
            )
            .silent(true),
        );
    });
}
