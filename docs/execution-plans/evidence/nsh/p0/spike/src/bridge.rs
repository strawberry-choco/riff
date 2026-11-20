use std::time::Duration;

use nsh_base::button;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::composition::AppRuntime;

#[derive(Clone, PartialEq, Default)]
pub struct PlaybackView {
    pub state: &'static str,
    pub position_secs: f32,
    pub duration_secs: Option<f32>,
    pub volume: f32,
    pub muted: bool,
    pub queue_len: usize,
    pub queue_index: Option<usize>,
}

pub struct UiModel {
    pub rt: AppRuntime,
    pub playback: Signal<PlaybackView>,
    pub ticks: Signal<u64>,
    pub presses: Signal<u32>,
}

pub fn store_path() -> std::path::PathBuf {
    let p = std::env::var_os("RIFF_SPIKE_STORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../../.scratch/nsh-p0/store/riff.sqlite3")
        });
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("create store dir");
    }
    p
}

fn snapshot(rt: &AppRuntime) -> PlaybackView {
    let s = rt.playback.lock_or_recover();
    PlaybackView {
        state: match s.playback_state {
            riff_playback::domain::PlaybackState::Playing => "playing",
            riff_playback::domain::PlaybackState::Paused => "paused",
            riff_playback::domain::PlaybackState::Stopped => "stopped",
        },
        position_secs: s.current_position.current.as_secs_f32(),
        duration_secs: s.current_position.total.map(|d| d.as_secs_f32()),
        volume: s.current_volume,
        muted: s.muted,
        queue_len: s.queue.tracks.len(),
        queue_index: s.queue.current_index,
    }
}

#[component]
pub fn BridgeApp() -> impl IntoElement {
    let (rt, _lifecycle) = AppRuntime::spawn(&store_path()).expect("spawn runtime");
    let model = stored(UiModel {
        rt,
        playback: signal(PlaybackView::default()),
        ticks: signal(0_u64),
        presses: signal(0_u32),
    });

    let playback_sig = model.with(|m| m.playback);
    let ticks_sig = model.with(|m| m.ticks);
    let presses_sig = model.with(|m| m.presses);

    // Signal writes happen outside the stored borrow: a write notifies bindings
    // that read the same store, which would re-borrow the RefCell mid-update.
    set_interval(Duration::from_millis(16), move || {
        let snap = model.with(|m| {
            let snap = snapshot(&m.rt);
            let queue = m.rt.playback.lock_or_recover().queue.clone();
            m.rt.session_views.sync_playback(&queue, 8);
            let _ = m.rt.backend_events.lock_or_recover().events();
            snap
        });
        playback_sig.set(snap);
        ticks_sig.update(|t| *t += 1);
    });

    let play = model;
    let pause = model;
    let next = model;
    let seek = model;

    view()
        .flex_1()
        .flex_col()
        .gap_2()
        .p(px(16.0))
        .child(
            text(move || {
                model.with(|m| format!("ticks {} presses {}", m.ticks.get(), m.presses.get()))
            })
            .test_id("ticks"),
        )
        .child(
            text(move || {
                model.with(|m| {
                    let v = m.playback.get();
                    format!(
                        "{} pos {:.1} dur {:?} vol {:.2} muted {} queue {} at {:?}",
                        v.state,
                        v.position_secs,
                        v.duration_secs,
                        v.volume,
                        v.muted,
                        v.queue_len,
                        v.queue_index
                    )
                })
            })
            .test_id("playback"),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    button()
                        .test_id("play")
                        .on_press(move || {
                            presses_sig.update(|p| *p += 1);
                            play.with(|m| m.rt.ui_transport.play_random());
                        })
                        .child(text("Play random")),
                )
                .child(
                    button()
                        .test_id("playpause")
                        .on_press(move || {
                            pause.with(|m| {
                                let s = m.rt.playback.lock_or_recover();
                                m.rt.ui_transport.play_pause(&s);
                            })
                        })
                        .child(text("Play/Pause")),
                )
                .child(
                    button()
                        .test_id("next")
                        .on_press(move || next.with(|m| m.rt.ui_transport.next()))
                        .child(text("Next")),
                )
                .child(
                    button()
                        .test_id("seek")
                        .on_press(move || {
                            seek.with(|m| {
                                let s = m.rt.playback.lock_or_recover();
                                m.rt.ui_transport.seek(&s, 1.0);
                            })
                        })
                        .child(text("Seek 1s")),
                ),
        )
}