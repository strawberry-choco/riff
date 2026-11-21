//! The player bar: the port of the egui `ui/playerbar.rs` behavior, not
//! pixels — seek row over a three-zone row of track info, transport and
//! controls. Every control reflects the `PlaybackView` signal and
//! dispatches through `rt.ui_transport`; seek and volume ride the nsh
//! slider, seek only interactive while the track's duration is known
//! (`--:--` and a disabled thumb otherwise, as the egui seek row did).

use std::sync::Arc;

use nsh_base::{SliderValue, slider};
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::{PlaybackState, RepeatMode};
use riff_backend::domain::Track;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::Palette;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

use super::focus;
use super::ring;

/// The bar: seek row spanning the top, then info | transport | controls.
pub fn bar(
    model: Stored<UiModel>,
    state: UiState,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .h(px(tokens::PLAYERBAR_H))
        .flex_col()
        .test_id("player-bar")
        .bg(move || palette.get().surface)
        .border_t(px(1.0))
        .border_color(move || palette.get().line)
        .px(px(tokens::SPACE_LG))
        .py(px(tokens::SPACE_SM))
        .gap(px(tokens::SPACE_XS))
        .child(seek_row(model, state))
        .child(
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_LG))
                .child(info(state))
                .child(transport(model, state, focus))
                .child(controls(model, state, focus)),
        )
}

/// The seek row: elapsed, the slider, total. The slider's `max` tracks the
/// duration signal so a longer track's bar stretches, and its value reads
/// the pump-quantized position.
fn seek_row(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .child(
            text(move || mmss(state.playback.get().position_secs))
                .text_xs()
                .text_color(move || palette.get().ink_2)
                .test_id("elapsed"),
        )
        .child(seek_slider(model, state).flex_1())
        .child(
            text(move || {
                state
                    .playback
                    .get()
                    .duration_secs
                    .map_or_else(|| "--:--".to_owned(), mmss)
            })
            .text_xs()
            .text_color(move || palette.get().ink_2)
            .test_id("total"),
        )
}

// The slider's f64 steps to the transport's f32 seconds: the step already
// quantized the value, nothing to lose.
#[allow(clippy::cast_possible_truncation)]
fn seek_slider(model: Stored<UiModel>, state: UiState) -> nsh_base::SliderElement {
    let palette = state.palette;
    let slider = slider()
        .test_id("seek")
        .accessibility_label("Seek")
        .value(move || {
            let view = state.playback.get();
            view.duration_secs
                .map(|total| SliderValue::Single(f64::from(view.position_secs.min(total))))
        })
        .min(0.0)
        .max(move || f64::from(state.playback.get().duration_secs.unwrap_or(1.0)))
        .step(0.1)
        .disabled(move || state.playback.get().duration_secs.is_none())
        .on_change(move |value: &SliderValue| {
            let target = value.start() as f32;
            model.with(|m| {
                let session = m.rt.playback.lock_or_recover();
                m.rt.ui_transport.seek(&session, target);
            });
        });
    slider_parts(slider, palette)
}

/// The nsh slider's anatomy, Rust-mode: the component's Tailwind classes are
/// TS-side defaults and arrive as `None` here, so track, rail, range and
/// thumb carry no size at all until this view sets them. The geometry is
/// the framework's own documented slider shape (`h-6` track, `h-1.5`-tall
/// rail, thumb over it); the colors are the riff palette's, with the range
/// in the brand fill the egui seek row painted.
#[must_use]
pub fn slider_parts(
    slider: nsh_base::SliderElement,
    palette: Signal<Palette>,
) -> nsh_base::SliderElement {
    slider
        .track_class("w-full h-6 justify-center")
        .track_style(|s| s.bg(move || palette.get().surface_3))
        .indicator_class("w-full")
        .indicator_style(|s| {
            s.h(px(tokens::SPACE_SM))
                .rounded(px(tokens::RADIUS_FULL))
                .bg(move || palette.get().surface_3)
        })
        .range_class("h-full")
        .range_style(|s| {
            s.rounded(px(tokens::RADIUS_FULL))
                .bg(move || palette.get().brand_primary)
        })
        .thumb_style(|s| {
            s.w(px(tokens::SPACE_LG))
                .h(px(tokens::SPACE_LG))
                .rounded(px(tokens::RADIUS_FULL))
                .bg(move || palette.get().ink)
        })
}

