//! The collection context menu: the shared Play, Play Next, Add to Queue
//! and Shuffle rows the egui components crate gives every set-of-tracks
//! surface (entity rows, folder nodes, playlist and smart-list headers).
//! Each target resolves its ids at dispatch time, so a menu opened against a
//! listing acts on what the listing holds then.

use nsh_core::element::desktop::{MenuItem, popup_menu_at};
use nsh_core::prelude::*;

use crate::bridge::UiModel;

use super::{
    CollectionTarget, add_batch_to_queue, play_batch, play_next_batch, shuffle_batch, target_ids,
};

/// Open the menu at the context event's page position.
pub fn open_collection_menu(model: Stored<UiModel>, target: CollectionTarget, at: Point<Points>) {
    let items = collection_menu_items(model, &target);
    drop(popup_menu_at(items, at));
}

/// The four shared rows, in the components crate's order.
#[must_use]
pub fn collection_menu_items(model: Stored<UiModel>, target: &CollectionTarget) -> Vec<MenuItem> {
    let play = target.clone();
    let next = target.clone();
    let queue = target.clone();
    let shuffle = target.clone();
    vec![
        MenuItem::new("Play").on_select(move || {
            play_batch(model, model.with(|m| target_ids(m, &play)));
        }),
        MenuItem::new("Play Next").on_select(move || {
            let tracks = model.with(|m| target_ids(m, &next));
            play_next_batch(model, &tracks);
        }),
        MenuItem::new("Add to Queue").on_select(move || {
            add_batch_to_queue(model, model.with(|m| target_ids(m, &queue)));
        }),
        MenuItem::new("Shuffle").on_select(move || {
            shuffle_batch(model, model.with(|m| target_ids(m, &shuffle)));
        }),
    ]
}
