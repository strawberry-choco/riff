//! The N1 scaffold's guards: the brand mark ships, `nsh.json` agrees with
//! the bootstrap, the manifest keeps the nsh frontend out of egui, and the
//! root carries the theme file. Every assertion is over committed files and
//! pure registry metadata — no engine, no window.

use std::path::Path;

use nsh_core::JsonValue;
use nsh_core::element::{AssetKind, Root};

#[test]
fn the_registry_ships_the_brand_mark() {
    let registry = riff_gui_nsh::registry();
    let mark = registry
        .asset_files()
        .find(|file| file.uri == "asset:/@riff-gui-nsh/brand/riff-1024.png")
        .expect("the registry ships the brand mark");
    assert_eq!(mark.kind, AssetKind::Image);
    assert!(
        Path::new(mark.source).is_file(),
        "the mark's source must be a file: {}",
        mark.source
    );
}

#[test]
fn nsh_json_agrees_with_the_bootstrap() {
    let json: JsonValue = include_str!("../nsh.json")
        .parse()
        .expect("nsh.json parses");
    assert_eq!(json["name"].as_str(), Some(riff_gui_nsh::APP_NAME));
    assert_eq!(json["id"].as_str(), Some(riff_gui_nsh::APP_ID));
    assert_eq!(json["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(json["root"].as_str(), Some("App"));
    assert_eq!(json["rust"].as_str(), Some("."));
    assert!(json["build"].as_u64().is_some_and(|build| build > 0));

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let icon = json["icon"].as_str().expect("nsh.json declares an icon");
    assert!(
        dir.join(icon).is_file(),
        "the declared icon must exist: {icon}"
    );
    if let Some(theme) = json["theme"].as_str() {
        assert!(
            dir.join(theme).is_file(),
            "the declared theme must exist: {theme}"
        );
    }
}

#[test]
fn the_manifest_keeps_the_nsh_frontend_out_of_egui() {
    // The one `riff-*` edge this frontend may ever name; N3 adds it with its
    // first `use riff_backend::`, so the allowlist needs no edit then.
    const ALLOWED_RIFF_DEPS: [&str; 1] = ["riff-backend"];

    let mut offenders = Vec::new();
    let mut in_dependencies = false;
    for (num, line) in include_str!("../Cargo.toml").lines().enumerate() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line.ends_with("dependencies]");
            if line.starts_with("[profile.") {
                offenders.push(format!("line {}: {line}", num + 1));
            }
            continue;
        }
        if !in_dependencies || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let token = line.split(['.', '=', ' ', '\t']).next().unwrap_or("");
        let egui = token == "egui" || token.starts_with("egui_") || token.starts_with("egui-");
        let riff = token.starts_with("riff-") && !ALLOWED_RIFF_DEPS.contains(&token);
        if egui || riff {
            offenders.push(format!("line {}: {line}", num + 1));
        }
    }
    assert!(
        offenders.is_empty(),
        "the nsh frontend names no egui crate, no riff-* crate but riff-backend, \
         and no profile table:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_root_carries_the_theme_file() {
    let root = Root::of::<riff_gui_nsh::App>().tokens(riff_gui_nsh::theme_tokens());
    assert!(
        root.tokens_ref().is_some(),
        "the root must carry the theme file's tokens"
    );
}
