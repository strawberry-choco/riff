//! ADR 0004 token-authority sweeps, homed with the source they judge.
//!
//! The widgetry split moved the theme store into this crate, and the guard
//! moved with it: this file sweeps **this crate's own source** for what a
//! reader of the design system must not do — construct or derive a color,
//! declare a dimension of its own, or set a spacing gap by number. Each
//! rule is mechanical, which is the point — the acceptance for the
//! token-authority work is "grep-verifiable", and encoded here it stops
//! being something a later restyle ticket can quietly drift out of.
//!
//! `theme.rs` is the store, so the literals and dimensions are its job: it
//! is the one exempt file, as it always was.

/// True when a code line derives an egui color instead of reading one: a
/// `Color32`/`Rgba` construction from scratch (any `from_*` constructor or
/// associated constant), or a palette color scaled into a new one at the
/// call site (`gamma_multiply`, `linear_multiply`, `to_opaque`, …).
///
/// The constructor path alone was not enough: `palette.error
/// .gamma_multiply(0.1)` reads a token and passed clean while still
/// deciding a color outside the design system. Reading a color's channels
/// (`color.r()` for a texture name, `px.a() == 0` to skip a pixel) is not
/// a derivation and stays unflagged, and [`theme::blend_over`] stays
/// sanctioned too — it is the helper the token module exposes for
/// composing colors, and its callers composite rasterized texture pixels
/// rather than choosing a style.
fn hardcoded_color_literal(line: &str) -> bool {
    for marker in ["Color32::", "Rgba::"] {
        let mut search = 0;
        while let Some(rel) = line[search..].find(marker) {
            let start = search + rel + marker.len();
            if line[start..].starts_with("from_")
                || line[start..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
            {
                return true;
            }
            search = start;
        }
    }
    [
        ".gamma_multiply(",
        ".gamma_multiply_u8(",
        ".linear_multiply(",
        ".to_opaque(",
        ".to_normalized_rgba(",
        ".to_normalized_gamma_f32(",
    ]
    .iter()
    .any(|derives| line.contains(derives))
}

/// Every `(path, line number, trimmed source)` triple the sweeps judge:
/// `.rs` files under this crate's `src`, minus the exempt files, minus
/// comment lines, so prose may name the APIs a sweep bans.
fn source_lines() -> Vec<(std::path::PathBuf, usize, String)> {
    /// The one file the sweeps skip: the store itself.
    const EXEMPT_FILES: [&str; 1] = ["theme.rs"];

    fn scan_dir(
        exempt: &[&str],
        dir: &std::path::Path,
        lines: &mut Vec<(std::path::PathBuf, usize, String)>,
    ) {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .expect("the crate's src directory must be readable")
            .map(|entry| entry.expect("directory entries must resolve").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                scan_dir(exempt, &path, lines);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| exempt.contains(&name))
            {
                let source = std::fs::read_to_string(&path).expect("source files must be UTF-8");
                for (idx, line) in source.lines().enumerate() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    lines.push((path.clone(), idx + 1, trimmed.to_owned()));
                }
            }
        }
    }

    let mut lines = Vec::new();
    scan_dir(
        &EXEMPT_FILES,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut lines,
    );
    lines
}

/// `path:line: source` for every swept line a token sweep objects to.
fn violations<'a>(
    mut offending: impl FnMut(&str) -> bool,
    lines: impl IntoIterator<Item = &'a (std::path::PathBuf, usize, String)>,
) -> Vec<String> {
    lines
        .into_iter()
        .filter(|(_, _, line)| offending(line))
        .map(|(path, num, line)| format!("{}:{num}: {line}", path.display()))
        .collect()
}

/// True when a line gives a view a dimension of its own: a `const` of a
/// measured type declared outside the store.
///
/// A derived declaration counts (`MIN_INNER_H = PLAY_BTN + 16.0 + 8.0`) —
/// the arithmetic belongs beside the tokens it reads, which is where the
/// surfaces that share a control disagree today. What a view may still own
/// is what no designer would retune and no second surface reads: a `usize`
/// count of rows to ask its read model for, a `[f32; 4]` of shape data for
/// one hand-painted glyph, a texture's raster resolution.
fn view_owned_dimension(line: &str) -> bool {
    let Some(decl) = line
        .strip_prefix("const ")
        .or_else(|| line.strip_prefix("pub const "))
    else {
        return false;
    };
    decl.split_once(": ").is_some_and(|(_, typed)| {
        ["f32 =", "egui::Vec2 ="]
            .iter()
            .any(|ty| typed.starts_with(ty))
    })
}

/// True when `text` sets a non-zero number: a digit run that starts on a
/// word boundary, so the `2` in `vec2` contributes nothing and `0.0` reads
/// as the zero it is.
fn sets_a_nonzero_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let starts_a_literal =
            bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric());
        if !starts_a_literal {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && matches!(bytes[i], b'0'..=b'9' | b'.') {
            i += 1;
        }
        if text[start..i]
            .parse::<f32>()
            .is_ok_and(|value| value != 0.0)
        {
            return true;
        }
    }
    false
}

