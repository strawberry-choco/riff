//! The artwork presentation primitive, its placeholder, and its cache key space.
//!
//! One owner for how cover art is PAINTED: the texture block, the fit that maps
//! a texture into it, the hairline frame, and the palette-aware well a surface
//! shows while it holds no texture. Every surface that draws artwork — the
//! Library rows and browser thumbnails, the player bar, Now Playing, and the
//! inspector — routes through [`paint`], so a fix to rounding, tint, or the
//! placeholder has one place to land.
//!
//! What stays with each host (ADR 0006): the cover *request*, the worker poll,
//! the texture upload, and the bounded LRU. [`lookup_cover_texture`] is the
//! whole artwork side of that seam — it reads and writes caller-owned buffers,
//! never a cache of its own.
//!
//! ## The one shared placeholder
//!
//! An artless item resolves the shared music-icon *tile* through
//! [`lookup_cover_texture`] and paints it as an ordinary texture: one image for
//! every identity, derived from `surface_2` / `ink_3`, so a palette-family flip
//! regenerates it ([`evict_generated`]).
//!
//! The [`Placeholder`]s below are the *other* state — the frame before a
//! texture exists at all. Each surface's idle block is recorded here as it
//! renders today, because consolidating the player bar's gradient or Now
//! Playing's text glyph into the shared tile is a visible design change, which
//! this extraction may not make silently (it moves no golden).
//!
//! Pure paint: reads [`Palette`] tokens (ADR 0004), mutates nothing, and
//! renders headlessly in `tests/ui_tests.rs` / `tests/golden_tests.rs`.

use eframe::egui;

use crate::ui::icons;
use crate::ui::theme::{self, Palette};
use riff_backend::app::cover_service::{COVER_CACHE_CAP, lru_insert};
use riff_backend::app::traits::RequestedSize;

// --- The cache key space -------------------------------------------------------
//
// Moved out of `ui::app` so the artwork module and the application module no
// longer form a sibling cycle: `ui::app` asks the Cover Service for art and
// lands it in the buffers below, and this module owns the key space those
// buffers are indexed by.

/// Key of the UI's texture cache: one artwork identity at one requested box.
///
/// The identity is a track path — albums and artists resolve through their
/// first track — so it is a `String` rather than a `TrackId`. The size is part
/// of the key because a hero upload and a thumbnail upload of the same track
/// are different pixels, and neither may stand in for the other.
pub type CoverCacheKey = (String, u32, u32);

/// The cache key for one artwork identity at one requested box.
#[must_use]
pub fn cover_cache_key(identity: &str, size: RequestedSize) -> CoverCacheKey {
    (identity.to_string(), size.width, size.height)
}

/// The canonical display boxes, one per kind of surface.
///
/// Surfaces ask for the nearest of these rather than their own exact pixel
/// size: every distinct `(identity, size)` pair is a separate worker job and a
/// separate texture, so an unbounded set of boxes would multiply both.
pub const COVER_THUMB: RequestedSize = RequestedSize {
    width: 56,
    height: 56,
};
pub const COVER_CARD: RequestedSize = RequestedSize {
    width: 200,
    height: 200,
};
pub const COVER_HERO: RequestedSize = RequestedSize {
    width: 512,
    height: 512,
};

// --- The shared placeholder tile ------------------------------------------------

/// Cache key under which the shared placeholder tile is stored. The `gen`
/// prefix keeps it out of the real-cover lookups: the request flow keeps
/// treating the track as artless, so real art still resolves, lands under the
/// track's own key, and wins over the tile.
pub const PLACEHOLDER_KEY: &str = "gen\u{1f}music-icon";

/// The tile's entry in the composite key space. It is drawn scaled to
/// whatever box the miss belongs to, so it needs one size-independent entry
/// rather than one per canonical size.
#[must_use]
pub fn placeholder_cache_key() -> CoverCacheKey {
    cover_cache_key(
        PLACEHOLDER_KEY,
        RequestedSize {
            width: 0,
            height: 0,
        },
    )
}

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
    textures: &mut std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
    ctx: &egui::Context,
    palette: &Palette,
    identity: &str,
    size: RequestedSize,
) -> egui::TextureHandle {
    // Real art first — it always wins over the placeholder tile.
    let key = cover_cache_key(identity, size);
    if let Some(texture) = touch(textures, lru_keys, &key) {
        return texture;
    }
    let placeholder = placeholder_cache_key();
    if let Some(texture) = touch(textures, lru_keys, &placeholder) {
        return texture;
    }

    let texture = ctx.load_texture(
        "riff cover placeholder",
        placeholder_image(palette),
        egui::TextureOptions::default(),
    );
    textures.insert(placeholder.clone(), texture.clone());
    for old in lru_insert(lru_keys, placeholder, COVER_CACHE_CAP) {
        textures.remove(&old);
    }
    texture
}

/// Clone a cached texture, marking its key most-recently-used.
fn touch<S: std::hash::BuildHasher>(
    textures: &std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
    key: &CoverCacheKey,
) -> Option<egui::TextureHandle> {
    let texture = textures.get(key)?;
    lru_keys.retain(|k| k != key);
    lru_keys.push(key.clone());
    Some(texture.clone())
}

