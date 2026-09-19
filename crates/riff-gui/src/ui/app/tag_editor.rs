//! The Inline Tag Editor's render-free controller (deepen-three-modules
//! issue 02): the ONE home in the UI layer for the editor's data and
//! lifecycle.
//!
//! The controller owns the per-selection draft, the outstanding single-Track
//! record, and the album batch's in-flight record, plus every lifecycle
//! transition — start editing, save, cancel, discard-on-selection-change —
//! and the polled-outcome matching that feeds the draft and the status line.
//! The Tag Edits front-end is constructor-injected and the controller's only
//! dependency, so it is testable without rendering and without a store.
//!
//! The draft never outlives its selection: that ADR-invariant is enforced
//! here ([`InlineTagEditor::reconcile`]), not by call-site discipline.

use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
use riff_backend::app::traits::TagEdit;
use riff_backend::domain::TrackId;
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::ui::selection::{DraftKind, TagDraft, TagField, TagRow};

use super::{InspectorContent, InspectorKind};

/// The Inline Tag Editor's state machine over the Tag Edits front-end: one
/// owner of the draft and the outstanding write records, with submission,
/// outcome polling, and status-line reporting as methods.
pub struct InlineTagEditor {
    /// The Tag Edit Service front end (ADR 0006): submits save intent and
    /// yields polled outcomes; the controller's only dependency.
    tag_edits: Box<dyn TagEdits>,
    /// The inline editor's open per-selection draft (tickets 02/03): `Some`
    /// while the editor is editing the selection; discarded the moment the
    /// selection changes — the draft never outlives its selection
    /// ([`Self::reconcile`]).
    draft: Option<TagDraft>,
    /// The one inline Tag Edit currently outstanding, recorded at submit so a
    /// polled outcome can be matched back to the draft (and its file name
    /// shown in the status line) — outcomes themselves carry no identity.
    in_flight: Option<(TrackId, PathBuf)>,
    /// An album batch's outstanding requests (ticket 03), serialized by the
    /// worker and polled in submission order.
    batch: Option<BatchInFlight>,
}

impl InlineTagEditor {
    /// Wire the controller to the Tag Edits front-end; starts idle.
    #[must_use]
    pub fn new(tag_edits: Box<dyn TagEdits>) -> Self {
        Self {
            tag_edits,
            draft: None,
            in_flight: None,
            batch: None,
        }
    }

    /// The open draft, when editing is in progress — the widget reads the
    /// buffers; `None` while idle.
    #[must_use]
    pub fn draft(&self) -> Option<&TagDraft> {
        self.draft.as_ref()
    }

    /// The open draft's mutable buffers (the widget edits fields in place);
    /// `None` while idle.
    #[must_use]
    pub fn draft_mut(&mut self) -> Option<&mut TagDraft> {
        self.draft.as_mut()
    }

    /// Start editing a single-Track readout: open the per-selection draft
    /// prefilled from the readout's rows. Replaces any prior draft.
    pub fn open_track(&mut self, track_id: TrackId, path: PathBuf, tags: &[TagRow]) {
        self.draft = Some(TagDraft::for_track(track_id, path, tags));
    }

    /// Start editing an Album readout: open the batch draft over the album's
    /// tracks in store order, prefilled from the readout's aggregation.
    /// Replaces any prior draft.
    pub fn open_album(&mut self, album_tracks: Vec<(TrackId, PathBuf)>, tags: &[TagRow]) {
        self.draft = Some(TagDraft::for_album(album_tracks, tags));
    }

    /// Validate the open draft and submit its requests through the Tag Edits
    /// front-end: a single-Track draft's one request, or an Album draft's
    /// dirty-only batch. Invalid numeric fields keep the draft open with an
    /// inline reason and submit nothing. Nothing is ever written without an
    /// explicit Save upstream.
    pub fn save(&mut self) {
        match self.draft.as_ref().map(|d| d.kind) {
            Some(DraftKind::Track) => self.submit_track(),
            Some(DraftKind::Album) => self.submit_album(),
            None => {}
        }
    }

    /// Cancel editing: discard the draft. An already-submitted write keeps
    /// completing — its polled outcome still reaches the status line, the
    /// editor simply stays gone.
    pub fn cancel(&mut self) {
        self.draft = None;
    }

