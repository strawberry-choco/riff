# 12 — Land ReplayGain normalization

**What to build:** Consistent loudness across differently mastered tracks: riff reads existing ReplayGain tags and applies the gain during playback, capped so amplified tracks never clip — and leaves untagged tracks untouched. The implementation already exists; verify it against the acceptance criteria, fix any gaps, and commit it.

**Blocked by:** 01 — Baseline green gate.

**Status:** ready-for-agent

- [ ] Track and peak ReplayGain tags are read from audio metadata during scanning
- [ ] When enabled in settings, tagged tracks play with the gain applied at the volume-scaling stage; gain is peak-capped so amplification cannot clip
- [ ] Tracks without ReplayGain tags play with no gain adjustment
- [ ] The settings toggle enables/disables normalization and changes take effect for subsequent playback
- [ ] Tests cover tag parsing (dB string variants) and gain-factor computation including the peak cap; feature committed