/// Drop the placeholder tile from the shared cache, keeping real covers.
/// Called when the tile's derivation inputs move — a palette-family flip
/// re-renders it under the active tokens.
pub fn evict_generated<S: std::hash::BuildHasher>(
    textures: &mut std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
) {
    let placeholder = placeholder_cache_key();
    lru_keys.retain(|k| *k != placeholder);
    textures.remove(&placeholder);
}

// --- The presentation primitive -------------------------------------------------

/// Full-texture UV rect for [`egui::Painter::image`].
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// How a texture maps onto the artwork block. The caller states it, so no
/// surface distorts its art by accident — and a surface that wants its art
/// stretched says so out loud.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// Stretch the texture over the whole block. This is what every surface
    /// selects today, so square art in a non-square block stays distorted;
    /// switching a surface to [`Fit::Contain`] is a visible change and belongs
    /// to its own behavior decision.
    Fill,
    /// Keep the artwork's own `aspect` (width ÷ height) inside the block,
    /// centered — letterboxed rather than stretched.
    Contain { aspect: f32 },
}

/// What the block shows while the host holds no texture for it — the frame
/// before a cover request lands, or an idle surface with nothing to ask for.
/// Each variant is one existing surface's idle block, now owned here.
#[derive(Debug, Clone, Copy)]
pub enum Placeholder {
    /// A bare `surface_2` rounded block (the inspector's art slot).
    Well { radius: f32 },
    /// The `surface_2` block carrying a rasterized music-glyph texture, inset
    /// by `inset` (a browser thumbnail slot).
    GlyphWell {
        radius: f32,
        glyph: egui::TextureId,
        inset: f32,
    },
    /// The `surface_2` block carrying the music note as text at `size_px`
    /// (the Now Playing cover).
    EmojiWell { radius: f32, size_px: f32 },
    /// The `surface_2`→`surface_3` gradient (the player bar's idle cover).
    Gradient,
}

/// One artwork block's props.
pub struct Artwork {
    /// The block's rect — the placeholder always fills it, a texture fills it
    /// or is letterboxed inside it per `fit`.
    pub rect: egui::Rect,
    /// The texture to paint: real art, or the shared placeholder tile the host
    /// resolved. `None` paints `placeholder`.
    pub texture: Option<egui::TextureId>,
    pub fit: Fit,
    /// Multiplied into the texture; [`theme::TEXTURE_TINT`] draws it unmodified.
    pub tint: egui::Color32,
    /// What the block shows while `texture` is `None`. `None` leaves an empty
    /// block — a surface that treats artwork as optional passes that.
    pub placeholder: Option<Placeholder>,
    /// Radius of the 1px `palette.border` hairline framing the block; `None`
    /// paints no frame.
    pub border: Option<f32>,
}

/// Paint one artwork block: the texture under its fit, or the surface's idle
/// placeholder, then the optional hairline frame over both. A block with
/// neither paints nothing at all — not even its frame.
pub fn paint(painter: &egui::Painter, palette: &Palette, art: &Artwork) {
    if art.texture.is_none() && art.placeholder.is_none() {
        return;
    }
    match art.texture {
        Some(texture) => {
            let (rect, uv) = match art.fit {
                Fit::Fill => (art.rect, UV_FULL),
                Fit::Contain { aspect } => (contained(art.rect, aspect), UV_FULL),
            };
            painter.image(texture, rect, uv, art.tint);
        }
        None => {
            if let Some(placeholder) = art.placeholder {
                paint_placeholder(painter, palette, art.rect, placeholder);
            }
        }
    }
    if let Some(radius) = art.border {
        painter.rect_stroke(
            art.rect,
            radius,
            egui::Stroke::new(1.0_f32, palette.border),
            egui::StrokeKind::Inside,
        );
    }
}

/// The largest centered rect with the artwork's `aspect` inside `rect`.
fn contained(rect: egui::Rect, aspect: f32) -> egui::Rect {
    if aspect <= 0.0 {
        return rect;
    }
    let width = rect.width().min(rect.height() * aspect);
    egui::Rect::from_center_size(rect.center(), egui::vec2(width, width / aspect))
}

fn paint_placeholder(
    painter: &egui::Painter,
    palette: &Palette,
    rect: egui::Rect,
    placeholder: Placeholder,
) {
    match placeholder {
        Placeholder::Well { radius } => {
            painter.rect_filled(rect, radius, palette.surface_2);
        }
        Placeholder::GlyphWell {
            radius,
            glyph,
            inset,
        } => {
            painter.rect_filled(rect, radius, palette.surface_2);
            painter.image(glyph, rect.shrink(inset), UV_FULL, palette.ink_3);
        }
        Placeholder::EmojiWell { radius, size_px } => {
            painter.rect_filled(rect, radius, palette.surface_2);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{1F3B5}",
                egui::FontId::proportional(size_px),
                palette.ink_3,
            );
        }
        Placeholder::Gradient => {
            // Two triangles whose vertex colors interpolate from surface_2
            // (top) down to surface_3 (bottom).
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(rect.left_top(), palette.surface_2);
            mesh.colored_vertex(rect.right_top(), palette.surface_2);
            mesh.colored_vertex(rect.right_bottom(), palette.surface_3);
            mesh.colored_vertex(rect.left_bottom(), palette.surface_3);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
        }
    }
}
