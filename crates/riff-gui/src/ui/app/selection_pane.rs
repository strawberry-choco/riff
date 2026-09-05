//! The selection panel pane (design-handoff issue 10): the third pane of
//! the three-pane explorer, rendered in its own right panel between the
//! detail column and the window edge. A persistent selection readout — it
//! follows the session's last album selection across section navigation
//! while the listener browses any section.
//!
//! Child module of `ui::app` so the pane methods keep direct access to
//! [`RiffApp`]'s fields, exactly like the methods they sit beside.

use eframe::egui;
use std::path::PathBuf;

use riff_backend::app::state::{LibrarySession, ViewMode};

use super::super::selection;
use super::{RiffApp, apply_selection_action, request_cover_intent, resolve_selection_panel};

impl RiffApp {
    /// The selection panel's right panel (handoff issue 10): a 300px pane
    /// between the detail column and the window edge — third pane of the
    /// three-pane explorer. Library content only: the Settings and Now
    /// Playing stages replace the whole explorer.
    pub(super) fn render_selection_panel_panel(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
    ) {
        if library.view_mode != ViewMode::Library {
            return;
        }
        egui::Panel::right("selection_panel")
            .exact_size(crate::ui::theme::SELECT_PANEL_W)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(16)))
            .show(ui, |ui| {
                self.render_selection_pane(ui, library);
            });
    }

    /// The selection panel pane (handoff issue 10): whatever album the
    /// session last selected, resolved through the Session Views facade —
    /// art requested through the album's first track, the details grid, and
    /// the Play album action over the album's track batch.
    fn render_selection_pane(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        let content = resolve_selection_panel(&mut self.views, library);
        let art = content.art_track.as_ref().map(|tid| {
            // The cover intent goes through the album's first track — the
            // same flow the browser column's thumbnails use; the texture
            // comes from the UI LRU, and a full miss resolves the generated
            // colour block (issue 14).
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
                self.theme.active.dark,
                &tid.0,
            )
        });
        let panel = selection::SelectionPanel {
            art: art.as_ref(),
            title: content.title.as_deref(),
            subtitle: content.subtitle.as_deref(),
            details: &content.details,
        };
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
    }
}
