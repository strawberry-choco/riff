//! OS system notifications for finished background work (macOS/Windows only).
//!
//! The frame decides *what* is newsworthy — a settled Library Scan, a settled
//! `ReplayGain` Pass, a playback failure — and hands those decisions over as
//! [`NoticePayload`]s on [`FrameOutput`](super::frame::FrameOutput). This
//! module is the enactment half: it turns one payload into one OS
//! notification, or nothing at all on Linux, which has no notifier
//! integration here (the same platform split the tray takes). Failures are
//! warned and swallowed: a toast the OS refuses is an inconvenience, not a
//! condition the app should surface twice — the status line already carries
//! the same text in-app.

use riff_backend::app::events::{NoticePayload, NoticeSource};

/// Raise `notice` as an OS notification, dropping it when it is identical to
/// the immediately preceding one (a failing autoplay queue would otherwise
/// walk the listener's screen with one toast per track).
#[cfg(target_os = "linux")]
pub fn show(_notice: &NoticePayload) {}

#[cfg(not(target_os = "linux"))]
pub fn show(notice: &NoticePayload) {
    use std::sync::Mutex;
    static LAST_SHOWN: Mutex<Option<(NoticeSource, String)>> = Mutex::new(None);
    let mut last = match LAST_SHOWN.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !supersedes_last_shown(&mut last, &notice.source, &notice.message) {
        return;
    }
    let result = notify_rust::Notification::new()
        .summary(title_for(&notice.source))
        .body(&notice.message)
        .show();
    if let Err(e) = result {
        tracing::warn!("Failed to show a system notification: {e}");
    }
}

/// The registry from a notice's source to its notification title. One match,
/// so a new source has exactly one place to gain its user-facing word.
#[cfg(not(target_os = "linux"))]
fn title_for(source: &NoticeSource) -> &'static str {
    match source {
        NoticeSource::Playback => "Playback",
        NoticeSource::Scan => "Library scan",
        NoticeSource::ReplayGain => "ReplayGain",
        NoticeSource::Library => "Library",
        NoticeSource::TagEdit => "Tag editor",
        NoticeSource::Settings => "Settings",
        NoticeSource::System => "riff",
    }
}

/// The dedupe decision, pure so it is assertable with no OS involved: a
/// notice that repeats the immediately preceding one is swallowed; anything
/// else becomes the new preceding one.
fn supersedes_last_shown(
    last: &mut Option<(NoticeSource, String)>,
    source: &NoticeSource,
    message: &str,
) -> bool {
    let same = last.as_ref().is_some_and(|(last_source, last_message)| {
        last_source == source && last_message == message
    });
    if same {
        false
    } else {
        *last = Some((source.clone(), message.to_string()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::supersedes_last_shown;
    use riff_backend::app::events::NoticeSource;

    #[test]
    fn a_repeat_of_the_last_notice_is_swallowed() {
        let mut last = None;
        assert!(supersedes_last_shown(
            &mut last,
            &NoticeSource::Scan,
            "Scan complete: 42 tracks"
        ));
        assert!(
            !supersedes_last_shown(&mut last, &NoticeSource::Scan, "Scan complete: 42 tracks"),
            "the identical notice immediately following is dropped"
        );
    }

    #[test]
    fn a_changed_message_or_source_shows_again() {
        let mut last = None;
        assert!(supersedes_last_shown(
            &mut last,
            &NoticeSource::Scan,
            "Scan complete: 42 tracks"
        ));
        assert!(
            supersedes_last_shown(&mut last, &NoticeSource::Scan, "Scan complete: 43 tracks"),
            "a different message is new"
        );
        assert!(
            supersedes_last_shown(
                &mut last,
                &NoticeSource::Playback,
                "Scan complete: 43 tracks"
            ),
            "a different source is new even with the same message"
        );
        assert!(
            supersedes_last_shown(&mut last, &NoticeSource::Scan, "Scan complete: 43 tracks"),
            "a notice seen before is new again once it no longer immediately repeats"
        );
    }
}
