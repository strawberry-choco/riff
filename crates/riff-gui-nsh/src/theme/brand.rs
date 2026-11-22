//! The riff brand mark: the wordmark's static equalizer glyph.
//!
//! One SVG file is the source for both places the mark must appear — the
//! titlebar's mark and the tray icon — so the two cannot drift. The geometry
//! is the egui strip's: four bars at `[0.55, 0.95, 0.7, 0.4]`, a 10% inset
//! and a radius of half the bar width, in `BRAND_500`.
//!
//! The markup is compiled in rather than read through an `asset:/` URI: a
//! dev `cargo run` has no asset base, which is why the tray's old PNG lookup
//! failed there. `assets/brand/riff-1024.png` stays for `nsh package`, which
//! turns it into the window icon.

/// The vendored brand mark, as SVG markup.
pub const MARK_SVG: &str = include_str!("../../assets/brand/mark.svg");

#[cfg(test)]
mod tests {
    /// The mark is the four-bar equalizer, not a generic glyph. Every bar is
    /// a rounded rect and the whole file carries the brand hue.
    #[test]
    fn the_mark_is_the_four_bar_equilibrium() {
        assert_eq!(super::MARK_SVG.matches("<rect").count(), 4);
        assert!(super::MARK_SVG.contains(r##"fill="#f0821e""##));
        assert_eq!(super::MARK_SVG.matches(r#"rx="64""#).count(), 4);
    }
}
