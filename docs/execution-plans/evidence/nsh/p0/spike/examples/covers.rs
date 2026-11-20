use std::time::{Duration, Instant};

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use riff_backend::app::traits::{DecodedCover, RequestedSize};
use riff_backend::composition::AppRuntime;
use riff_backend::domain::TrackId;

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

const BOX: u32 = 56;

struct CoverModel {
    rt: AppRuntime,
    arrived: Vec<ImageSrc>,
    started: Option<Instant>,
    load_ms: Option<u128>,
}

fn store_path() -> std::path::PathBuf {
    std::env::var_os("RIFF_SPIKE_STORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../../.scratch/nsh-p0/store/riff.sqlite3")
        })
}

fn fixture_root() -> std::path::PathBuf {
    std::env::var_os("RIFF_SPIKE_LIBRARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../../.scratch/nsh-p0/library")
        })
}

fn album_dirs(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(artists) = std::fs::read_dir(root) else { return out };
    for a in artists.flatten() {
        let Ok(albums) = std::fs::read_dir(a.path()) else { continue };
        for al in albums.flatten() {
            if al.path().is_dir() {
                out.push(al.path());
            }
        }
    }
    out.sort();
    out
}

#[component]
pub fn CoverApp() -> impl IntoElement {
    let (rt, _life) = AppRuntime::spawn(&store_path()).expect("spawn runtime");
    let dirs = album_dirs(&fixture_root());
    let model = stored(CoverModel {
        rt,
        arrived: Vec::new(),
        started: Some(Instant::now()),
        load_ms: None,
    });
    let count = signal(0_usize);
    let total = dirs.len();

    for dir in &dirs {
        model.with(|m| {
            m.rt.covers
                .request_folder(dir, RequestedSize { width: BOX, height: BOX });
        });
    }

    set_interval(Duration::from_millis(16), move || {
        model.with(|m| {
            let batch: Vec<(TrackId, RequestedSize, Option<DecodedCover>)> = m.rt.covers.poll();
            for (_id, _size, cover) in batch {
                if let Some(c) = cover {
                    m.arrived.push(ImageSrc::rgba(c.width, c.height, c.rgba));
                }
            }
            if m.arrived.len() >= total && m.load_ms.is_none() {
                m.load_ms = m.started.map(|s| s.elapsed().as_millis());
            }
            count.set(m.arrived.len());
        });
    });

    view()
        .flex_1()
        .flex_col()
        .gap_2()
        .p(px(8.0))
        .child(
            text(move || {
                model.with(|m| {
                    format!(
                        "covers {}/{} load_ms {:?}",
                        count.get(),
                        total,
                        m.load_ms
                    )
                })
            })
            .test_id("cover-readout"),
        )
        .child(
            view()
                .flex_1()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .child(dynamic(move || {
                    count.get();
                    model.with(|m| {
                        m.arrived
                            .iter()
                            .map(|src| image(src.clone()).w(px(BOX as f32)).h(px(BOX as f32)))
                            .collect::<Vec<_>>()
                    })
                })),
        )
}

fn main() -> impl std::process::Termination {
    env_logger::init();
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<CoverApp>()),
        platform::AppOptions {
            title: "riff cover proof".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "io.github.strawberry-choco.riff".to_owned(),
            size: (1000, 800),
            ..Default::default()
        },
    )
}