# 10 — Land gapless playback

**What to build:** Consecutive album tracks flow into each other with no audible gap: the next track is decoded and buffered before the current one ends and handed off seamlessly. This is the riskiest slice of v0.2.0 — the implementation already exists in the working tree; verify it carefully against the acceptance criteria with real audio, fix any gaps, and commit.

**Blocked by:** 01 — Baseline green gate; 07 — Land player control bar + Now Playing view (transport behavior must be settled first).

**Status:** ready-for-agent

- [ ] Consecutive tracks from the same album transition with no audible silence; the successor is pre-decoded and buffered before the current track ends
- [ ] Works across all supported formats (MP3, FLAC, AAC, Opus, WAV, OGG Vorbis); ineligible or mismatched handoffs fall back to the normal gapped path rather than glitching
- [ ] Shuffled playback, manual next/previous, pause, stop, and seek all behave exactly as before — gapless only applies to consecutive queue progression
- [ ] Repeat-one loops the same track seamlessly
- [ ] Memory stays bounded across long listening sessions (pre-buffer has a cap; no growth with queue length)
- [ ] Track-end handoff is reliable — no premature or late transitions, and no duplicated track-start events after a handoff
- [ ] Decision logic (eligibility, format compatibility, pre-buffer cap) is covered by tests; feature committed
