---
name: Bug report
about: Something in riff does not do what it is supposed to do
title: "[Bug]: "
labels: ["bug"]
assignees: []
---

<!-- riff is pre-1.0 and no packaged build has been published, so "version" below means the commit you built. -->

**What are you running?** The commit SHA is most useful — `git rev-parse --short HEAD`. There are no release tags yet, so `git describe` will not name a version. If you built from a fork, say which.

**Which OS, and which version?** For example: Windows 11 24H2, macOS 15.3, Fedora 41, Debian 13. If the audio device or backend seems relevant (ALSA, WASAPI, CoreAudio), include it.

**What did you do?** The steps, starting from a fresh launch if you can. Which view you were in, which button you pressed, which keyboard shortcut.

**What did you expect to happen?**

**What happened instead?** The exact error text, panic message, or wrong behavior. Paste it rather than paraphrasing.

**Is a library scan involved?** A bug in scanning is a different bug from a bug in playback, and it is worth saying which one you hit. Specifically:

- Did the problem appear during or after a scan, on a freshly added folder, or only after restarting?
- Is a scan status or error message showing in the UI? If so, copy it exactly — it is usually the most diagnostic thing in the report.
- Roughly how many tracks and folders are in the library?

**Is a specific track involved?** The format (MP3, FLAC, AAC/M4A, Opus, OGG, WAV) and rough file size. You do not need to attach the file — describing it is enough, and it is better not to attach real media. If it is reproducible with a different file, that is worth saying.

**Anything else?** Console output or log lines, whether it is reproducible every time or only sometimes, and whether it started after a specific commit or dependency update.

If this is a suspected security problem rather than a bug, do not post it here — use the private route in [SECURITY.md](SECURITY.md).
