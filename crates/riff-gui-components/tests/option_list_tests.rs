//! Inline option-list selector probes (Smart Playlists ticket 06).
//!
//! The selector is the component library's one generic single-choice control:
//! fed `(label, key)` pairs, the current key and an identity, it draws its
//! options **inline in the pane's own layout flow** — no popup, no new layer —
//! and reports the clicked key to the caller. These probes are the typed-intent
//! seam: every test clicks a real option through the a11y label and asserts the
//! key that arrives in the caller's buffer, never a pixel. The golden that
//! proves the same click through a composed frame lives in the root
//! `riff-tests` crate (`golden_tests::…::option_list_after_click_dark`).
//!
//! Moved here from the root suite's pattern, per the widgetry split: the
//! primitive lives in this crate, so its probes do too.

// The probes keep the root suite's test scaffolding shapes (long local
// fixture functions, closure-heavy harness builders) while this crate keeps
// `[lints] workspace = true`; the file-level opt-out scopes the root
// `riff-tests` crate's documented manifest-level exemption to just the
// files that need it.
#![allow(clippy::pedantic)]

use egui_kittest::kittest::{NodeT, Queryable};
use riff_gui_components::option_list::{OptionChoice, OptionList, option_list};
use riff_gui_theme::theme::Palette;

/// The three options every single-list fixture offers.
fn choices() -> Vec<OptionChoice<'static, String>> {
    [("One", "one"), ("Two", "two"), ("Three", "three")]
        .into_iter()
        .map(|(label, key)| OptionChoice {
            key: key.to_string(),
            label,
        })
        .collect()
}

/// Render the selector seam over a caller-owned current-key buffer at
/// `size`, with `initial` chosen. The draw closure writes the key the
/// selector reports straight into the harness state — the caller-owned
/// buffer the props-in / typed-out contract promises to mutate nothing but.
fn selector_harness(
    size: egui::Vec2,
    initial: Option<&str>,
) -> egui_kittest::Harness<'static, Option<String>> {
    let palette = Palette::dark();
    let options = choices();
    // The draw closure MOVES its fixtures in, so it is `'static` and the
    // harness it builds owns everything it reads.
    egui_kittest::Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, current: &mut Option<String>| {
                let reported = option_list(
                    ui,
                    &palette,
                    &OptionList {
                        id: egui::Id::new("fixture_selector"),
                        options: &options,
                        current: current.as_ref(),
                    },
                );
                if let Some(key) = reported {
                    *current = Some(key);
                }
            },
            initial.map(str::to_string),
        )
}

/// A click on an option reports that option's key into the caller's buffer,
/// and a second click moves the choice — the typed-intent contract the rule
/// editor's field, comparison and value pickers (tickets 14/15/20) will pin
/// their drafts on.
#[test]
fn test_clicking_an_option_reports_its_key_and_a_later_click_moves_the_choice() {
    let mut harness = selector_harness(egui::vec2(320.0, 200.0), Some("one"));
    harness.run();
    assert_eq!(harness.state().as_deref(), Some("one"));

    harness.get_by_label("Two").click();
    harness.run();
    assert_eq!(
        harness.state().as_deref(),
        Some("two"),
        "a real click on an inline option reports its key"
    );

    harness.get_by_label("Three").click();
    harness.run();
    assert_eq!(
        harness.state().as_deref(),
        Some("three"),
        "the choice moves on the next click, inline, with no popup to open first"
    );
}

/// The current choice carries its mark in the accessibility tree — toggled
/// on as the radio of the single-choice group, others toggled off — because
/// the harness and assistive tech both address the control by label and
/// must be able to read which label is in force. (An egui fact pinned here:
/// `WidgetInfo::selected` lands in AccessKit as `Toggled`, not as the
/// `selected` state — `response.rs:955` in egui 0.35.)
#[test]
fn test_the_current_choice_is_marked_in_the_a11y_tree() {
    use egui::accesskit::Toggled;

    let mut harness = selector_harness(egui::vec2(320.0, 200.0), Some("two"));
    harness.run();

    let current = harness
        .query_by_label("Two")
        .expect("the current option is addressable by its label");
    assert_eq!(
        current.accesskit_node().toggled(),
        Some(Toggled::True),
        "the current option reads as the chosen radio in the a11y tree"
    );
    for label in ["One", "Three"] {
        let other = harness
            .query_by_label(label)
            .unwrap_or_else(|| panic!("{label} is addressable by its label"));
        assert_eq!(
            other.accesskit_node().toggled(),
            Some(Toggled::False),
            "{label} is not the current choice and must not read chosen"
        );
    }
}

/// With nothing chosen yet (the draft-editor case), Tab reaches the first
/// option and Enter selects it; a second Tab moves the focus, not the
/// selection — the control is keyboard-reachable end to end, not only
/// mouse-reachable.
#[test]
fn test_options_are_tab_reachable_and_enter_selects_the_focused_option() {
    let mut harness = selector_harness(egui::vec2(320.0, 200.0), None);
    harness.run();
    assert_eq!(harness.state(), &None, "the draft starts with no choice");

    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(
        harness
            .query_by_label("One")
            .is_some_and(|node| node.is_focused()),
        "the first Tab focuses the first option"
    );

    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(
        harness.state().as_deref(),
        Some("one"),
        "Enter on the focused option selects it — keyboard parity with the click"
    );

    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(
        harness
            .query_by_label("Two")
            .is_some_and(|node| node.is_focused()),
        "Tab moves the focus to the next option..."
    );
    assert_eq!(
        harness.state().as_deref(),
        Some("one"),
        "...without moving the selection"
    );
}

