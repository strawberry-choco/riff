//! The display→business transforms at the menu boundary, asserted at their own
//! seam.
//!
//! The Track menu's vocabulary belongs to the component crate
//! (`riff_gui_components::menu`): its [`SortKey`], [`TrackKey`] and
//! [`PlaylistKey`] are display words, not the session's business types. The
//! frontend owns every crossing, and the direction that must be **total** is
//! display → business — with no wildcard arm, so the component crate can never
//! emit an intent the host cannot answer. The business → display direction is
//! deliberately partial: a business sort mode no surface displays maps to
//! `None`, and adding one must not force a UI change.
//!
//! These assertions are the price the mirror was chosen with: a business
//! option that *should* be displayed earns no compile error, so the totality
//! half is what a test must pin. The transforms themselves carry the other
//! half structurally — a new `SortKey` variant without a business arm fails
//! to compile at `sort_key_to_track_sort`, because no wildcard arm exists
//! there to absorb it.

use riff_backend::app::state::TrackSort;
use riff_backend::domain::{PlaylistId, TrackId};
use riff_gui::ui::app::track_menu::{
    playlist_id_to_key, playlist_key_to_id, track_id_to_key, track_key_to_id,
};
use riff_gui::ui::browser::{sort_key_to_track_sort, track_sort_to_sort_key};
use riff_gui_components::menu::{PlaylistKey, SortKey, TrackKey};

#[test]
fn every_display_sort_member_maps_to_a_business_member() {
    // The transform is a wildcard-free match, so this enumeration is the one
    // place a display member without a business counterpart can still slip
    // through — which is why it names every member the display enum has: a
    // member added to `SortKey` must be answered HERE and at the transform,
    // or the build breaks at the transform first.
    for (display, business) in [
        (SortKey::NumberAsc, TrackSort::NumberAsc),
        (SortKey::NumberDesc, TrackSort::NumberDesc),
        (SortKey::TitleAsc, TrackSort::TitleAsc),
        (SortKey::TitleDesc, TrackSort::TitleDesc),
    ] {
        assert_eq!(
            sort_key_to_track_sort(display),
            business,
            "a display sort member must name exactly its business counterpart"
        );
    }
}

#[test]
fn every_business_sort_mode_displayed_today_round_trips_through_the_display_key() {
    // The business → display direction is deliberately partial, so this
    // asserts today's fact — every mode the session has is one a surface
    // displays — rather than a rule. If a mode nobody displays is added to
    // `TrackSort`, this test's enumeration must be extended with the honest
    // expectation (`None`), not the transform widened.
    for sort in [
        TrackSort::NumberAsc,
        TrackSort::NumberDesc,
        TrackSort::TitleAsc,
        TrackSort::TitleDesc,
    ] {
        assert_eq!(
            track_sort_to_sort_key(sort).map(sort_key_to_track_sort),
            Some(sort),
            "a displayed business mode must survive the round trip unchanged"
        );
    }
}

#[test]
fn track_identity_keys_round_trip_across_the_display_boundary() {
    let track = TrackId("music/album/01.flac".to_string());
    let key = track_id_to_key(&track);
    assert_eq!(key, TrackKey("music/album/01.flac".to_string()));
    assert_eq!(
        track_key_to_id(key),
        track,
        "the key string is the identity"
    );
}

#[test]
fn playlist_identity_keys_round_trip_across_the_display_boundary() {
    let playlist = PlaylistId("42".to_string());
    let key = playlist_id_to_key(&playlist);
    assert_eq!(key, PlaylistKey("42".to_string()));
    assert_eq!(
        playlist_key_to_id(key),
        playlist,
        "the key string is the identity"
    );
}
