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

/// The arrow focus ring's chrome slots, in paint order. The search field,
/// the player bar's transport cluster and its toggles — not the seek and
/// volume sliders (their own arrow keys nudge their value once really
/// focused, and a ring slot there would move twice per press) and not the
/// window controls (the platform's own buttons). The sidebar's rows are the
/// ring's other segment ([`Slot::Sidebar`]); the key map walks both as one
/// sequence.
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

/// One arrow-traversal position: a [`focus`] chrome slot, a sidebar row
/// past the chrome slots (the row's index in the sidebar's display order,
/// which runs LIBRARY, SMART LISTS, PLAYLISTS), or a track-listing row past
/// the sidebar (its index in the listing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Chrome(usize),
    Sidebar(usize),
    List(usize),
}

/// Steps the traversal by `delta`, wrapping through the chrome slots, the
/// `sidebar_rows` sidebar rows and the `list_rows` listing rows as one
/// sequence; a cleared traversal enters at the search field whichever way
/// it steps. Both row counts are the caller's reads (they move with the
/// fold, the playlists and the listing), so a wrap out of a shrunken
/// segment lands in the next one.
pub(crate) fn step(at: Option<Slot>, delta: i32, sidebar_rows: usize, list_rows: usize) -> Slot {
    let chrome = i32::try_from(focus::COUNT).expect("the ring's slot count fits an i32");
    let sidebar = i32::try_from(sidebar_rows).unwrap_or(i32::MAX - chrome);
    let list = i32::try_from(list_rows).unwrap_or(i32::MAX - chrome - sidebar);
    let flat = match at {
        None => return Slot::Chrome(focus::SEARCH),
        Some(Slot::Chrome(at)) => i32::try_from(at).expect("a ring slot fits an i32"),
        Some(Slot::Sidebar(at)) => chrome + i32::try_from(at).expect("a row index fits an i32"),
        Some(Slot::List(at)) => {
            chrome + sidebar + i32::try_from(at).expect("a row index fits an i32")
        }
    };
    let next = flat
        .saturating_add(delta)
        .rem_euclid(chrome + sidebar + list);
    if next < chrome {
        Slot::Chrome(usize::try_from(next).expect("rem_euclid is non-negative"))
    } else if next < chrome + sidebar {
        Slot::Sidebar(
            usize::try_from(next - chrome).expect("the remainder is past the chrome slots"),
        )
    } else {
        Slot::List(
            usize::try_from(next - chrome - sidebar)
                .expect("the remainder is past the chrome and sidebar slots"),
        )
    }
}

/// The ring's one treatment: a hairline border the palette's focus ring
/// paints while `focused` holds, invisible otherwise — so the border's box
/// is reserved at every moment and focus never shifts layout. The test's
/// closure may capture a non-`Copy` handle (a row's facts closure), hence
/// `Clone`.
pub(crate) fn ring_when<E: nsh_core::element::Interactive + nsh_core::element::Styled>(
    element: E,
    focused: impl Fn() -> bool + Clone + 'static,
    palette: Signal<Palette>,
) -> E {
    let focused = move || focused();
    element
        .project_focused(focused.clone())
        .border(px(tokens::BORDER_HAIRLINE))
        .rounded(px(tokens::RADIUS_SM))
        .border_color(move || {
            if focused() {
                palette.get().focus_ring
            } else {
                transparent_black()
            }
        })
}

/// The chrome ring's paint: focused while the arrow focus names `slot`.
pub(crate) fn ring<E: nsh_core::element::Interactive + nsh_core::element::Styled>(
    element: E,
    slot: usize,
    focus: Signal<Option<usize>>,
    palette: Signal<Palette>,
) -> E {
    ring_when(element, move || focus.get() == Some(slot), palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_wraps_through_every_slot_in_both_directions() {
        assert_eq!(step(None, 1, 0, 0), Slot::Chrome(focus::SEARCH));
        assert_eq!(
            step(Some(Slot::Chrome(focus::REPEAT)), 1, 0, 0),
            Slot::Chrome(focus::SEARCH)
        );
        assert_eq!(
            step(Some(Slot::Chrome(focus::SEARCH)), -1, 0, 0),
            Slot::Chrome(focus::REPEAT)
        );
    }

    #[test]
    fn the_traversal_walks_chrome_into_the_sidebar_and_back() {
        let rows = 3;
        assert_eq!(
            step(Some(Slot::Chrome(focus::REPEAT)), 1, rows, 0),
            Slot::Sidebar(0)
        );
        assert_eq!(
            step(Some(Slot::Sidebar(2)), 1, rows, 0),
            Slot::Chrome(focus::SEARCH)
        );
        assert_eq!(
            step(Some(Slot::Chrome(focus::SEARCH)), -1, rows, 0),
            Slot::Sidebar(2)
        );
        assert_eq!(
            step(Some(Slot::Sidebar(0)), -1, rows, 0),
            Slot::Chrome(focus::REPEAT)
        );
    }

    #[test]
    fn the_traversal_walks_the_sidebar_into_the_list_and_back() {
        assert_eq!(step(Some(Slot::Sidebar(1)), 1, 2, 4), Slot::List(0));
        assert_eq!(
            step(Some(Slot::List(3)), 1, 2, 4),
            Slot::Chrome(focus::SEARCH)
        );
        assert_eq!(
            step(Some(Slot::Chrome(focus::SEARCH)), -1, 2, 4),
            Slot::List(3)
        );
        assert_eq!(step(Some(Slot::List(0)), -1, 2, 4), Slot::Sidebar(1));
        assert_eq!(step(Some(Slot::List(1)), 1, 2, 4), Slot::List(2));
    }

    #[test]
    fn a_folded_sidebar_leaves_the_chrome_ring_intact() {
        assert_eq!(
            step(Some(Slot::Chrome(focus::REPEAT)), 1, 0, 0),
            Slot::Chrome(focus::SEARCH)
        );
    }

    #[test]
    fn an_empty_listing_has_no_list_segment() {
        assert_eq!(
            step(Some(Slot::Sidebar(0)), 1, 1, 0),
            Slot::Chrome(focus::SEARCH)
        );
        assert_eq!(
            step(Some(Slot::Chrome(focus::SEARCH)), -1, 1, 0),
            Slot::Sidebar(0)
        );
    }
}