/// Track info: title over the artist-album meta line, the egui idle copy
/// when nothing plays.
fn info(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .child(
            view()
                .flex_1()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .child(
                    text(move || {
                        state.playback.get().current.map_or_else(
                            || "Nothing playing".to_owned(),
                            |track| track.metadata.title.clone().unwrap_or_default(),
                        )
                    })
                    .text_sm()
                    .truncate()
                    .text_color(move || palette.get().ink)
                    .test_id("track-title"),
                )
                .child(
                    text(move || meta_line(state.playback.get().current))
                        .text_xs()
                        .truncate()
                        .text_color(move || palette.get().ink_3)
                        .test_id("track-meta"),
                ),
        )
}

/// `"Artist — Album"`, empty while nothing plays.
fn meta_line(current: Option<Arc<Track>>) -> String {
    let Some(track) = current else {
        return String::new();
    };
    let artist = track.metadata.artist.clone().unwrap_or_default();
    let album = track.metadata.album.clone().unwrap_or_default();
    match (artist.is_empty(), album.is_empty()) {
        (true, true) => String::new(),
        (false, true) => artist,
        (true, false) => album,
        (false, false) => format!("{artist} — {album}"),
    }
}

/// The transport cluster: previous, the state-driven primary, next.
fn transport(
    model: Stored<UiModel>,
    state: UiState,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .child(ghost(
            palette,
            focus,
            Icon::SkipBack,
            focus::PREVIOUS,
            "transport-previous",
            "Previous",
            move || model.with(|m| m.rt.ui_transport.previous()),
        ))
        .child(primary(model, state, focus))
        .child(ghost(
            palette,
            focus,
            Icon::SkipForward,
            focus::NEXT,
            "transport-next",
            "Next",
            move || model.with(|m| m.rt.ui_transport.next()),
        ))
}

/// The primary button: pause while playing, resume while paused, and —
/// stopped, with no selection to play until N5 brings one — a fresh random
/// session, exactly the egui bar's stopped branch.
fn primary(
    model: Stored<UiModel>,
    state: UiState,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    let playing = move || state.playback.get().state == PlaybackState::Playing;
    let on_press = move || {
        model.with(|m| {
            let session = m.rt.playback.lock_or_recover();
            match session.playback_state {
                PlaybackState::Playing => m.rt.ui_transport.pause(),
                PlaybackState::Paused => m.rt.ui_transport.resume(),
                PlaybackState::Stopped => m.rt.ui_transport.play_random(),
            }
        });
    };
    ring(
        view()
            .on_press(on_press)
            .test_id("transport-play")
            .label(move || if playing() { "Pause" } else { "Play" })
            .p(px(tokens::SPACE_MD))
            .rounded(px(tokens::RADIUS_FULL))
            .bg(move || palette.get().brand_primary)
            .child(
                when(playing, {
                    move || icon(Icon::Pause, Icon::ICON_SIZE, move || palette.get().on_brand)
                })
                .otherwise({
                    move || icon(Icon::Play, Icon::ICON_SIZE, move || palette.get().on_brand)
                }),
            ),
        focus::PLAY,
        focus,
        palette,
    )
}

/// A ghost control: the glyph, a hover wash, and its ring slot.
#[allow(clippy::too_many_arguments)]
fn ghost(
    palette: Signal<Palette>,
    focus: Signal<Option<usize>>,
    glyph: Icon,
    slot: usize,
    id: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    ring(
        view()
            .on_press(on_press)
            .test_id(id)
            .label(label)
            .p(px(tokens::SPACE_SM))
            .rounded(px(tokens::RADIUS_FULL))
            .hover(|s| s.bg(move || palette.get().row_hover))
            .child(icon(glyph, Icon::ICON_SIZE, move || palette.get().ink)),
        slot,
        focus,
        palette,
    )
}

