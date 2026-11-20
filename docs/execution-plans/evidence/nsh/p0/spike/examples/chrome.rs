use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes, WindowControl};
use nsh_core::element::desktop::{current_window, title_bar};
use nsh_core::prelude::*;

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

fn window_bar() -> impl IntoElement {
    let mut bar = TitleBarOptions::default();
    bar.style = TitleBarStyle::Hidden;
    bar.height = Some(px(56.0));
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(bar);
    current_window().set(attributes);

    view()
        .h(px(56.0))
        .flex_row()
        .items_center()
        .window_drag(true)
        .bg(move || nsh_base::theme().primary)
        .pl(move || title_bar().left + px(12.0))
        .pr(move || title_bar().right + px(12.0))
        .child(
            text("riff")
                .flex_1()
                .text_color(move || nsh_base::theme().primary_foreground)
                .test_id("app-name"),
        )
        .child(
            text(move || {
                let t = title_bar();
                format!("top {:?} left {:?} right {:?}", t.top, t.left, t.right)
            })
            .text_color(move || nsh_base::theme().primary_foreground)
            .test_id("bar-widths"),
        )
        .child(
            text("Min")
                .window_control(WindowControl::Minimize)
                .text_color(move || nsh_base::theme().primary_foreground)
                .test_id("min"),
        )
        .child(
            text("Max")
                .window_control(WindowControl::Maximize)
                .text_color(move || nsh_base::theme().primary_foreground)
                .test_id("max"),
        )
        .child(
            text("Close")
                .window_control(WindowControl::Close)
                .text_color(move || nsh_base::theme().primary_foreground)
                .test_id("close"),
        )
}

#[component]
pub fn ChromeApp() -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .child(window_bar())
        .child(
            view()
                .flex_1()
                .items_center()
                .justify_center()
                .child(text("chrome proof").test_id("body")),
        )
}

fn main() -> impl std::process::Termination {
    env_logger::init();
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<ChromeApp>()),
        platform::AppOptions {
            title: "riff chrome proof".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "io.github.strawberry-choco.riff".to_owned(),
            size: (900, 600),
            ..Default::default()
        },
    )
}