/// Two selectors live in one pane: SAME keys, different labels, different
/// identities — each answers only its own clicks, and a row id collision
/// (which is exactly what a shared `egui::Id` would cause, since row ids
/// derive from the instance id plus the key) would surface as an egui panic.
/// A generic selector that cannot host two instances is useless to the rule
/// editor, where a row shows a field picker and an operator picker beside
/// each other over one key vocabulary.
#[test]
fn test_two_selectors_in_one_pane_keep_their_own_identity() {
    let palette = Palette::dark();
    let labelled = |prefix: &str| -> Vec<OptionChoice<'static, String>> {
        choices()
            .into_iter()
            .map(|choice| OptionChoice {
                key: choice.key,
                label: Box::leak(format!("{prefix} {}", choice.label).into_boxed_str()),
            })
            .collect()
    };
    let left = labelled("L");
    let right = labelled("R");
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(480.0, 200.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, picks: &mut Vec<String>| {
                ui.horizontal(|ui| {
                    for (name, options) in [("left", &left), ("right", &right)] {
                        ui.vertical(|ui| {
                            if let Some(key) = option_list(
                                ui,
                                &palette,
                                &OptionList {
                                    id: egui::Id::new(("two_selectors", name)),
                                    options,
                                    current: None,
                                },
                            ) {
                                picks.push(format!("{name}/{key}"));
                            }
                        });
                    }
                });
            },
            Vec::new(),
        );
    harness.run();

    harness.get_by_label("R Two").click();
    harness.run();
    assert_eq!(
        harness.state().as_slice(),
        &["right/two".to_string()],
        "a click lands in exactly the selector whose row was clicked"
    );

    harness.get_by_label("L One").click();
    harness.run();
    assert_eq!(
        harness.state().as_slice(),
        &["right/two".to_string(), "left/one".to_string()],
        "the other instance keeps its own identity: its click reports its own source and key"
    );
}

/// Overflow is defined, not accidental: the list never escapes the pane
/// horizontally — a too-long label is truncated to the row's text budget —
/// and its vertical growth is absorbed by the host pane's scroll. A deep
/// list's bottom option stays reachable: scroll to it inside the pane, then
/// click it, and the key arrives.
#[test]
fn test_a_long_list_scrolls_within_the_host_pane_and_long_labels_truncate() {
    let palette = Palette::dark();
    const DEPTH: usize = 24;
    let options: Vec<OptionChoice<'_, String>> = (0..DEPTH)
        .map(|index| {
            let label = if index == 7 {
                // A label no pane will ever fit on one line — the selector
                // must truncate it, never let it widen or escape the row.
                "An absurdly long library value that would certainly escape this pane if the selector let it".to_string()
            } else {
                format!("Option {index}")
            };
            OptionChoice {
                key: format!("k{index}"),
                label: Box::leak(label.into_boxed_str()),
            }
        })
        .collect();
    let size = egui::vec2(220.0, 160.0);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, current: &mut Option<String>| {
                egui::ScrollArea::vertical()
                    .id_salt("fixture_pane")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(key) = option_list(
                            ui,
                            &palette,
                            &OptionList {
                                id: egui::Id::new("fixture_overflow_selector"),
                                options: &options,
                                current: current.as_ref(),
                            },
                        ) {
                            *current = Some(key);
                        }
                    });
            },
            Some("k0".to_string()),
        );
    harness.run();

    // Nothing escapes: every option's rect sits inside the pane's width.
    let last = format!("Option {}", DEPTH - 1);
    for label in [
        "Option 0",
        "An absurdly long library value that would certainly escape this pane if the selector let it",
        &last,
    ] {
        let node = harness
            .query_by_label(label)
            .unwrap_or_else(|| panic!("{label} is addressable by its label"));
        let rect = node.rect();
        assert!(
            rect.min.x >= 0.0 && rect.max.x <= size.x,
            "the row for {label} escapes the pane horizontally: {rect:?} in a {} wide frame",
            size.x,
        );
    }

    // The deep tail is reachable: the pane's own scroll absorbs the list's
    // vertical growth. A real wheel over the pane brings the tail on screen,
    // and from there a real click selects it like any other option.
    harness.hover_at(egui::pos2(size.x / 2.0, size.y / 2.0));
    let mut on_screen = false;
    for _ in 0..32 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        let rect = harness.get_by_label(&last).rect();
        if rect.min.y >= 0.0 && rect.max.y <= size.y {
            on_screen = true;
            break;
        }
    }
    assert!(
        on_screen,
        "the host pane's scroll brings the tail of a long list on screen — \
         the list grew past the pane's height and nothing escaped it",
    );
    harness.get_by_label(&last).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().as_deref(),
        Some("k23"),
        "scrolling within the host pane keeps even the tail of a long list clickable"
    );
}
