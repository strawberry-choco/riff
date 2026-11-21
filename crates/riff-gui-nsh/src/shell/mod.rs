//! The app shell: the port of the egui frontend's chrome — `ui/frame.rs`'s
//! layout, `ui/chrome.rs`'s titlebar, `ui/sidebar.rs`'s nav panel and
//! `ui/playerbar.rs`'s control bar — onto the nsh element tree. Every color
//! reads the palette signal and every dimension the token store (ADR 0004);
//! the transport intents dispatch through the model's `rt.ui_transport` and
//! write no signal (the pump is the one writer).

pub mod layout;
pub mod player_bar;
pub mod sidebar;
pub mod title_bar;

pub use layout::install;

use nsh_core::prelude::*;

use crate::theme::Palette;
use crate::theme::tokens;

/// The arrow focus ring: the shell's interactive elements in paint order.
/// The search field, the player bar's transport cluster and its toggles —
/// not the seek and volume sliders (their own arrow keys nudge their value
/// once really focused, and a ring slot there would move twice per press),
/// not the window controls (the platform's own buttons), and not the
/// sidebar rows (inert until N5 wires their selection).
pub(crate) mod focus {
    pub(crate) const SEARCH: usize = 0;
    pub(crate) const PREVIOUS: usize = 1;
    pub(crate) const PLAY: usize = 2;
    pub(crate) const NEXT: usize = 3;
    pub(crate) const MUTE: usize = 4;
    pub(crate) const SHUFFLE: usize = 5;
    pub(crate) const REPEAT: usize = 6;
    pub(crate) const COUNT: usize = 7;
}

/// Steps the ring by `delta`, wrapping through every slot; a cleared ring
/// enters at the search field whichever way it steps.
pub(crate) fn step(from: Option<usize>, delta: i32) -> usize {
    let Some(at) = from else {
        return focus::SEARCH;
    };
    let count = i32::try_from(focus::COUNT).expect("the ring's slot count fits an i32");
    let at = i32::try_from(at).expect("a ring slot fits an i32");
    usize::try_from((at + delta).rem_euclid(count)).expect("rem_euclid is non-negative")
}

/// The ring's one treatment: a hairline border the palette's focus ring
/// paints while the arrow focus names `slot`, invisible otherwise — so the
/// border's box is reserved at every moment and focus never shifts layout.
pub(crate) fn ring<E: nsh_core::element::Interactive + nsh_core::element::Styled>(
    element: E,
    slot: usize,
    focus: Signal<Option<usize>>,
    palette: Signal<Palette>,
) -> E {
    element
        .project_focused(move || focus.get() == Some(slot))
        .border(px(1.0))
        .rounded(px(tokens::RADIUS_SM))
        .border_color(move || {
            if focus.get() == Some(slot) {
                palette.get().focus_ring
            } else {
                transparent_black()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_wraps_through_every_slot_in_both_directions() {
        assert_eq!(step(None, 1), focus::SEARCH);
        assert_eq!(step(Some(focus::REPEAT), 1), focus::SEARCH);
        assert_eq!(step(Some(focus::SEARCH), -1), focus::REPEAT);
    }
}