    /// Discard the draft the moment its selection leaves: the draft never
    /// outlives its selection (ADR-invariant, enforced here, not by call-site
    /// discipline). A submitted write keeps completing; only the editor stops
    /// rendering once its selection leaves.
    pub fn reconcile(&mut self, content: &InspectorContent) {
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| !Self::draft_belongs(draft, content))
        {
            self.draft = None;
        }
    }

    /// Drain polled Tag Edit outcomes from the front-end. On [`Saved`] the
    /// open draft closes (or the album batch tallies), and the status line
    /// reports the saved file; on `Failed` the draft keeps its inline reason
    /// — there is no silent-success path.
    pub fn poll_outcomes(&mut self, scan_status: &mut Option<String>) {
        while let Some(outcome) = self.tag_edits.poll() {
            // Outcomes carry no identity; the outstanding record captured at
            // submit time routes the outcome to its flow. Only one edit is
            // outstanding at a time: the album batch's record first, then the
            // single-track inline record.
            if self.batch.is_some() {
                self.apply_batch_outcome(outcome, scan_status);
            } else if self.in_flight.is_some() {
                self.apply_track_outcome(outcome, scan_status);
            }
        }
    }

    /// Whether the draft still belongs to the resolved readout: a Track draft
    /// to that track, an Album draft to the album whose track batch it
    /// targets. An Artist/Genre readout never hosts one.
    fn draft_belongs(draft: &TagDraft, content: &InspectorContent) -> bool {
        match (draft.kind, content.kind) {
            (DraftKind::Track, InspectorKind::Track) => {
                content.track_ids.first() == Some(&draft.track_id)
            }
            (DraftKind::Album, InspectorKind::Album) => {
                let ids: Vec<&TrackId> = draft.album_tracks.iter().map(|(id, _)| id).collect();
                content.track_ids.iter().collect::<Vec<_>>() == ids
            }
            _ => false,
        }
    }

    /// Submit a single-Track draft's one request and record it as
    /// outstanding. Invalid numeric fields keep the draft open with an inline
    /// reason and submit nothing.
    fn submit_track(&mut self) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        match (
            parse_number("Year", &draft.fields[TagField::Year.index()]),
            parse_number("Track number", &draft.fields[TagField::TrackNumber.index()]),
        ) {
            (Ok(year), Ok(track_number)) => {
                draft.error = None;
                draft.saving = true;
                let request = TagEditRequest {
                    track_id: draft.track_id.clone(),
                    path: draft.path.clone(),
                    edit: TagEdit {
                        title: Some(draft.fields[TagField::Title.index()].clone()),
                        artist: Some(draft.fields[TagField::Artist.index()].clone()),
                        album: Some(draft.fields[TagField::Album.index()].clone()),
                        album_artist: Some(draft.fields[TagField::AlbumArtist.index()].clone()),
                        genre: Some(draft.fields[TagField::Genre.index()].clone()),
                        year,
                        track_number,
                        ..Default::default()
                    },
                };
                self.in_flight = Some((request.track_id.clone(), request.path.clone()));
                self.tag_edits.submit(request);
            }
            (Err(error), _) | (_, Err(error)) => {
                draft.error = Some(error);
            }
        }
    }

    /// Submit the album draft's dirty-only batch: one [`TagEditRequest`] per
    /// album Track, carrying only the fields whose typed text differs from
    /// the row's originally displayed value (`Some`) — a `(different)` row
    /// left with an empty input is untouched, so it is skipped and can never
    /// blank a tag on every Track; clearing a shared tag stays a single-Track
    /// action. A fully untouched editor submits nothing and writes no files
    /// (Save is disabled while nothing is dirty). Invalid numeric fields keep
    /// the draft open with an inline reason and submit nothing.
    fn submit_album(&mut self) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if !draft.any_dirty() {
            return;
        }
        // Only a dirty numeric field is parsed: an untouched Year/Track
        // Number stays `None` in every request rather than being rewritten.
        let year = if draft.is_dirty(TagField::Year) {
            match parse_number("Year", &draft.fields[TagField::Year.index()]) {
                Ok(y) => y,
                Err(error) => {
                    draft.error = Some(error);
                    return;
                }
            }
        } else {
            None
        };
        let track_number = if draft.is_dirty(TagField::TrackNumber) {
            match parse_number("Track number", &draft.fields[TagField::TrackNumber.index()]) {
                Ok(n) => n,
                Err(error) => {
                    draft.error = Some(error);
                    return;
                }
            }
        } else {
            None
        };
        let value = |field: TagField| -> Option<String> {
            draft
                .is_dirty(field)
                .then(|| draft.fields[field.index()].clone())
        };
        let pending = draft
            .album_tracks
            .iter()
            .map(|(track_id, path)| {
                let request = TagEditRequest {
                    track_id: track_id.clone(),
                    path: path.clone(),
                    edit: TagEdit {
                        title: value(TagField::Title),
                        artist: value(TagField::Artist),
                        album: value(TagField::Album),
                        album_artist: value(TagField::AlbumArtist),
                        genre: value(TagField::Genre),
                        year,
                        track_number,
                        ..Default::default()
                    },
                };
                self.tag_edits.submit(request);
                (track_id.clone(), path.clone())
            })
            .collect::<VecDeque<_>>();

        draft.error = None;
        draft.saving = true;
        draft.batch = Some(crate::ui::selection::BatchStatus {
            total: pending.len(),
            saved: 0,
            failed: 0,
            first_failure: None,
        });
        self.batch = Some(BatchInFlight::new(pending));
    }

    /// Apply one polled Tag Edit outcome to the single-Track flow. `Saved`
    /// closes the matching draft and reports the saved file; `Failed` keeps
    /// the editor open with the reason inline. A draft that has already been
    /// discarded (the selection moved) never comes back: the status line
    /// still reports the outcome, the editor simply stays gone.
    fn apply_track_outcome(&mut self, outcome: TagEditOutcome, scan_status: &mut Option<String>) {
        let Some((track_id, path)) = self.in_flight.take() else {
            return;
        };
        match outcome {
            TagEditOutcome::Saved => {
                let name = path.file_name().map_or_else(
                    || path.to_string_lossy().to_string(),
                    |n| n.to_string_lossy().to_string(),
                );
                *scan_status = Some(format!("Tags saved for {name}"));
                tracing::info!("Tags written for {:?}", path);
                if self.draft.as_ref().is_some_and(|d| d.track_id == track_id) {
                    self.draft = None;
                }
            }
            TagEditOutcome::Failed { reason } => {
                tracing::warn!("Tag edit failed for {:?}: {}", path, reason);
                if let Some(d) = self.draft.as_mut()
                    && d.track_id == track_id
                {
                    d.error = Some(reason);
                    d.saving = false;
                }
            }
        }
    }

    /// Apply one polled Tag Edit outcome to the album batch: each outcome
    /// lands on the next pending request in submission order (the worker
    /// serializes the batch), updating the draft's tallies and the status
    /// line — "Tags saved for X" per save, the failure reason per failure,
    /// exactly the per-request surface single-Track saves use. When the last
    /// outcome lands the draft stops saving, its
    /// [`crate::ui::selection::BatchStatus`] turns done, and the Save bar
    /// shows the "Saved N of M tracks" summary (orange when any failed). A
    /// draft that has already been discarded (the selection moved) never
    /// comes back; the status line still reports each outcome.
    fn apply_batch_outcome(&mut self, outcome: TagEditOutcome, scan_status: &mut Option<String>) {
        let Some(in_flight) = self.batch.as_mut() else {
            return;
        };
        let Some((_track_id, path)) = in_flight.pending.pop_front() else {
            return;
        };
        match outcome {
            TagEditOutcome::Saved => {
                in_flight.saved += 1;
                let name = path.file_name().map_or_else(
                    || path.to_string_lossy().to_string(),
                    |n| n.to_string_lossy().to_string(),
                );
                *scan_status = Some(format!("Tags saved for {name}"));
            }
            TagEditOutcome::Failed { reason } => {
                in_flight.failed += 1;
                in_flight.first_failure.get_or_insert(reason.clone());
                *scan_status = Some(reason);
            }
        }
        if let Some(d) = self.draft.as_mut() {
            d.batch = Some(crate::ui::selection::BatchStatus {
                total: in_flight.total,
                saved: in_flight.saved,
                failed: in_flight.failed,
                first_failure: in_flight.first_failure.clone(),
            });
        }
        if in_flight.pending.is_empty() {
            self.batch = None;
            if let Some(d) = self.draft.as_mut() {
                d.saving = false;
            }
        }
    }
}

/// The album batch's outstanding requests, in submission order — the worker
/// serializes the batch, so polled outcomes arrive in the same order and each
/// lands on the record popped first. The tallies feed the draft's
/// [`crate::ui::selection::BatchStatus`] the Save bar renders.
#[derive(Debug, Clone)]
struct BatchInFlight {
    pending: VecDeque<(TrackId, PathBuf)>,
    total: usize,
    saved: usize,
    failed: usize,
    first_failure: Option<String>,
}

impl BatchInFlight {
    fn new(pending: VecDeque<(TrackId, PathBuf)>) -> Self {
        let total = pending.len();
        Self {
            pending,
            total,
            saved: 0,
            failed: 0,
            first_failure: None,
        }
    }
}

/// Parse an optional numeric tag field; empty input means "leave unset".
fn parse_number(label: &str, raw: &str) -> Result<Option<u32>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        trimmed
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("{label} must be a whole number"))
    }
}
