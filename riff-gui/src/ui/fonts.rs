//! Font wiring for the riff UI (Issue 02).
//!
//! The Inter family is vendored into `assets/fonts/` and compiled into the
//! binary with `include_bytes!`, so the whole UI renders in Inter on every
//! platform with no runtime font discovery. System CJK fonts are still
//! scanned at startup and appended as *fallbacks* behind Inter, preserving
//! the CJK support the app shipped with.
//!
//! The pure seam for tests is [`font_definitions`]; [`configure_fonts`] is
//! the thin side-effectful wrapper that installs it on an [`egui::Context`].

use std::path::Path;

use egui::FontFamily;

/// Font-data key of the vendored Inter Regular face — the primary UI font
/// (`--riff-font-sans` head of chain).
pub const INTER_PRIMARY_KEY: &str = "inter-regular";

/// Font-data key under which the discovered system CJK fallback is inserted.
pub const CJK_FALLBACK_KEY: &str = "cjk";

// --- Vendored Inter faces (`assets/fonts/`, SIL OFL 1.1) ---------------------
//
// The mockup uses four weights: regular body text, medium buttons/row labels,
// semibold section headers/h1, and bold wordmark accents.

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");

/// Every vendored Inter face as `(font-data key, raw bytes)`; Regular comes
/// first and doubles as the primary proportional face.
pub const INTER_FACES: &[(&str, &[u8])] = &[
    (INTER_PRIMARY_KEY, INTER_REGULAR),
    ("inter-medium", INTER_MEDIUM),
    ("inter-semibold", INTER_SEMIBOLD),
    ("inter-bold", INTER_BOLD),
];

/// Named family rendering Inter Medium — buttons and row labels in the
/// mockup (`font-medium`).
#[must_use]
pub fn family_medium() -> FontFamily {
    FontFamily::Name("riff-inter-medium".into())
}

/// Named family rendering Inter `SemiBold` — section headers, h1s, and the
/// Now Playing title in the mockup (`font-semibold`).
#[must_use]
pub fn family_semibold() -> FontFamily {
    FontFamily::Name("riff-inter-semibold".into())
}

/// Named family rendering Inter Bold — the wordmark accent (`font-bold`).
#[must_use]
pub fn family_bold() -> FontFamily {
    FontFamily::Name("riff-inter-bold".into())
}

/// Build the full [`egui::FontDefinitions`] for the app: vendored Inter as
/// the primary proportional face, egui's bundled fonts kept as fallbacks,
/// the monospace family left resolvable for time readouts, and a system CJK
/// font appended behind Inter when one exists.
#[must_use]
pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();

    for (key, bytes) in INTER_FACES {
        fonts
            .font_data
            .insert((*key).to_owned(), egui::FontData::from_static(bytes).into());
    }

    // Inter owns Latin; every other family keeps its defaults behind it.
    if let Some(chain) = fonts.families.get_mut(&FontFamily::Proportional) {
        chain.insert(0, INTER_PRIMARY_KEY.to_owned());
    }
    fonts
        .families
        .insert(family_medium(), vec!["inter-medium".to_owned()]);
    fonts
        .families
        .insert(family_semibold(), vec!["inter-semibold".to_owned()]);
    fonts
        .families
        .insert(family_bold(), vec!["inter-bold".to_owned()]);

    // CJK fallback preserved from the pre-Inter setup, but demoted to *after*
    // Inter: those fonts' Latin glyphs must no longer shadow Inter's.
    if let Some(bytes) = find_cjk_font_bytes() {
        fonts.font_data.insert(
            CJK_FALLBACK_KEY.to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            family_medium(),
            family_semibold(),
            family_bold(),
        ] {
            if let Some(chain) = fonts.families.get_mut(&family) {
                chain.push(CJK_FALLBACK_KEY.to_owned());
            }
        }
        tracing::info!("riff: registered system CJK fallback font");
    } else {
        tracing::debug!("riff: no CJK font found on system — Asian characters may not display");
    }

    fonts
}

/// Install [`font_definitions`] on `ctx`. Called once at startup before the
/// first frame.
pub fn configure_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}

/// Scan the well-known per-platform font locations for a CJK-capable font
/// and return its bytes. Prefers `.otf`/`.ttf` over `.ttc` since egui's
/// `ab_glyph` backend may not support TrueType Collections on all platforms.
fn find_cjk_font_bytes() -> Option<Vec<u8>> {
    let paths: &[&str] = if cfg!(target_os = "linux") {
        &[
            // Noto Sans SC (Simplified Chinese) — individual OTF
            "/usr/share/fonts/opentype/noto/NotoSansSC-Regular.otf",
            "/usr/share/fonts/noto/NotoSansSC-Regular.otf",
            "/usr/share/fonts/noto-cjk/NotoSansSC-Regular.otf",
            // Noto Sans JP (Japanese) — individual OTF
            "/usr/share/fonts/opentype/noto/NotoSansJP-Regular.otf",
            "/usr/share/fonts/noto/NotoSansJP-Regular.otf",
            "/usr/share/fonts/noto-cjk/NotoSansJP-Regular.otf",
            // Noto Sans KR (Korean) — individual OTF
            "/usr/share/fonts/opentype/noto/NotoSansKR-Regular.otf",
            "/usr/share/fonts/noto/NotoSansKR-Regular.otf",
            "/usr/share/fonts/noto-cjk/NotoSansKR-Regular.otf",
            // Noto Sans CJK (all-in-one) — TTC
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/TTF/noto/NotoSansCJK-Regular.ttc",
            // Droid Sans Fallback (Android/Linux)
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            // WenQuanYi — TTC
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/PingFang.ttc",
            "/Library/Fonts/NotoSansCJK-Regular.ttc",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msyhbd.ttc",
            "C:\\Windows\\Fonts\\malgun.ttf",
            "C:\\Windows\\Fonts\\msgothic.ttc",
        ]
    } else {
        &[]
    };

    for path_str in paths {
        let path = Path::new(path_str);
        if !path.exists() {
            continue;
        }
        match std::fs::read(path) {
            Ok(bytes) => {
                tracing::info!("riff: loaded CJK font from {}", path.display());
                return Some(bytes);
            }
            Err(e) => {
                tracing::warn!("riff: failed to read font {}: {}", path.display(), e);
            }
        }
    }
    None
}
