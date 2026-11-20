use nsh_base::button;
use nsh_core::prelude::*;

#[component]
pub fn InteractiveApp() -> impl IntoElement {
    let mode = signal("dark");
    let keys = signal(String::new());
    let focus = signal(String::new());
    let dropped = signal(0_usize);
    let index = signal(0_i32);

    on_key("mod+k", move |_| focus.set("search".to_string()));
    on_key("escape", move |_| focus.set(String::new()));
    on_key("space", move |_| keys.update(|k| k.push_str("space ")));
    on_key("down", move |_| index.update(|i| *i += 1));

    let toggle = mode;

    view()
        .flex_1()
        .flex_col()
        .gap_2()
        .p(px(16.0))
        .test_id("root")
        .bg(move || {
            if mode.get() == "dark" {
                nsh_base::theme().background
            } else {
                nsh_base::theme().primary
            }
        })
        .on_file_drop(move |event: &nsh_core::element::FileDropEvent| {
            dropped.set(event.uris.len());
        })
        .child(text(move || format!("mode {}", mode.get())).test_id("mode"))
        .child(text(move || format!("focus [{}]", focus.get())).test_id("focus"))
        .child(text(move || format!("keys [{}]", keys.get())).test_id("keys"))
        .child(text(move || format!("index {}", index.get())).test_id("index"))
        .child(text(move || format!("dropped {}", dropped.get())).test_id("dropped"))
        .child(
            button()
                .test_id("toggle")
                .on_press(move || {
                    toggle.update(|m| *m = if *m == "dark" { "light" } else { "dark" });
                })
                .child(text("Toggle palette")),
        )
}