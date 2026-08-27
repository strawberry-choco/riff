//! Vendored Lucide glyphs and the icon helper that serves them (Issue 06).
//!
//! The redesign's ~22 interface glyphs are Lucide SVGs committed under
//! `assets/icons/` (ISC license, see `assets/icons/LICENSE-Lucide.txt`).
//! They are compiled into the binary via [`include_str!`] and rasterized
//! on demand into tinted egui textures, so every icon follows the active
//! palette instead of a flat color (ADR 0004).
//!
//! Later restyle tickets (07–12) draw their controls through
//! [`icon_button`] / [`IconCache::texture`] rather than emoji or text
//! glyphs.

use eframe::egui;
use std::collections::HashMap;

use super::theme;

/// The vendored Lucide glyphs, in asset-name order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    ArrowLeft,
    Close,
    Disc,
    Folder,
    FolderOpen,
    Library,
    ListMusic,
    Minimize,
    Moon,
    Music,
    Pause,
    Pencil,
    Play,
    Plus,
    Repeat,
    RepeatOne,
    RefreshCw,
    Search,
    Settings,
    Shuffle,
    SkipBack,
    SkipForward,
    Sparkles,
    Square,
    Sun,
    Trash,
    VolumeHigh,
    VolumeMuted,
}

impl Icon {
    /// Every glyph the shell vendors; later tickets pick from this list.
    pub const ALL: &'static [Self] = &[
        Self::ArrowLeft,
        Self::Close,
        Self::Disc,
        Self::Folder,
        Self::FolderOpen,
        Self::Library,
        Self::ListMusic,
        Self::Minimize,
        Self::Moon,
        Self::Music,
        Self::Pause,
        Self::Pencil,
        Self::Play,
        Self::Plus,
        Self::Repeat,
        Self::RepeatOne,
        Self::RefreshCw,
        Self::Search,
        Self::Settings,
        Self::Shuffle,
        Self::SkipBack,
        Self::SkipForward,
        Self::Sparkles,
        Self::Square,
        Self::Sun,
        Self::Trash,
        Self::VolumeHigh,
        Self::VolumeMuted,
    ];

    /// The vendored asset stem for this glyph (the `assets/icons/` name).
    #[must_use]
    pub fn asset_name(self) -> &'static str {
        match self {
            Self::ArrowLeft => "arrow-left",
            Self::Close => "x",
            Self::Disc => "disc-3",
            Self::Folder => "folder",
            Self::FolderOpen => "folder-open",
            Self::Library => "library",
            Self::ListMusic => "list-music",
            Self::Minimize => "minus",
            Self::Moon => "moon",
            Self::Music => "music",
            Self::Pause => "pause",
            Self::Pencil => "pencil",
            Self::Play => "play",
            Self::Plus => "plus",
            Self::Repeat => "repeat",
            Self::RepeatOne => "repeat-1",
            Self::RefreshCw => "refresh-cw",
            Self::Search => "search",
            Self::Settings => "settings",
            Self::Shuffle => "shuffle",
            Self::SkipBack => "skip-back",
            Self::SkipForward => "skip-forward",
            Self::Sparkles => "sparkles",
            Self::Square => "square",
            Self::Sun => "sun",
            Self::Trash => "trash-2",
            Self::VolumeHigh => "volume-2",
            Self::VolumeMuted => "volume-x",
        }
    }

    /// The embedded Lucide SVG source for this glyph.
    #[must_use]
    pub fn svg(self) -> &'static str {
        match self {
            Self::ArrowLeft => include_str!("../../assets/icons/arrow-left.svg"),
            Self::Close => include_str!("../../assets/icons/x.svg"),
            Self::Disc => include_str!("../../assets/icons/disc-3.svg"),
            Self::Folder => include_str!("../../assets/icons/folder.svg"),
            Self::FolderOpen => include_str!("../../assets/icons/folder-open.svg"),
            Self::Library => include_str!("../../assets/icons/library.svg"),
            Self::ListMusic => include_str!("../../assets/icons/list-music.svg"),
            Self::Minimize => include_str!("../../assets/icons/minus.svg"),
            Self::Moon => include_str!("../../assets/icons/moon.svg"),
            Self::Music => include_str!("../../assets/icons/music.svg"),
            Self::Pause => include_str!("../../assets/icons/pause.svg"),
            Self::Pencil => include_str!("../../assets/icons/pencil.svg"),
            Self::Play => include_str!("../../assets/icons/play.svg"),
            Self::Plus => include_str!("../../assets/icons/plus.svg"),
            Self::Repeat => include_str!("../../assets/icons/repeat.svg"),
            Self::RepeatOne => include_str!("../../assets/icons/repeat-1.svg"),
            Self::RefreshCw => include_str!("../../assets/icons/refresh-cw.svg"),
            Self::Search => include_str!("../../assets/icons/search.svg"),
            Self::Settings => include_str!("../../assets/icons/settings.svg"),
            Self::Shuffle => include_str!("../../assets/icons/shuffle.svg"),
            Self::SkipBack => include_str!("../../assets/icons/skip-back.svg"),
            Self::SkipForward => include_str!("../../assets/icons/skip-forward.svg"),
            Self::Sparkles => include_str!("../../assets/icons/sparkles.svg"),
            Self::Square => include_str!("../../assets/icons/square.svg"),
            Self::Sun => include_str!("../../assets/icons/sun.svg"),
            Self::Trash => include_str!("../../assets/icons/trash-2.svg"),
            Self::VolumeHigh => include_str!("../../assets/icons/volume-2.svg"),
            Self::VolumeMuted => include_str!("../../assets/icons/volume-x.svg"),
        }
    }
}

