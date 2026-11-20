//! The debug playback readout: the bridge's signals made visible, with
//! transport controls to drive them (the N3 evidence screenshot captures
//! this screen mid-playback). Reachable only in debug builds — the entry
//! point lives under `#[cfg(debug_assertions)]` in `crate::app` — beside the
//! N2 fixture, on the same toggle pattern.
//!
//! The readout renders from the `UiState` handles only; the controls
//! dispatch through the model's transport and never write a signal (the
//! pump is the only writer). Keys: mod+alt+r toggles the readout, space
//! toggles play/pause, mod+alt+p plays a random session, mod+alt+n skips,
//! mod+alt+o stops (the queue stays parked), mod+alt+s seeks to 1 s.

use nsh_base::button;
use nsh_core::prelude::*;

use nsh_core::element::{Signal, Stored};
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::fonts::{INTER_FAMILY, WEIGHT_SEMIBOLD};
use crate::theme::tokens;

/// The readout's control surface: mod+alt+r toggles it. Debug builds open on
/// the readout; the fixture stays reachable on mod+alt+t.
pub fn install() -> Signal<bool> {
    let show = signal(true);
    on_key("mod+alt+r", move |_| show.update(|s| *s = !*s));
    show
}

/// The readout: the playback view, the queue line, the current track, the
/// revisions and the last notice, then the transport button row.
pub fn readout(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .gap(px(tokens::SPACE_LG))
        .p(px(tokens::SPACE_LG))
        .bg(move || state.palette.get().background)
        .text_color(move || state.palette.get().ink)
        .child(title(state))
        .child(playback_line(state))
        .child(queue_line(state))
        .child(track_line(state))
        .child(revisions_line(state))
        .child(notice_line(state))
        .child(controls(model))
}

/// The header.
fn title(state: UiState) -> impl IntoElement {
    text("riff bridge — playback readout (debug)")
        .text_xl()
        .font_family(INTER_FAMILY)
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(move || state.palette.get().ink)
}

/// The playback state line, the headless tests' primary readout.
fn playback_line(state: UiState) -> impl IntoElement {
    let line = move || {
        let v = state.playback.get();
        let duration = v
            .duration_secs
            .map_or("--".to_owned(), |d| format!("{d:.1}"));
        format!(
            "{} pos {:.1} dur {duration} vol {:.2} muted {}",
            state_name(v.state),
            v.position_secs,
            v.volume,
            v.muted,
        )
    };
    text(line).text_sm().test_id("playback")
}

/// The queue line: position in queue, shuffle and repeat.
fn queue_line(state: UiState) -> impl IntoElement {
    let line = move || {
        let v = state.playback.get();
        format!(
            "queue {}/{} shuffle {} repeat {:?}",
            v.queue_index, v.queue_len, v.shuffle, v.repeat
        )
    };
    text(line).text_sm().test_id("queue")
}

/// The current track line.
fn track_line(state: UiState) -> impl IntoElement {
    let line = move || {
        state.playback.get().current.map_or_else(
            || "now nothing".to_owned(),
            |track| {
                let title = track.metadata.title.clone().unwrap_or_default();
                let artist = track.metadata.artist.clone().unwrap_or_default();
                format!("now {title} — {artist}")
            },
        )
    };
    text(line).text_sm().test_id("track")
}

/// The revisions line: the counters a screen's `dynamic` blocks read.
fn revisions_line(state: UiState) -> impl IntoElement {
    let line = move || {
        format!(
            "lib {} pl {} covers {}",
            state.library_revision.get(),
            state.playlist_revision.get(),
            state.covers.get(),
        )
    };
    text(line).text_sm().test_id("revisions")
}

/// The last typed notice, if any.
fn notice_line(state: UiState) -> impl IntoElement {
    let line = move || {
        state.notices.get().last().map_or_else(
            || "no notices".to_owned(),
            |notice| format!("notice {:?}: {}", notice.severity, notice.message),
        )
    };
    text(line).text_sm().test_id("notice")
}

