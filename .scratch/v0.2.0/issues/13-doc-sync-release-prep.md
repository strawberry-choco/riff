# 13 — Documentation sync and v0.2.0 release prep

**What to build:** The documentation tells the truth about what shipped, and the release build is proven: feature catalog statuses match the implemented reality, the roadmap reflects completion, the execution todo is closed out, and a release build succeeds — leaving the repo in a state ready to tag v0.2.0.

**Blocked by:** all tickets 02 through 12 (every landed feature must be verified and committed first).

**Status:** done

- [ ] The feature catalog statuses match shipped reality: player control bar (stop button, mute), Now Playing view (up-next play next, in-view progress), system tray (close-to-tray), and all new v0.2.0 features (tag writing, smart playlists, playlists, gapless, ReplayGain, progressive disclosure, accessibility, cache hardening) are marked as implemented with accurate summaries
- [ ] The roadmap marks v0.2.0 items complete and still-deferred items remain explicitly deferred
- [ ] The execution todo is updated to reflect completion (or points at this ticket set as its successor)
- [ ] `cargo build --release` succeeds on Windows and Linux (CI), and the release binary starts and plays audio
- [ ] All v0.2.0 work is committed; no stray uncommitted implementation changes remain; the repo is ready for a v0.2.0 tag
