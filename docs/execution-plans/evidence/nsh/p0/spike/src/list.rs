use nsh_base::button;
use nsh_core::prelude::*;
use riff_backend::app::state::TrackSort;
use riff_backend::composition::AppRuntime;

pub struct ListModel {
    pub rt: AppRuntime,
}

pub fn store_path() -> std::path::PathBuf {
    std::env::var_os("RIFF_SPIKE_STORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../../.scratch/nsh-p0/store/riff.sqlite3")
        })
}

#[component]
pub fn ListApp() -> impl IntoElement {
    let (rt, _life) = AppRuntime::spawn(&store_path()).expect("spawn runtime");
    let model = stored(ListModel { rt });
    let selected = signal(usize::MAX);
    let list = element_ref();

    let jump = list.clone();
    let count_model = model;

    view()
        .flex_1()
        .flex_col()
        .child(
            view()
                .flex_row()
                .gap_2()
                .p(px(8.0))
                .child(text(move || format!("selected {}", selected.get())).test_id("selected"))
                .child(
                    button()
                        .test_id("jump")
                        .on_press(move || jump.scroll_to_index(40000, percentage(0.5), px(0.0), false))
                        .child(text("Jump 40000")),
                ),
        )
        .child(
            virtual_list(
                move || count_model.with(|m| m.rt.session_views.track_count("", TrackSort::NumberAsc)),
                move |at| {
                    let m = model;
                    let sel = selected;
                    view()
                        .h(px(44.0))
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px(px(8.0))
                        .bg(move || {
                            if sel.get() == at.index() {
                                nsh_base::theme().accent
                            } else {
                                nsh_base::theme().background
                            }
                        })
                        .on_press(move || sel.set(at.index()))
                        .child(text(move || format!("{}", at.index())).test_id("row-index"))
                        .child(
                            text(move || {
                                let i = at.index();
                                m.with(|mm| {
                                    mm.rt
                                        .session_views
                                        .track_row("", TrackSort::NumberAsc, i)
                                        .map(|t| t.metadata.title.clone().unwrap_or_default())
                                        .unwrap_or_else(|| "?".to_string())
                                })
                            })
                            .test_id("row-title"),
                        )
                },
            )
            .estimated_item_size(px(44.0))
            .flex_1()
            .element_ref(&list)
            .test_id("track-list"),
        )
}