/// The transport button row. Each handler dispatches through the model's
/// transport — the same intents the production player bar sends at N4 — and
/// writes no signal: the pump reads the session the engine mutates.
fn controls(model: Stored<UiModel>) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(tokens::SPACE_SM))
        .child(
            button()
                .test_id("play")
                .on_press(move || model.with(|m| m.rt.ui_transport.play_random()))
                .child(text("Play random")),
        )
        .child(
            button()
                .test_id("playpause")
                .on_press(move || {
                    model.with(|m| {
                        let session = m.rt.playback.lock_or_recover();
                        m.rt.ui_transport.play_pause(&session);
                    });
                })
                .child(text("Play/Pause")),
        )
        .child(
            button()
                .test_id("next")
                .on_press(move || model.with(|m| m.rt.ui_transport.next()))
                .child(text("Next")),
        )
        .child(
            button()
                .test_id("stop")
                .on_press(move || model.with(|m| m.rt.ui_transport.stop()))
                .child(text("Stop")),
        )
        .child(
            button()
                .test_id("seek")
                .on_press(move || {
                    model.with(|m| {
                        let session = m.rt.playback.lock_or_recover();
                        m.rt.ui_transport.seek(&session, 1.0);
                    });
                })
                .child(text("Seek 1s")),
        )
        .child(
            button()
                .test_id("vol-down")
                .on_press(move || step_volume(model, -0.1))
                .child(text("Vol −")),
        )
        .child(
            button()
                .test_id("vol-up")
                .on_press(move || step_volume(model, 0.1))
                .child(text("Vol +")),
        )
        .child(
            button()
                .test_id("mute")
                .on_press(move || {
                    model.with(|m| {
                        let mut session = m.rt.playback.lock_or_recover();
                        m.rt.ui_transport.toggle_mute(&mut session);
                    });
                })
                .child(text("Mute")),
        )
}

/// Nudge the volume by `delta`, through the transport's `set_volume` (the
/// write path the settings round-trip proves: the pump's diff-commit lands
/// it in the store).
fn step_volume(model: Stored<UiModel>, delta: f32) {
    model.with(|m| {
        let mut session = m.rt.playback.lock_or_recover();
        let next = (session.current_volume + delta).clamp(0.0, 1.0);
        m.rt.ui_transport.set_volume(&mut session, next);
    });
}

/// The debug key surface: the transport intents on keystrokes, so the
/// windowed drive scripts press keys instead of clicking coordinates.
pub(crate) fn install_transport_keys(model: Stored<UiModel>) {
    on_key("space", move |_| {
        model.with(|m| {
            let session = m.rt.playback.lock_or_recover();
            m.rt.ui_transport.play_pause(&session);
        });
    });
    on_key("mod+alt+p", move |_| {
        model.with(|m| m.rt.ui_transport.play_random());
    });
    on_key("mod+alt+n", move |_| {
        model.with(|m| m.rt.ui_transport.next());
    });
    on_key("mod+alt+o", move |_| {
        model.with(|m| m.rt.ui_transport.stop());
    });
    on_key("mod+alt+s", move |_| {
        model.with(|m| {
            let session = m.rt.playback.lock_or_recover();
            m.rt.ui_transport.seek(&session, 1.0);
        });
    });
    on_key("mod+alt+m", move |_| {
        model.with(|m| {
            let mut session = m.rt.playback.lock_or_recover();
            m.rt.ui_transport.toggle_mute(&mut session);
        });
    });
}

/// The session's playback state, spelled the way the spike's readout was, so
/// the headless assertions match on the same words.
fn state_name(state: riff_backend::app::state::PlaybackState) -> &'static str {
    match state {
        riff_backend::app::state::PlaybackState::Playing => "playing",
        riff_backend::app::state::PlaybackState::Paused => "paused",
        riff_backend::app::state::PlaybackState::Stopped => "stopped",
    }
}
