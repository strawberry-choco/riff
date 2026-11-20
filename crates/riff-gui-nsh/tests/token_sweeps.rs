//! The ADR 0004 token-authority sweep, homed with the source it judges and
//! deferred here from N1 (`docs/execution-plans/evidence/nsh/n1/decisions.tsv`
//! row 2026-11-20T12:13:32Z): a sweep over a crate with no design values yet
//! passes vacuously, so it lands with the theme port that gives it teeth.
//!
//! The model is `crates/riff-gui-theme/tests/token_sweeps.rs`, adapted to
//! the nsh vocabulary: this file sweeps **this crate's `src`** for what a
//! reader of the design system must not do — construct or derive an nsh
//! color, declare a measured dimension of its own, or set a spacing gap by
//! number. The store files (`src/theme/`) are the exempt ones, exactly as
//! `theme.rs` was the one exempt file in the egui crates.
//!
//! Non-vacuous by construction: the classifiers are self-tested below, and
//! the scan is asserted to have covered the non-store sources.

use std::path::Path;

/// True when a code line constructs or derives an nsh color instead of
/// reading one: a packed literal (`rgba(0x…)` / `rgb(0x…)` / `Color(0x…)` /
/// `hsla(…)`), a packed-to-Hsla conversion (`to_hsla`), or one of the color
/// math methods ([`Hsla`]'s `Colorize` set and the alpha operators) that
/// turn a read token into a new color at the call site.
fn color_construction(line: &str) -> bool {
    for marker in ["rgba(0x", "rgb(0x", "Color(0x", "hsla(", "to_hsla("] {
        if line.contains(marker) {
            return true;
        }
    }
    [
        ".opacity(",
        ".alpha(",
        ".lighten(",
        ".darken(",
        ".mix(",
        ".saturate(",
        ".spin(",
    ]
    .iter()
    .any(|derives| line.contains(derives))
}

/// True when a line declares a measured constant outside the store: the
/// `f32` dimension tokens live in `tokens.rs` alone, on the egui store's
/// rule that a value a designer would retune is declared once, beside its
/// family.
fn dimension_declaration(line: &str) -> bool {
    let Some(decl) = line
        .strip_prefix("const ")
        .or_else(|| line.strip_prefix("pub const "))
    else {
        return false;
    };
    decl.split_once(": ")
        .is_some_and(|(_, typed)| typed.starts_with("f32 ="))
}

/// True when `text` sets a non-zero number: a digit run that starts on a
/// word boundary, so the `2` in `px_2` contributes nothing and `0.0` reads
/// as the zero it is. (The egui sweep's own predicate, carried over.)
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

/// The spacing setters a view uses on nsh, as their call-shape prefixes:
/// `gap(px(8.0))` and friends. A view leaves its gaps to the spacing scale
/// (`px(tokens::SPACE_MD)`), never to a number.
const SPACING_CALLS: [&str; 14] = [
    ".gap(px(",
    ".gap_x(px(",
    ".gap_y(px(",
    ".p(px(",
    ".px(px(",
    ".py(px(",
    ".pt(px(",
    ".pb(px(",
    ".pl(px(",
    ".pr(px(",
    ".m(px(",
    ".mx(px(",
    ".my(px(",
    ".inset(px(",
];

/// True when a line sets one of the spacing setters to a numeric literal
/// rather than a token read: the number must start right at the call's
/// argument, so `px(tokens::SPACE_MD)` passes and `px(8.0)` fails.
fn numeric_spacing(line: &str) -> bool {
    SPACING_CALLS
        .iter()
        .any(|call| line.contains(call) && sets_a_nonzero_number(&line[call.len()..]))
}

