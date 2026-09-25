//! Structured feedback carried from the application to the titlebar paint.
//!
//! The backend already stamps every notice with a [`NoticeSource`] and
//! [`NoticeSeverity`] (`riff_backend::app::events`). This module keeps that
//! structure alive across the application→paint boundary instead of flattening
//! every stream into the one `scan_status` string — which let a Library Scan
//! update silently overwrite a live playback error. Each source owns its own
//! persistent slot on the [`FeedbackBoard`], and the titlebar paints the
//! highest-severity active notice, so the streams coexist.
//!
//! The frontend owns this model (it is paint state, not a session fact): the
//! [`RiffApp`] holds one board, the scan / playback / Tag Edit producers write
//! their slot, and the board's display feeds the existing `scan_status`
//! line unchanged — same placement, same copy, no new surface.

use riff_backend::app::events::{NoticePayload, NoticeSeverity, NoticeSource};

/// A recovery affordance a notice can offer. It rides the notice to the paint
/// boundary so a surface may later act on it; the titlebar copy is unchanged
/// today, so recovery is carried and preserved rather than rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Re-run the Library Scan that failed.
    Rescan,
}

/// One structured feedback notice: what happened (`message`), how bad
/// (`severity`), where it came from (`source`), whether it persists, and an
/// optional recovery affordance.
#[derive(Debug, Clone, PartialEq)]
pub struct Feedback {
    pub severity: NoticeSeverity,
    pub source: NoticeSource,
    pub message: String,
    pub recovery: Option<Recovery>,
}

impl Feedback {
    /// Build a notice from a drained backend payload, keeping its severity and
    /// source (the structured facts that must survive the boundary).
    #[must_use]
    pub fn from_notice(payload: &NoticePayload, recovery: Option<Recovery>) -> Self {
        Self {
            severity: payload.severity.clone(),
            source: payload.source.clone(),
            message: payload.message.clone(),
            recovery,
        }
    }
}

/// The per-source feedback slots the titlebar composes. One slot per source so
/// a scan progress line never erases a playback error: they are stored apart and
/// only merged for display.
#[derive(Default)]
pub struct FeedbackBoard {
    scan: Option<Feedback>,
    playback: Option<Feedback>,
    tag_edit: Option<Feedback>,
}

impl FeedbackBoard {
    /// Record one Library Scan notice into the scan slot.
    pub fn set_scan(&mut self, message: impl Into<String>, severity: NoticeSeverity) {
        self.scan = Some(Feedback {
            severity,
            source: NoticeSource::Scan,
            message: message.into(),
            recovery: None,
        });
    }

    /// Record a Library mutation (Clear Library) notice into the scan slot —
    /// the same status line the scan reports through.
    pub fn set_library(&mut self, message: impl Into<String>, severity: NoticeSeverity) {
        self.scan = Some(Feedback {
            severity,
            source: NoticeSource::Library,
            message: message.into(),
            recovery: None,
        });
    }

    /// Record a Tag Edit notice (single success/failure or a partial batch)
    /// into the tag-edit slot.
    pub fn set_tag_edit(&mut self, message: impl Into<String>, severity: NoticeSeverity) {
        self.tag_edit = Some(Feedback {
            severity,
            source: NoticeSource::TagEdit,
            message: message.into(),
            recovery: None,
        });
    }

    /// Record a structured notice, routing it to its source's slot. A playback
    /// notice carries its recovery intent through untouched.
    pub fn put(&mut self, feedback: Feedback) {
        let slot = match feedback.source {
            NoticeSource::Playback | NoticeSource::Settings | NoticeSource::System => {
                &mut self.playback
            }
            NoticeSource::Scan | NoticeSource::Library => &mut self.scan,
            NoticeSource::TagEdit => &mut self.tag_edit,
        };
        *slot = Some(feedback);
    }

    /// Drop the notice from one source (e.g. a notice superseded or cleared).
    pub fn clear(&mut self, source: &NoticeSource) {
        match source {
            NoticeSource::Playback | NoticeSource::Settings | NoticeSource::System => {
                self.playback = None;
            }
            NoticeSource::Scan | NoticeSource::Library => self.scan = None,
            NoticeSource::TagEdit => self.tag_edit = None,
        }
    }

    /// Drop every notice (an empty notice state clears correctly).
    pub fn clear_all(&mut self) {
        *self = Self::default();
    }

    /// Whether no source currently has anything to report.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scan.is_none() && self.playback.is_none() && self.tag_edit.is_none()
    }

    /// The notice the titlebar paints: the highest-severity active source, with
    /// a stable tie-break (playback, then Tag Edit, then scan) so a lower
    /// severity never masks a higher one that is still live.
    #[must_use]
    pub fn display(&self) -> Option<&Feedback> {
        let rank = |f: &Feedback| match f.severity {
            NoticeSeverity::Error => 2,
            NoticeSeverity::Warning => 1,
            NoticeSeverity::Info => 0,
        };
        // Iterated low-priority-first because `max_by_key` keeps the LAST of
        // equal keys: playback (last) wins a severity tie, then Tag Edit, then
        // scan — and any higher severity still outranks them all.
        [
            self.scan.as_ref(),
            self.tag_edit.as_ref(),
            self.playback.as_ref(),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|f| rank(f))
    }

    /// The display notice's message — the string fed to the existing
    /// `scan_status` line, so the titlebar's copy and placement are unchanged.
    #[must_use]
    pub fn display_message(&self) -> Option<String> {
        self.display().map(|f| f.message.clone())
    }
}
