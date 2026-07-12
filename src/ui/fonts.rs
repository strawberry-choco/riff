use std::path::Path;

/// Try to find a CJK-capable font on the system and register it with egui.
/// This enables display of Chinese, Japanese, and Korean characters.
pub fn configure_fonts(ctx: &egui::Context) {
    // Prioritize .otf/.ttf over .ttc since egui's ab_glyph backend may not support
    // TrueType Collections well on all platforms.
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
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "cjk".to_owned(),
                    egui::FontData::from_owned(bytes),
                );
                // Prepend CJK font to proportional family.
                // CJK fonts like Noto Sans CJK, PingFang, Microsoft YaHei
                // all have high-quality Latin glyphs, so it's safe to use them
                // as the primary proportional font.
                if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    family.insert(0, "cjk".to_owned());
                }
                ctx.set_fonts(fonts);
                tracing::info!("riff: loaded CJK font from {}", path.display());
                return;
            }
            Err(e) => {
                tracing::warn!("riff: failed to read font {}: {}", path.display(), e);
            }
        }
    }
    tracing::debug!("riff: no CJK font found on system — Asian characters may not display");
}
