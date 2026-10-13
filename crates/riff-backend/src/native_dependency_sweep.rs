//! The native-dependency sweep: `riff-backend` declares no native dependency,
//! so no symbol of its own may name a symphonia type.
//!
//! A symphonia type appearing in this crate means the application layer holds
//! a value it must build and rebuild — the `CodecRegistry` the composition
//! root used to construct by hand, which was also not `Clone`, so the decoder
//! factory rebuilt it for every decoder it minted. That fact now lives in
//! `riff-infra`'s decoder, beside the registry it builds.
//!
//! This is the same kind of sweep the design-token rules use: it reads this
//! crate's own source, skips comment lines (prose may name what it bans, as
//! the paragraph above does), and names the offending line so the fix is to
//! move the fact into the adapter — never to widen the sweep.

#[cfg(test)]
mod tests {
    /// Symphonia-owned names: the crate path and the types the composition
    /// root once held. `SymphoniaDecoder` is deliberately absent — it is a
    /// `riff-infra` adapter name, not a symphonia type.
    const BANNED: &[&str] = &[
        "symphonia",
        "CodecRegistry",
        "codec_registry",
        "CodecParameters",
        "AudioCodecId",
        "default_codec_registry",
    ];

    /// The file the sweep itself lives in: its banned list necessarily spells
    /// every name it bans, so it is exempt — every other file in the crate is
    /// judged, `lib.rs` included.
    const SWEEP_FILE: &str = "native_dependency_sweep.rs";

    #[test]
    fn no_symbol_names_a_symphonia_type() {
        fn scan(dir: &std::path::Path, violations: &mut Vec<String>) {
            let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
                .expect("the crate's source tree must be readable")
                .map(|entry| entry.expect("directory entries must resolve").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    scan(&path, violations);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && path.file_name().and_then(|name| name.to_str()) != Some(SWEEP_FILE)
                {
                    let source =
                        std::fs::read_to_string(&path).expect("source files must be UTF-8");
                    for (index, line) in source.lines().enumerate() {
                        let trimmed = line.trim_start();
                        if trimmed.starts_with("//") {
                            continue;
                        }
                        if BANNED.iter().any(|banned| trimmed.contains(banned)) {
                            violations.push(format!("{}:{}: {trimmed}", path.display(), index + 1));
                        }
                    }
                }
            }
        }

        let mut violations = Vec::new();
        scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut violations,
        );
        assert!(
            violations.is_empty(),
            "riff-backend declares no native dependency, so no symbol of its own \
             may name a symphonia type; move the fact into riff-infra:\n{}",
            violations.join("\n")
        );
    }
}
