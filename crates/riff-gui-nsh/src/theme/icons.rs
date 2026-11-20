//! The riff icon set on nsh, ported from the egui design system's
//! `icons.rs`: the same 35 vendored Lucide glyphs (ISC license, see
//! `assets/icons/LICENSE-Lucide.txt`), drawn through nsh's `svg()` element
//! from the vendored sources.
//!
//! The path taken is **SVG with tint through the `svg()` element**, the
//! plan's first choice, verified in the nsh source rather than assumed: a
//! `svg()` element draws its document as canvas shapes that stay sharp at
//! every size, and the paints that were `currentColor` take the element's
//! text color at draw time (`ViewPaint::FillCurrentColor`,
//! `nsh-core-0.6.1/src/svg/drawing.rs:1-10`; documented at
//! `element/host.rs:381-399`). Every Lucide source strokes
//! `stroke="currentColor"`, so setting the element's text color is the tint,
//! no rasterization or re-parsing per size involved. The sources ride in as
//! inline markup ([`Icon::markup`]) — the first-frame path — because the
//! alternative `asset:/` URI needs a location the dev `cargo run` layout
//! does not carry (see the trail); the files stay shipped by `assets!()`
//! for `nsh run` and packaging. The `svg` feature the platform crates carry
//! (root `Cargo.toml`, ADR 0022) is what enables the native SVG renderer.
//! The egui store's resvg
//! rasterizer and its texture cache are not ported: nsh parses each source
//! once, keeps it vector, and leases it while shown.

use nsh_core::element::{AdaptiveColor, ImageSrc, IntoReactive, Styled, Svg, px, svg};

/// The vendored Lucide glyphs, in asset-name order (the egui store's
/// variant list, carried over unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Lucide `arrow-left`.
    ArrowLeft,
    /// The Lucide chevron glyphs for collapsible section headers: down when
    /// expanded, right when folded.
    ChevronDown,
    /// Lucide `chevron-right`.
    ChevronRight,
    /// Lucide `x`, the close affordance.
    Close,
    /// Lucide `disc-3`, the Library empty-state disc.
    Disc,
    /// The Lucide corner-bracket expand glyph (fullscreen/expand button).
    Expand,
    /// The Lucide corner-bracket collapse glyph (fullscreen/expand button
    /// while the enlarged view is up).
    Collapse,
    /// Lucide `folder`.
    Folder,
    /// Lucide `folder-open`.
    FolderOpen,
    /// Lucide `heart`, the favorite control.
    Heart,
    /// Lucide `library`.
    Library,
    /// Lucide `layout-grid`.
    LayoutGrid,
    /// Lucide `list`.
    List,
    /// Lucide `list-music`.
    ListMusic,
    /// Lucide `minus`, the minimize caption control.
    Minimize,
    /// Lucide `moon`, the dark-theme toggle glyph.
    Moon,
    /// Lucide `music`.
    Music,
    /// Lucide `pause`.
    Pause,
    /// Lucide `pencil`, the tag editor.
    Pencil,
    /// Lucide `play`.
    Play,
    /// Lucide `plus`.
    Plus,
    /// Lucide `repeat`.
    Repeat,
    /// Lucide `repeat-1`.
    RepeatOne,
    /// Lucide `refresh-cw`, the scan control.
    RefreshCw,
    /// Lucide `search`.
    Search,
    /// Lucide `settings`.
    Settings,
    /// Lucide `shuffle`.
    Shuffle,
    /// Lucide `skip-back`.
    SkipBack,
    /// Lucide `skip-forward`.
    SkipForward,
    /// Lucide `sparkles`, the Smart Playlists glyph.
    Sparkles,
    /// Lucide `square`, the stop transport control.
    Square,
    /// Lucide `sun`, the light-theme toggle glyph.
    Sun,
    /// Lucide `trash-2`, the destructive affordance.
    Trash,
    /// Lucide `volume-2`.
    VolumeHigh,
    /// Lucide `volume-x`.
    VolumeMuted,
}

impl Icon {
    /// The natural 24 pt size every Lucide source carries
    /// (`viewBox 0 0 24 24`), the size an unstyled glyph lays out at.
    /// Exposed so a call site can name the icon scale's base instead of a
    /// bare 24.0.
    pub const ICON_SIZE: f32 = 24.0;

