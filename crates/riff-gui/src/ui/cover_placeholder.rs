//! The generated-colour cover placeholder (design-handoff issue 14).
//!
//! Tracks and albums without embedded artwork render a solid colour block
//! derived from their own identity instead of a fallback glyph, so covers
//! read as part of the design's palette. The derivation is a pure function
//! of the identity string and the palette family — same item, same colour,
//! every time — and the hue is mapped onto a muted band per family so the
//! block never clashes with the active tokens (no white flash on dark, no
//! black hole on light).

use eframe::egui;

use crate::ui::app::{COVER_CACHE_CAP, lru_insert};

// The colour derivation itself lives in `theme` — the sanctioned home of
// colour construction (the no-hardcoded-colors scan exempts it) — and is
// re-exported here so the placeholder seam stays one import away.
pub use crate::ui::theme::generated_colour;

// --- Cache-through generation -------------------------------------------------
//
// The block materializes as a real texture inside the UI's existing cover
// texture map + LRU (the same bounded cache real covers ride), so every
// render site keeps its `Option<TextureHandle>` seam and the block renders
// through the same texture path as real art — including headless snapshots.

/// Cache-key prefix marking a generated block. The reserved prefix keeps
/// generated entries out of the real-cover lookups: the request flow (which
/// checks the plain `TrackId` key) keeps treating the track as artless, so
/// real art still resolves, lands under the plain key, and wins.
pub const GENERATED_KEY_PREFIX: &str = "gen\u{1f}";

/// The cache key under which one identity's generated block is stored.
#[must_use]
pub fn generated_key(identity: &str) -> String {
    format!("{GENERATED_KEY_PREFIX}{identity}")
}

/// The generated block as a 1×1 image: a solid colour needs no resolution —
/// every render site stretches it over its allotted square.
#[must_use]
pub fn generated_image(seed: &str, dark: bool) -> egui::ColorImage {
    let colour = generated_colour(seed, dark);
    egui::ColorImage::from_rgba_unmultiplied([1, 1], &[colour.r(), colour.g(), colour.b(), u8::MAX])
}

/// Resolve one identity's cover texture through the shared cache: the real
/// cover under the plain key when one is cached, otherwise the identity's
/// generated block (created once on a full miss, then cached), evicting
/// through the same LRU cap real covers obey. `dark` selects the palette
/// family the block is derived for. The return is handed to the render
/// sites' `Option<TextureHandle>` seams, which keep their pre-texture
/// fallbacks for the not-yet-rendered window.
pub fn lookup_cover_texture<S: std::hash::BuildHasher>(
    textures: &mut std::collections::HashMap<String, egui::TextureHandle, S>,
    lru_keys: &mut Vec<String>,
    ctx: &egui::Context,
    dark: bool,
    identity: &str,
) -> egui::TextureHandle {
    // Real art first — it always wins over the generated block.
    if let Some(texture) = touch(textures, lru_keys, identity) {
        return texture;
    }
    let key = generated_key(identity);
    if let Some(texture) = touch(textures, lru_keys, &key) {
        return texture;
    }

    let texture = ctx.load_texture(
        format!("generated cover {identity}"),
        generated_image(identity, dark),
        egui::TextureOptions::default(),
    );
    textures.insert(key.clone(), texture.clone());
    for old in lru_insert(lru_keys, key, COVER_CACHE_CAP) {
        textures.remove(&old);
    }
    texture
}

/// Clone a cached texture, marking its key most-recently-used.
fn touch<S: std::hash::BuildHasher>(
    textures: &std::collections::HashMap<String, egui::TextureHandle, S>,
    lru_keys: &mut Vec<String>,
    key: &str,
) -> Option<egui::TextureHandle> {
    let texture = textures.get(key)?;
    lru_keys.retain(|k| k != key);
    lru_keys.push(key.to_string());
    Some(texture.clone())
}

/// Drop every generated block from the shared cache, keeping real covers.
/// Called when the blocks' derivation inputs move: a palette-family switch
/// (re-derive under the new tokens) and the "Read embedded artwork" toggle
/// (let affected tracks re-resolve so real art can surface).
pub fn evict_generated<S: std::hash::BuildHasher>(
    textures: &mut std::collections::HashMap<String, egui::TextureHandle, S>,
    lru_keys: &mut Vec<String>,
) {
    lru_keys.retain(|k| !k.starts_with(GENERATED_KEY_PREFIX));
    textures.retain(|k, _| !k.starts_with(GENERATED_KEY_PREFIX));
}
