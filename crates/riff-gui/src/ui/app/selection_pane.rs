//! The inspector (the collapsible selection panel, design-handoff issue 10;
//! elastic-column spec): the elastic stage's rightmost column, rendered only
//! while a selection exists. Follows the live selection — the selected track
//! when a track row was single-clicked (in any track listing), otherwise the
//! deepest path entity (album > artist > genre) — and hides completely when
//! nothing is selected. Click-to-lock only: no hover behavior.
//!
//! Child module of `ui::app` so the pane methods keep direct access to
//! [`RiffApp`]'s fields, exactly like the methods they sit beside.

use eframe::egui;
use std::path::PathBuf;

use riff_backend::app::state::LibrarySession;

use super::super::selection;
use super::{
    COVER_CARD, InspectorKind, RiffApp, apply_selection_action, request_cover_intent,
    resolve_inspector,
};

impl RiffApp {
    /// The inspector column: whatever the session has selected, resolved
    /// through the Session Views seam — art requested through the selection's
    /// cover track, the details grid, and the Play / Add to Queue actions
    /// over the selection's track batch. The stage gates visibility: this
    /// method only runs when [`resolve_inspector`] resolved a readout.
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
            // same flow the browser column's thumbnails use; the texture
            // comes from the UI LRU, and a full miss resolves the shared
            // music-icon placeholder tile.
            request_cover_intent(
                &self.cover_textures,
                self.covers.as_ref(),
                tid.clone(),
                PathBuf::from(&tid.0),
                COVER_CARD,
            );
            crate::ui::cover_placeholder::lookup_cover_texture(
                &mut self.cover_textures,
                &mut self.cover_lru_keys,
                ui.ctx(),
                &self.theme.active,
                &tid.0,
                COVER_CARD,
            )
        });
        let panel = selection::SelectionPanel {
            art: art.as_ref(),
            title: content.title.as_deref(),
            subtitle: content.subtitle.as_deref(),
            details: &content.details,
            tags: &content.tags,
            // The editor renders only while a draft is open for this exact
            // readout — a Track draft on that track, an Album batch draft on
            // that album (ticket 03) — which the controller's reconcile
            // just enforced.
            editor: self.tag_editor.draft_mut(),
            // A track readout plays just that one track (the primary action
            // reads **Play**); entity readouts play their whole batch.
            single: content.kind == InspectorKind::Track,
            // The inspector always offers the quick-action row: Play and
            // Add to Queue.
            queue: true,
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
                selection::SelectionAction::PlayAlbum | selection::SelectionAction::Queue => {
                    apply_selection_action(action, self.transport.as_ref(), &content.track_ids);
                }
                // The Save bar's intents belong to the editor flow: the
                // controller owns the draft and the request.
                selection::SelectionAction::SaveTagEdit => self.tag_editor.save(),
                selection::SelectionAction::CancelTagEdit => self.tag_editor.cancel(),
                // A tag row click opens the per-selection draft — a Track
                // draft on a track readout, an Album batch draft on an album
                // readout — resolved through the same Session Views source.
                selection::SelectionAction::StartEdit => {
                    if self.tag_editor.draft().is_none() {
                        self.open_inline_draft(&content);
                    }
                }
            }
        }
    }
}
