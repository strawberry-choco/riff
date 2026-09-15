//! The music-icon cover placeholder.
//!
//! Tracks and albums whose artwork cannot be resolved — no embedded image,
//! no filesystem cover — render a neutral tile: the surface well with a
//! music glyph, the same "just a music icon" placeholder the now-playing
//! cover paints behind its art. Every artless item shares ONE tile under a
//! single cache key (the removed generated-colour block was the only
//! per-identity input, so nothing varies by item anymore), and it derives
//! from the active palette — `surface_2` for the well, `ink_3` for the
//! glyph — so a palette-family flip re-renders it like any themed surface.
//!
//! The tile materializes as a real texture inside the UI's existing cover
//! texture map + LRU (the same bounded cache real covers ride), so every
//! render site keeps its `Option<TextureHandle>` seam and the tile renders
//! through the same texture path as real art — including headless snapshots.

use eframe::egui;

use crate::ui::app::{COVER_CACHE_CAP, lru_insert};
use crate::ui::icons;
use crate::ui::theme::{self, Palette};

/// Cache key under which the shared placeholder tile is stored. The `gen`
/// prefix keeps it out of the real-cover lookups: the request flow (which
/// checks the plain `TrackId` key) keeps treating the track as artless, so
/// real art still resolves, lands under the plain key, and wins over the
/// tile.
pub const PLACEHOLDER_KEY: &str = "gen\u{1f}music-icon";

/// Render resolution of the tile in pixels. Every render site stretches
/// the square to its own cover size — 40px browser thumbnails up to the
/// 240px now-playing cover — so the tile is rasterized generously and
/// only ever downscaled to stay crisp.
const TILE_PX: usize = 256;

/// Build the placeholder image for `palette`: the `surface_2` well with
/// the music glyph tinted `ink_3`, centered at half the tile.
#[must_use]
pub fn placeholder_image(palette: &Palette) -> egui::ColorImage {
    let well = palette.surface_2;
    let mut rgba = Vec::with_capacity(TILE_PX * TILE_PX * 4);
    for _ in 0..TILE_PX * TILE_PX {
        rgba.extend_from_slice(&[well.r(), well.g(), well.b(), u8::MAX]);
    }
    let mut image = egui::ColorImage::from_rgba_unmultiplied([TILE_PX, TILE_PX], &rgba);

    let Some(glyph) = icons::rasterize(icons::Icon::Music.svg(), TILE_PX / 2, palette.ink_3) else {
        return image;
    };
    let inset = (TILE_PX - glyph.size[0]) / 2;
    for (i, px) in glyph.pixels.iter().enumerate() {
        if px.a() == 0 {
            continue;
        }
        let x = i % glyph.size[0] + inset;
        let y = i / glyph.size[0] + inset;
        image.pixels[y * TILE_PX + x] = theme::blend_over(image.pixels[y * TILE_PX + x], *px);
    }
    image
}

/// Resolve one item's cover texture through the shared cache: the real
/// cover under the plain key when one is cached, otherwise the shared
/// placeholder tile (created once on a full miss, then cached), evicting
/// through the same LRU cap real covers obey. `palette` supplies the
/// tile's well and glyph colours. The return is handed to the render
/// sites' `Option<TextureHandle>` seams, which keep their pre-texture
/// fallbacks for the not-yet-rendered window.
pub fn lookup_cover_texture<S: std::hash::BuildHasher>(
    textures: &mut std::collections::HashMap<String, egui::TextureHandle, S>,
    lru_keys: &mut Vec<String>,
    ctx: &egui::Context,
    palette: &Palette,
    identity: &str,
) -> egui::TextureHandle {
    // Real art first — it always wins over the placeholder tile.
    if let Some(texture) = touch(textures, lru_keys, identity) {
        return texture;
    }
    if let Some(texture) = touch(textures, lru_keys, PLACEHOLDER_KEY) {
        return texture;
    }

    let texture = ctx.load_texture(
        "riff cover placeholder",
        placeholder_image(palette),
        egui::TextureOptions::default(),
    );
    textures.insert(PLACEHOLDER_KEY.to_string(), texture.clone());
    for old in lru_insert(lru_keys, PLACEHOLDER_KEY.to_string(), COVER_CACHE_CAP) {
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

/// Drop the placeholder tile from the shared cache, keeping real covers.
/// Called when the tile's derivation inputs move — a palette-family flip
/// re-renders it under the active tokens.
pub fn evict_generated<S: std::hash::BuildHasher>(
    textures: &mut std::collections::HashMap<String, egui::TextureHandle, S>,
    lru_keys: &mut Vec<String>,
) {
    lru_keys.retain(|k| k != PLACEHOLDER_KEY);
    textures.remove(PLACEHOLDER_KEY);
}