/// Every `(path, line number, trimmed source)` triple the sweeps judge:
/// `.rs` files under this crate's `src`, minus the store files, minus
/// comment lines, so prose may name the APIs a sweep bans.
fn source_lines() -> Vec<(std::path::PathBuf, usize, String)> {
    /// The store files the sweeps skip: the ported design system itself.
    const EXEMPT_FILES: [&str; 4] = ["palette.rs", "tokens.rs", "fonts.rs", "icons.rs"];

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
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
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

/// No file outside the store constructs or derives an nsh color.
#[test]
fn test_view_code_contains_no_color_constructions() {
    let violations = violations(color_construction, &source_lines());
    assert!(
        violations.is_empty(),
        "view code must style itself from the palette signal and tokens (ADR 0004); \
         found color constructions:\n{}",
        violations.join("\n")
    );
}

/// No file outside the store declares a measured dimension of its own.
#[test]
fn test_view_code_declares_no_dimensions_of_its_own() {
    let violations = violations(dimension_declaration, &source_lines());
    assert!(
        violations.is_empty(),
        "dimension tokens are declared in the store, not in a view (ADR 0004); \
         found dimension declarations:\n{}",
        violations.join("\n")
    );
}

/// No file outside the store sets a spacing gap by number.
#[test]
fn test_view_code_sets_no_spacing_by_number() {
    let violations = violations(numeric_spacing, &source_lines());
    assert!(
        violations.is_empty(),
        "spacing gaps read the spacing scale (ADR 0004); found numeric gaps:\n{}",
        violations.join("\n")
    );
}

/// The color sweep flags derivations, not channel reads.
#[test]
fn test_the_color_sweep_flags_constructions_and_derivations() {
    for constructs in [
        "    let fill = rgba(0xff5252ff);",
        "    let wash = rgb(0xff5252);",
        "    let packed = Color(0x00000073);",
        "    let hsl = hsla(0.1, 0.8, 0.5, 1.0);",
        "    const RING: Hsla = rgba(0xa78bfaff).to_hsla();",
        "    let dim = palette.ink.opacity(0.5);",
        "    let lifted = palette.surface_3.lighten(0.1);",
    ] {
        assert!(color_construction(constructs), "sweep misses {constructs}");
    }
    // Reading a token, comparing it, or naming it in prose stays clean —
    // the sweep protects the store, it does not argue with it.
    for benign in [
        "    .bg(move || palette.get().surface)",
        "    .text_color(move || palette.get().ink_3)",
        "    assert_eq!(p.brand_primary, palette::BRAND_500);",
        "    if p.dark { shadow_dark } else { shadow_light }",
    ] {
        assert!(
            !color_construction(benign),
            "sweep false-positives {benign}"
        );
    }
}

/// The spacing sweep flags numeric gaps, not token reads or tailwind steps.
#[test]
fn test_the_spacing_sweep_flags_numeric_gaps_only() {
    for offender in [
        ".gap(px(8.0))",
        "        .p(px(12.0 + tokens::SPACE_MD))",
        "        .py(px(4.0))",
    ] {
        assert!(numeric_spacing(offender), "sweep misses {offender}");
    }
    for benign in [
        ".gap(px(tokens::SPACE_MD))",
        ".p(px(tokens::SPACE_LG))",
        ".gap_4()",
        ".gap_x(px(tokens::SPACE_SM))",
        // Comment lines are dropped by the scan before the classifier runs;
        // `.0` field accesses and zero values read as the structure they are.
        ".my(px(0.0))",
    ] {
        assert!(!numeric_spacing(benign), "sweep false-positives {benign}");
    }
}

/// The dimension sweep flags measured constants, not the store's or a
/// view's non-design locals.
#[test]
fn test_the_dimension_sweep_flags_f32_consts_only() {
    for offender in ["pub const HERO_W: f32 = 240.0;", "const BAR_H: f32 = 88.0;"] {
        assert!(dimension_declaration(offender), "sweep misses {offender}");
    }
    for benign in [
        "pub const ALL: [Variant; 4] = [",
        "const ICON_SIZE: f32;",
        "    let scale = 2.0;",
        // The store's own tokens are exempt at the file level (the scan
        // skips tokens.rs), and non-design consts stay clean.
        "pub const SAMPLES: usize = 4;",
    ] {
        assert!(
            !dimension_declaration(benign),
            "sweep false-positives {benign}"
        );
    }
}

/// The sweep is non-vacuous: the scan must have covered the non-store
/// sources this crate ships, so an empty or misplaced scan cannot green the
/// three sweeps above.
#[test]
fn test_the_scan_covers_the_non_store_sources() {
    let lines = source_lines();
    let covered: std::collections::BTreeSet<String> = lines
        .iter()
        .map(|(path, _, _)| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    for name in ["mod.rs", "app.rs", "dev_tokens.rs"] {
        assert!(covered.contains(name), "the scan never saw {name}");
    }
    for store in ["palette.rs", "tokens.rs", "fonts.rs", "icons.rs"] {
        assert!(
            !covered.contains(store),
            "the exempt store file was swept: {store}"
        );
    }
    assert!(
        lines
            .iter()
            .any(|(path, _, _)| path.ends_with("screens/dev_tokens.rs")),
        "the fixture screen must be among the swept sources"
    );
}
