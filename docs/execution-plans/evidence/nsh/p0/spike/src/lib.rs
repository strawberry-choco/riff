use nsh_base::button;
use nsh_core::prelude::*;

pub mod bridge;
pub mod list;
pub mod interactive;

#[native_module("riff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
    let count = signal(0_u32);
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .child(text(move || format!("Count: {}", count.get())).test_id("count"))
        .child(
            button()
                .test_id("tap")
                .on_press(move || count.update(|c| *c += 1))
                .child(text("Tap")),
        )
}