/// The right cluster: shuffle, repeat, the queue position, mute and the
/// volume slider — every toggle carrying the active brand tint while its
/// mode is engaged, as the egui bar's toggles did.
fn controls(
    model: Stored<UiModel>,
    state: UiState,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    view()
        .flex_1()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(tokens::SPACE_MD))
        .child(shuffle(model, state, focus))
        .child(repeat(model, state, focus))
        .child(now_playing_toggle(model, state))
        .child(queue_toggle(model, state))
        .child(queue_position(state))
        .child(mute(model, state, focus))
        .child(volume(model, state))
}

fn shuffle(
    model: Stored<UiModel>,
    state: UiState,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    let active = move || state.playback.get().shuffle;
    ghost_active(
        palette,
        focus,
        Icon::Shuffle,
        focus::SHUFFLE,
        "shuffle",
        "Shuffle",
        active,
        move || {
            model.with(|m| {
                m.rt.ui_transport
                    .toggle_shuffle(&mut m.rt.playback.lock_or_recover());
            });
        },
    )
}

fn repeat(
    model: Stored<UiModel>,
    state: UiState,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    let active = move || state.playback.get().repeat != RepeatMode::None;
    let glyph = move || {
        if state.playback.get().repeat == RepeatMode::One {
            Icon::RepeatOne
        } else {
            Icon::Repeat
        }
    };
    let tint = move || {
        if active() {
            palette.get().brand_primary
        } else {
            palette.get().ink
        }
    };
    ring(
        view()
            .on_press(move || {
                model.with(|m| {
                    m.rt.ui_transport
                        .toggle_repeat(&mut m.rt.playback.lock_or_recover());
                });
            })
            .test_id("repeat")
            .label("Repeat")
            .p(px(tokens::SPACE_SM))
            .rounded(px(tokens::RADIUS_FULL))
            .hover(|s| s.bg(move || palette.get().row_hover))
            .child(dynamic(move || icon(glyph(), Icon::ICON_SIZE, tint))),
        focus::REPEAT,
        focus,
        palette,
    )
}

/// The Now Playing toggle: the egui bar's `ToggleExpanded`, flipped through
/// the session's view mode and mirrored to the nav signal the stage reads.
fn now_playing_toggle(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let open = move || state.nav.get().view_mode == riff_backend::app::state::ViewMode::NowPlaying;
    let glyph = move || if open() { Icon::Collapse } else { Icon::Expand };
    view()
        .on_press(move || {
            if open() {
                crate::screens::now_playing::close(model, state);
            } else {
                crate::screens::now_playing::open(model, state);
            }
        })
        .test_id("now-playing-toggle")
        .label(move || {
            if open() {
                "Close Now Playing"
            } else {
                "Now Playing"
            }
        })
        .p(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_FULL))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(dynamic(move || {
            icon(glyph(), Icon::ICON_SIZE, move || palette.get().ink)
        }))
}

/// The queue panel's toggle: the egui bar's `ToggleQueue`, opening the panel
/// above the bar.
fn queue_toggle(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let open = move || state.queue_open.get();
    let tint = move || {
        if open() {
            palette.get().brand_primary
        } else {
            palette.get().ink
        }
    };
    view()
        .on_press(move || toggle_queue(model))
        .test_id("queue-toggle")
        .label(move || if open() { "Close queue" } else { "Open queue" })
        .p(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_FULL))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(icon(Icon::ListMusic, Icon::ICON_SIZE, tint))
}

/// Flip the queue panel's flag: the session field plus the signal mirror.
fn toggle_queue(model: Stored<UiModel>) {
    let open = model.with(|m| m.state.queue_open);
    let next = model.with(|m| {
        let mut library = m.rt.library.lock_or_recover();
        library.queue_open = !library.queue_open;
        library.queue_open
    });
    open.set(next);
}

