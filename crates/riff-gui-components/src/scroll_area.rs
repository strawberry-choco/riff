//! The egui half of [`ScrollControl`]: the one constructor for the
//! `ScrollArea` a `ScrollControl`-driven column renders in.
//!
//! The handshake itself — vertical, unanimated, keyed by the control's salt,
//! opened at the control's offset when it names one — is stated here once
//! instead of being hand-built at each render site. The *reason* it is
//! unanimated is NOT restated: it, and the warning against flipping the other
//! eight `ScrollArea`s in the app to match, are written once on
//! [`ScrollControl`] ("Why the columns that take a `ScrollControl` also pass
//! `.animated(false)`"). Read that paragraph before changing what this module
//! builds; this module is the HOW, `ScrollControl` is the WHY.
//!
//! Two entry points, because the two call shapes are genuinely two states: a
//! site that has a [`ScrollControl`] this frame, and one that may not.

use egui;

use crate::scroll_control::ScrollControl;

/// The `ScrollArea` for a column that holds a [`ScrollControl`] this frame:
/// vertical, unanimated, keyed by the control's salt, and opened at the
/// control's start offset when it names one (the WHY of `.animated(false)`
/// lives on [`ScrollControl`], as do the other `ScrollArea`s' exemption).
///
/// `auto_shrink` is a per-site fact passed through, never unified: some
/// columns shrink today and some have never called the setter, and this fold
/// keeps each site's exact builder set.
pub fn controlled_scroll_area(control: ScrollControl, auto_shrink: bool) -> egui::ScrollArea {
    let mut area = base_area(auto_shrink).id_salt(control.salt);
    if let Some(offset) = control.start {
        area = area.vertical_scroll_offset(offset);
    }
    area
}

/// [`controlled_scroll_area`] for a site that may not hold a control this
/// frame. Without one, the same vertical, unanimated area is built keyed by
/// `no_control_salt`, and egui's own scroll state is left untouched.
pub fn optional_scroll_area(
    control: Option<ScrollControl>,
    no_control_salt: &'static str,
    auto_shrink: bool,
) -> egui::ScrollArea {
    match control {
        Some(control) => controlled_scroll_area(control, auto_shrink),
        None => base_area(auto_shrink).id_salt(no_control_salt),
    }
}

/// The shared base every site spells the same way: vertical and unanimated,
/// with `.auto_shrink(false)` applied only for the sites that pass it.
fn base_area(auto_shrink: bool) -> egui::ScrollArea {
    let area = egui::ScrollArea::vertical().animated(false);
    if auto_shrink {
        area
    } else {
        area.auto_shrink(false)
    }
}
