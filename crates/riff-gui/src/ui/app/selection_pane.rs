//! The inspector (the collapsible selection panel, design-handoff issue 10;
//! elastic-column spec): the elastic stage's rightmost column, rendered only
//! while a selection exists. Follows the live selection — the selected track
//! when a track row was single-clicked (in any track listing), otherwise the
//! deepest path entity (album > artist > genre) — and hides completely when
//! nothing is selected. Click-to-lock only: no hover behavior.
//!
//! A READOUT, not a control panel: it displays art, title, subtitle, the
//! readout's kind, tag rows, and the details grid, and its one way inward is a
//! tag row opening the Inline Tag Editor. It offers no playback or queueing
//! action — those belong to the selection's own context menu.
//!
//! Child module of `ui::app` so the pane methods keep direct access to
//! [`RiffApp`]'s fields, exactly like the methods they sit beside.

use eframe::egui;

use riff_backend::app::state::LibrarySession;

use super::super::selection;
use super::{COVER_CARD, InspectorKind, RiffApp, cover_texture_for, resolve_inspector};

/// The readout KIND's display name, for the header chip. The widget seam takes
/// the name rather than the kind so it never has to know the app's own type —
/// and so the chip cannot be right about the readout by accident, because the
/// caller has to name all four cases.
fn kind_label(kind: InspectorKind) -> &'static str {
    match kind {
        InspectorKind::Album => "Album",
        InspectorKind::Artist => "Artist",
        InspectorKind::Genre => "Genre",
        InspectorKind::Track => "Track",
    }
}

impl RiffApp {
    /// The inspector column: whatever the session has selected, resolved
    /// through the Session Views seam — art requested through the selection's
    /// cover track, the details grid, and the tag rows the listener can click
    /// their way into. A READOUT: it displays the selection and offers no
    /// playback or queueing action, because those live on the selection's own
    /// context menu. The stage gates visibility: this method only runs when
    /// [`resolve_inspector`] resolved a readout.
    pub(super) fn render_inspector(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        let content = resolve_inspector(&mut self.views, library);
        // The draft never outlives its selection: the controller discards a
        // changed selection (or a readout the store no longer carries) this
        // frame, so a stale half-typed edit can never leak onto a different
        // Track. An already-submitted request keeps completing; only the
        // editor stops rendering once its selection leaves.
        self.tag_editor.reconcile(&content);
        let art = content.art_track.as_ref().map(|tid| {
            // The cover intent goes through the selection's cover track —
            // same flow the browser column's thumbnails use; the Cover Cache
            // decides whether a request is due and the View half answers with
            // the texture, which on a full miss is the shared music-icon
            // placeholder tile.
            cover_texture_for(
                &mut self.cover_cache,
                self.covers.as_ref(),
                &mut self.cover_textures,
                &mut self.cover_lru_keys,
                ui.ctx(),
                &self.theme.active,
                tid,
                COVER_CARD,
            )
        });
        let panel = selection::SelectionPanel {
            art: art.as_ref(),
            title: content.title.as_deref(),
            subtitle: content.subtitle.as_deref(),
            // The header chip names what is being read out, so the readout can
            // never introduce itself as an Album it is not. The name is resolved
            // here because this is where the readout's kind is known; the widget
            // seam only ever sees a string.
            kind: kind_label(content.kind),
            details: &content.details,
            tags: &content.tags,
            // The editor renders only while a draft is open for this exact
            // readout — a Track draft on that track, an Album batch draft on
            // that album (ticket 03) — which the controller's reconcile
            // just enforced.
            editor: self.tag_editor.draft_mut(),
        };
        // The panel's 16px inset, as the fixed pane had — the art block and
        // readout sit clear of the column edge. The intents bubble OUT of the
        // closure (which holds the panel's borrow of `inline_draft`) so they
        // can be applied against whole-`self` afterwards.
        let actions = egui::Frame::new()
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                let mut actions = Vec::new();
                selection::show_selection_panel(
                    ui,
                    &mut self.icons,
                    &self.theme.active,
                    panel,
                    &mut actions,
                );
                actions
            })
            .inner;
        for action in actions {
            match action {
                // The Save bar's intents belong to the editor flow: the
                // controller owns the draft and the request.
                selection::SelectionAction::SaveTagEdit => self.tag_editor.save(),
                selection::SelectionAction::CancelTagEdit => self.tag_editor.cancel(),
                // A tag row click opens the per-selection draft — a Track
                // draft on a track readout, an Album batch draft on an album
                // readout — resolved through the same Session Views source.
                // This is the panel's ONE way inward; there is no other arm,
                // because there is no longer anything else it can report.
                selection::SelectionAction::StartEdit => {
                    if self.tag_editor.draft().is_none() {
                        self.open_inline_draft(&content);
                    }
                }
            }
        }
    }
}
