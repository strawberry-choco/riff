# ReplayGain 2.0 via BS.1770, Never 1.0

**Status**: Accepted
**Date**: 2026-10-04

riff measures loudness itself (the ReplayGain Pass), and the measurement has to be one specific thing, decided once: **ReplayGain 2.0** — BS.1770 integrated loudness against a **−18 LUFS** reference, with **true-peak** measurement for clipping prevention. ReplayGain 1.0 is never produced.

BS.1770 is what every modern tool converged on: foobar2000, rsgain, and loudgain all write RG 2.0 values when asked, and −18 LUFS is the reference both ReplayGain 2.0 and the EBU R128 convention share, which is also what makes the Opus R128 tag conversion (RFC 7845) a pure re-encoding of the same number rather than a second measurement. ReplayGain 1.0 would mean keeping a second, incompatible formula (A-weighting, no gating) alive for no listener who asked for it.

The implementation is the pure-Rust [`ebur128` crate](https://crates.io/crates/ebur128) behind the library capability's `LoudnessAnalyzer` port, implemented in `riff-infra` over the same decode stack playback uses. One decode of a Track yields its track gain (−18 − integrated loudness) and its true peak (4× oversampled, so inter-sample overshoots are caught); the album aggregate is the energy-weighted mean of the members' loudnesses weighted by duration, and the album peak is the maximum member peak — pure math in `riff_library::app::replaygain`, not inside the adapter, because it recomputes from Store facts without re-decoding.

## Considered Options

- **ReplayGain 1.0 (rejected)**: the legacy formula still rides inside many old tags, and riff keeps *reading* those (the gain-string parser accepts whatever the file carries). But producing 1.0 would bake an obsolete psychoacoustic model into new files, and the values do not interoperate with the R128-convention Opus tags the RFC mandates.
- **A different reference than −18 LUFS (rejected)**: ReplayGain 2.0's reference is −18 LUFS and the R128 convention hardcodes it; any other reference would make riff's written tags measure differently in every other player.
- **Sample peak instead of true peak (rejected)**: sample peak misses inter-sample overshoots, which is exactly the case clipping prevention exists for; the cost is one 4× oversampling filter pass inside the meter.

## Consequences

- `ebur128` becomes a direct dependency of `riff-infra` (the adapter crate, per ADR 0009's membership rule).
- The `TrackLoudness` a measurement yields, and the `album_aggregate` that combines measurements, are the two building blocks the ReplayGain Pass composes; neither re-reads a file.
- Files tagged in the 1.0 convention keep playing — the reader accepts whatever gain strings they carry — but riff never writes a 1.0 value, so a measured file gains a 2.0 tag set next to whatever it had.
