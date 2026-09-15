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
    InspectorKind, RiffApp, apply_selection_action, request_cover_intent, resolve_inspector,
};

impl RiffApp {
    /// The inspector column: whatever the session has selected, resolved
    /// through the Session Views seam — art requested through the selection's
    /// cover track, the details grid, and the Play / Add to Queue actions
    /// over the selection's track batch. The stage gates visibility: this
    /// method only runs when [`resolve_inspector`] resolved a readout.
    pub(super) fn render_inspector(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        let content = resolve_inspector(&mut self.views, library);
        let art = content.art_track.as_ref().map(|tid| {
            // The cover intent goes through the selection's cover track —
            // same flow the browser column's thumbnails use; the texture
            // comes from the UI LRU, and a full miss resolves the shared
            // music-icon placeholder tile.
            request_cover_intent(
                self.cover_textures.contains_key(&tid.0),
                self.covers.as_ref(),
                tid.clone(),
                PathBuf::from(&tid.0),
            );
            crate::ui::cover_placeholder::lookup_cover_texture(
                &mut self.cover_textures,
                &mut self.cover_lru_keys,
                ui.ctx(),
                &self.theme.active,
                &tid.0,
            )
        });
        let panel = selection::SelectionPanel {
            art: art.as_ref(),
            title: content.title.as_deref(),
            subtitle: content.subtitle.as_deref(),
            details: &content.details,
            // A track readout plays just that one track (the primary action
            // reads **Play**); entity readouts play their whole batch.
            single: content.kind == InspectorKind::Track,
            // The inspector always offers the quick-action row: Play and
            // Add to Queue.
            queue: true,
        };
        // The panel's 16px inset, as the fixed pane had — the art block and
        // readout sit clear of the column edge.
        egui::Frame::new()
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
                for action in actions {
                    apply_selection_action(action, self.transport.as_ref(), &content.track_ids);
                }
            });
    }
}
