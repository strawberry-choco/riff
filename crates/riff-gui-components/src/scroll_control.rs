//! The widget half of the scroll protocol: the [`ScrollControl`] the
//! scroll-area wrapper ([`super::scroll_area`]) renders under. The struct is plain
//! data, and the `.animated(false)` rationale — the WHY of the builder set the
//! wrapper applies is written on the struct itself; the wrapper is the HOW.

/// Scroll handling for one list column this frame, as the widgets' seam:
/// the stable per-slot salt that keys the column's egui `ScrollArea` state
/// and the offset to start the frame at (the Scroll Memory's saved value, or
/// 0 on a reset). `start: None` leaves egui's own state untouched — a drill
/// column between selection changes.
///
/// # Why the columns that take a `ScrollControl` also pass `.animated(false)`
///
/// A list column handed a `ScrollControl` names the offset it wants *this
/// frame*, either by restoring a remembered position or by forcing the top
/// after a selection change. In egui 0.35 `ScrollArea::animated` is consulted
/// in exactly one place — the `scroll_to_*` / `scroll_to_rect` branch, where
/// the area reconciles a programmatic offset against what is on screen
/// (`containers/scroll_area.rs`) — and the columns that take a `ScrollControl`
/// reach that branch through `vertical_scroll_offset` / `show_rows`' own
/// keep-in-view adjustment. So the flag governs **programmatic scroll-to
/// offsets, not wheel feel**: wheel and drag inertia are unaffected by it, and
/// the eight areas that leave it on are not feeling sluggish.
///
/// It is off here because a keep-in-view jump must not lerp. The user asked
/// for a row, not for a glide toward it, and an offset the module is
/// restoring is a *fact about where the list is*, not a preference — easing a
/// value the Scroll Memory is meant to be authoritative about would leave the
/// list visibly disagreeing with its own record for the length of the tween.
/// The alternative considered and rejected was flipping the other eight areas
/// off as well: it would have been a behaviour change spread over five more
/// surfaces, and on the evidence above it would buy nothing, because those
/// areas do not take a `ScrollControl` and so do not drive a programmatic
/// offset from here. The inconsistency is real; the fix is this paragraph, not
/// the other eight call sites.
#[derive(Debug, Clone, Copy)]
pub struct ScrollControl {
    pub salt: &'static str,
    pub start: Option<f32>,
}