/// Rasterize one Lucide SVG at `size_px × size_px`, painting its strokes in
/// `color`. Lucide sources stroke `currentColor`; substituting it before
/// parsing tints the glyph to the active palette's ink (ADR 0004). Returns
/// `None` when the size is out of range or the vendored source fails to
/// parse — which would be a packaging bug, not a runtime condition.
#[must_use]
#[expect(clippy::cast_precision_loss)]
pub fn rasterize(svg: &str, size_px: usize, color: egui::Color32) -> Option<egui::ColorImage> {
    let px = u32::try_from(size_px).ok()?;
    if px == 0 {
        return None;
    }

    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let tinted = svg.replace("currentColor", &hex);

    // Icon-only paths need no fonts: default options carry an empty font
    // database, which is exactly right here.
    let tree = resvg::usvg::Tree::from_str(&tinted, &resvg::usvg::Options::default()).ok()?;
    let scale = px as f32 / tree.size().width();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied RGBA; egui wants straight alpha.
    let rgba = pixmap.take_demultiplied();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [px as usize, px as usize],
        &rgba,
    ))
}

/// Cache key: one texture per (glyph, pixel size, tint) triple.
type IconKey = (Icon, u32, egui::Color32);

/// Rasterized-glyph cache shared by every icon control in the app. Textures
/// are keyed by `(icon, size, color)` so palette switches re-tint instead of
/// showing stale colors.
#[derive(Default)]
pub struct IconCache {
    textures: HashMap<IconKey, egui::TextureHandle>,
}

impl IconCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The texture id for `icon` at `size` points in `color`, rasterizing
    /// and caching on first request. Fresh frames hand back the cached
    /// texture's id (a small copy) instead of cloning the handle.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        icon: Icon,
        size: f32,
        color: egui::Color32,
    ) -> egui::TextureId {
        let px = size.round().max(1.0) as u32;
        let key = (icon, px, color);
        if let Some(tex) = self.textures.get(&key) {
            return tex.id();
        }

        let image = rasterize(icon.svg(), px as usize, color).unwrap_or_else(|| {
            tracing::warn!("Failed to rasterize icon {}", icon.asset_name());
            // Invisible placeholder, derived from a token rather than a flat
            // constructor (ADR 0004): rasterization never fails for vendored
            // sources, so this only ever shows as a blank pixel.
            let clear = theme::INK.gamma_multiply(0.0);
            egui::ColorImage::new([1, 1], vec![clear])
        });
        // The key rides along in the texture name so distinct entries can
        // never alias one texture id.
        let tex = ctx.load_texture(
            format!(
                "riff-icon-{}-{px}-{:02x}{:02x}{:02x}",
                icon.asset_name(),
                color.r(),
                color.g(),
                color.b()
            ),
            image,
            egui::TextureOptions::LINEAR,
        );
        let id = tex.id();
        self.textures.insert(key, tex);
        id
    }
}

/// An icon-only button with an accessibility label, so assistive tech (and
/// the golden-image harness) can find it by name despite having no text.
pub fn icon_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    icon: Icon,
    label: &str,
    size: f32,
    color: egui::Color32,
) -> egui::Response {
    let tex_id = cache.texture(ui.ctx(), icon, size, color);
    let sized = egui::load::SizedTexture::new(tex_id, egui::vec2(size, size));
    let response = ui.add(egui::Button::image(egui::Image::from_texture(sized)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}