/// True when a line leaves a gap between items that the spacing scale does
/// not name. Zero stays allowed: setting `item_spacing` to nothing is how a
/// widget opts out of egui's layout so it can place its own sub-rectangles,
/// which is structure rather than design.
fn numeric_item_spacing(line: &str) -> bool {
    line.contains("item_spacing") && sets_a_nonzero_number(line)
}

/// No file outside the store derives an egui color.
#[test]
fn test_view_code_contains_no_hardcoded_color_literals() {
    let violations = violations(hardcoded_color_literal, &source_lines());
    assert!(
        violations.is_empty(),
        "view code must style itself from theme tokens (ADR 0004); \
         found hardcoded colors:\n{}",
        violations.join("\n")
    );
}

/// The color sweep flags derivations, not channel reads.
#[test]
fn test_the_color_sweep_flags_derivations_but_not_channel_reads() {
    // The hole the widening closed: a palette color scaled at a call site
    // decides a color outside the design system without naming a
    // constructor, so it must fail the sweep — as must a literal.
    for derives in [
        "    let fill = palette.error.gamma_multiply(0.1);",
        "    let wash = palette.ink_3.gamma_multiply(0.4);",
        "    let ring = theme::INK.linear_multiply(0.5);",
        "    let edge = palette.border.gamma_multiply_u8(200);",
        "    let flat = Color32::from_rgb(0, 0, 0);",
    ] {
        assert!(hardcoded_color_literal(derives), "sweep misses {derives}");
    }
    // Reading a color's channels composes nothing, and `blend_over` is the
    // helper the token module exposes for composing them: both stay clean,
    // or the sweep would be arguing with the design system instead of
    // protecting it.
    for benign in [
        "    let key = (icon, px, color.r(), color.g(), color.b());",
        "    if px.a() == 0 { continue; }",
        "    image.pixels[i] = theme::blend_over(image.pixels[i], *px);",
        "    painter.galley(pos, galley, palette.ink);",
    ] {
        assert!(
            !hardcoded_color_literal(benign),
            "sweep false-positives {benign}"
        );
    }
}

/// The geometry half of the same claim as the color sweep: `theme.rs` is
/// the store, so a view that declares a measured constant has taken a
/// design value into its own hands.
#[test]
fn test_view_code_declares_no_dimensions_of_its_own() {
    let violations = violations(view_owned_dimension, &source_lines());
    assert!(
        violations.is_empty(),
        "component geometry belongs in theme.rs's `geometry` section, next \
         to the surface that paints it (ADR 0004); found view-owned \
         dimensions:\n{}",
        violations.join("\n")
    );

    for declares in [
        "const ROW_H: f32 = 40.0;",
        "pub const HEADER_H: f32 = 28.0;",
        "const MIN_INNER_H: f32 = PLAY_BTN + 16.0 + 8.0;",
        "pub const MIN_STAGE_SIZE: egui::Vec2 = egui::vec2(520.0, 456.0);",
    ] {
        assert!(view_owned_dimension(declares), "sweep misses {declares}");
    }
    for owns in [
        // What the rule deliberately does not reach: a read-model bound,
        // one glyph's shape data, a raster resolution, a token module's
        // own `const fn`, and a plain local.
        "pub const UP_NEXT_LIMIT: usize = 5;",
        "const WORDMARK_BARS: [f32; 4] = [0.55, 0.95, 0.7, 0.4];",
        "const TILE_PX: usize = 256;",
        "pub const fn corner(radius: f32) -> CornerRadius {",
        "    let row_h = theme::geometry::sidebar::ROW_H;",
    ] {
        assert!(!view_owned_dimension(owns), "sweep false-positives {owns}");
    }
}

/// The spacing half: a gap between items is a scale step, so a numeric
/// `item_spacing` assignment in a view is a gap the design system has
/// never heard of.
#[test]
fn test_view_code_sets_no_spacing_of_its_own() {
    let violations = violations(numeric_item_spacing, &source_lines());
    assert!(
        violations.is_empty(),
        "spacing between items comes from theme's SPACE_* scale (ADR 0004); \
         found numeric item_spacing assignments:\n{}",
        violations.join("\n")
    );

    for sets in [
        "ui.spacing_mut().item_spacing.x = 6.0;",
        "ui.spacing_mut().item_spacing.x = 10.0;",
        "ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);",
    ] {
        assert!(numeric_item_spacing(sets), "sweep misses {sets}");
    }
    for reads in [
        "ui.spacing_mut().item_spacing.x = 0.0;",
        "strip_ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);",
        "ui.spacing_mut().item_spacing.x = theme::SPACE_MD;",
    ] {
        assert!(
            !numeric_item_spacing(reads),
            "sweep false-positives {reads}"
        );
    }
}