    /// Every glyph the shell vendors; later tickets pick from this list.
    pub const ALL: &'static [Icon] = &[
        Icon::ArrowLeft,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::Close,
        Icon::Disc,
        Icon::Expand,
        Icon::Collapse,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::Heart,
        Icon::Library,
        Icon::LayoutGrid,
        Icon::List,
        Icon::ListMusic,
        Icon::Minimize,
        Icon::Moon,
        Icon::Music,
        Icon::Pause,
        Icon::Pencil,
        Icon::Play,
        Icon::Plus,
        Icon::Repeat,
        Icon::RepeatOne,
        Icon::RefreshCw,
        Icon::Search,
        Icon::Settings,
        Icon::Shuffle,
        Icon::SkipBack,
        Icon::SkipForward,
        Icon::Sparkles,
        Icon::Square,
        Icon::Sun,
        Icon::Trash,
        Icon::VolumeHigh,
        Icon::VolumeMuted,
    ];

    /// The vendored asset stem (the `assets/icons/` name).
    #[must_use]
    pub fn asset_name(self) -> &'static str {
        match self {
            Icon::ArrowLeft => "arrow-left",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronRight => "chevron-right",
            Icon::Close => "x",
            Icon::Disc => "disc-3",
            Icon::Expand => "maximize",
            Icon::Collapse => "minimize-corners",
            Icon::Folder => "folder",
            Icon::FolderOpen => "folder-open",
            Icon::Heart => "heart",
            Icon::Library => "library",
            Icon::LayoutGrid => "layout-grid",
            Icon::List => "list",
            Icon::ListMusic => "list-music",
            Icon::Minimize => "minus",
            Icon::Moon => "moon",
            Icon::Music => "music",
            Icon::Pause => "pause",
            Icon::Pencil => "pencil",
            Icon::Play => "play",
            Icon::Plus => "plus",
            Icon::Repeat => "repeat",
            Icon::RepeatOne => "repeat-1",
            Icon::RefreshCw => "refresh-cw",
            Icon::Search => "search",
            Icon::Settings => "settings",
            Icon::Shuffle => "shuffle",
            Icon::SkipBack => "skip-back",
            Icon::SkipForward => "skip-forward",
            Icon::Sparkles => "sparkles",
            Icon::Square => "square",
            Icon::Sun => "sun",
            Icon::Trash => "trash-2",
            Icon::VolumeHigh => "volume-2",
            Icon::VolumeMuted => "volume-x",
        }
    }

    /// The vendored SVG source for this glyph, compiled into the binary and
    /// shown as inline markup: the drawing parses once per source in the
    /// first frame, with no `asset:/` location needed at runtime (the
    /// framework's directory bases resolve `asset:/@<package>/<path>`, a
    /// packaged layout a dev `cargo run` does not have; the files stay
    /// listed by `assets!()` for `nsh run` and packaging).
    #[must_use]
    pub fn markup(self) -> &'static str {
        match self {
            Icon::ArrowLeft => include_str!("../../assets/icons/arrow-left.svg"),
            Icon::ChevronDown => include_str!("../../assets/icons/chevron-down.svg"),
            Icon::ChevronRight => include_str!("../../assets/icons/chevron-right.svg"),
            Icon::Close => include_str!("../../assets/icons/x.svg"),
            Icon::Disc => include_str!("../../assets/icons/disc-3.svg"),
            Icon::Expand => include_str!("../../assets/icons/maximize.svg"),
            Icon::Collapse => include_str!("../../assets/icons/minimize-corners.svg"),
            Icon::Folder => include_str!("../../assets/icons/folder.svg"),
            Icon::FolderOpen => include_str!("../../assets/icons/folder-open.svg"),
            Icon::Heart => include_str!("../../assets/icons/heart.svg"),
            Icon::Library => include_str!("../../assets/icons/library.svg"),
            Icon::LayoutGrid => include_str!("../../assets/icons/layout-grid.svg"),
            Icon::List => include_str!("../../assets/icons/list.svg"),
            Icon::ListMusic => include_str!("../../assets/icons/list-music.svg"),
            Icon::Minimize => include_str!("../../assets/icons/minus.svg"),
            Icon::Moon => include_str!("../../assets/icons/moon.svg"),
            Icon::Music => include_str!("../../assets/icons/music.svg"),
            Icon::Pause => include_str!("../../assets/icons/pause.svg"),
            Icon::Pencil => include_str!("../../assets/icons/pencil.svg"),
            Icon::Play => include_str!("../../assets/icons/play.svg"),
            Icon::Plus => include_str!("../../assets/icons/plus.svg"),
            Icon::Repeat => include_str!("../../assets/icons/repeat.svg"),
            Icon::RepeatOne => include_str!("../../assets/icons/repeat-1.svg"),
            Icon::RefreshCw => include_str!("../../assets/icons/refresh-cw.svg"),
            Icon::Search => include_str!("../../assets/icons/search.svg"),
            Icon::Settings => include_str!("../../assets/icons/settings.svg"),
            Icon::Shuffle => include_str!("../../assets/icons/shuffle.svg"),
            Icon::SkipBack => include_str!("../../assets/icons/skip-back.svg"),
            Icon::SkipForward => include_str!("../../assets/icons/skip-forward.svg"),
            Icon::Sparkles => include_str!("../../assets/icons/sparkles.svg"),
            Icon::Square => include_str!("../../assets/icons/square.svg"),
            Icon::Sun => include_str!("../../assets/icons/sun.svg"),
            Icon::Trash => include_str!("../../assets/icons/trash-2.svg"),
            Icon::VolumeHigh => include_str!("../../assets/icons/volume-2.svg"),
            Icon::VolumeMuted => include_str!("../../assets/icons/volume-x.svg"),
        }
    }

    /// The glyph as an element of `size` points, painted in `tint`: an nsh
    /// SVG drawing whose `currentColor` strokes take the element's text
    /// color, sharp at any size (see the module doc). The tint takes
    /// anything reactive — a palette `Hsla`, or a binding reading the
    /// palette signal — so icons follow variant switches. This is the icon
    /// API's whole surface — size and color, as the plan asks.
    #[must_use]
    pub fn element(self, size: f32, tint: impl IntoReactive<AdaptiveColor>) -> Svg {
        svg(ImageSrc::svg_markup(self.markup()))
            .w(px(size))
            .h(px(size))
            .text_color(tint)
    }
}

/// A convenience alias so call sites read like the egui store's
/// `icon_button` sites did: `icon(Icon::Play, 16.0, palette.ink)`.
#[must_use]
pub fn icon(icon: Icon, size: f32, tint: impl IntoReactive<AdaptiveColor>) -> Svg {
    icon.element(size, tint)
}