/// `"1/2"` — the egui bar's queue readout; nothing while the queue is
/// empty.
fn queue_position(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let line = move || {
        let view = state.playback.get();
        if view.queue_len > 0 {
            format!("{}/{}", view.queue_index + 1, view.queue_len)
        } else {
            String::new()
        }
    };
    text(line)
        .text_xs()
        .text_color(move || palette.get().ink_3)
        .test_id("queue-position")
}

fn mute(model: Stored<UiModel>, state: UiState, focus: Signal<Option<usize>>) -> impl IntoElement {
    let palette = state.palette;
    let muted = move || state.playback.get().muted;
    let glyph = move || {
        if muted() {
            Icon::VolumeMuted
        } else {
            Icon::VolumeHigh
        }
    };
    let tint = move || {
        if muted() {
            palette.get().brand_primary
        } else {
            palette.get().ink
        }
    };
    ring(
        view()
            .on_press(move || {
                model.with(|m| {
                    m.rt.ui_transport
                        .toggle_mute(&mut m.rt.playback.lock_or_recover());
                });
            })
            .test_id("volume-mute")
            .label("Mute")
            .p(px(tokens::SPACE_SM))
            .rounded(px(tokens::RADIUS_FULL))
            .hover(|s| s.bg(move || palette.get().row_hover))
            .child(dynamic(move || icon(glyph(), Icon::ICON_SIZE, tint))),
        focus::MUTE,
        focus,
        palette,
    )
}

/// The volume slider: `0..=1`, reading the session's `current_volume` —
/// which keeps its value while muted, so unmuting restores the level
/// (the egui bar's contract).
// Same step-quantized f64-to-f32 handoff as the seek slider above.
#[allow(clippy::cast_possible_truncation)]
fn volume(model: Stored<UiModel>, state: UiState) -> nsh_base::SliderElement {
    let palette = state.palette;
    let slider = slider()
        .test_id("volume")
        .accessibility_label("Volume")
        .value(move || Some(SliderValue::Single(f64::from(state.playback.get().volume))))
        .min(0.0)
        .max(1.0)
        .step(0.01)
        .w(px(tokens::COLUMN_MIN_W))
        .on_change(move |value: &SliderValue| {
            let level = value.start() as f32;
            model.with(|m| {
                m.rt.ui_transport
                    .set_volume(&mut m.rt.playback.lock_or_recover(), level);
            });
        });
    slider_parts(slider, palette)
}

/// A ghost whose glyph tints with the brand while `active` holds — the
/// shuffle case; the repeat and mute glyphs swap by mode and tint inline.
#[allow(clippy::too_many_arguments)]
fn ghost_active(
    palette: Signal<Palette>,
    focus: Signal<Option<usize>>,
    glyph: Icon,
    slot: usize,
    id: &'static str,
    label: &'static str,
    active: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let tint = move || {
        if active() {
            palette.get().brand_primary
        } else {
            palette.get().ink
        }
    };
    ring(
        view()
            .on_press(on_press)
            .test_id(id)
            .label(label)
            .p(px(tokens::SPACE_SM))
            .rounded(px(tokens::RADIUS_FULL))
            .hover(|s| s.bg(move || palette.get().row_hover))
            .child(icon(glyph, Icon::ICON_SIZE, tint)),
        slot,
        focus,
        palette,
    )
}

/// The bar's clock: whole seconds as `m:ss`, the egui seek row's readout
/// shape. `max(0.0)` bounds the value first, so the cast cannot truncate a
/// negative into a huge magnitude.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn mmss(secs: f32) -> String {
    let whole = secs.max(0.0).round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_minutes_seconds() {
        assert_eq!(mmss(0.0), "0:00");
        assert_eq!(mmss(74.4), "1:14");
        assert_eq!(mmss(-3.0), "0:00");
    }
}
