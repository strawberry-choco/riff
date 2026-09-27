<h1 align="center">riff</h1>

<h3 align="center">A quiet music player for the collection you already own.</h3>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="Apache 2.0 license"></a>
  <a href="https://github.com/strawberry-choco/riff/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/strawberry-choco/riff/ci.yml?branch=master&label=CI" alt="Build status"></a>
</p>

<p align="center">
  <img src="https://github.com/user-attachments/assets/b337f3b2-969d-4b01-9102-62586a71cab3" alt="riff's library: artists, albums and tracks in one row of columns, with cover art and a detail inspector" width="900">
  <br>
  <em>riff browsing a demo library — the artists and albums in these screenshots are invented for testing.</em>
</p>

riff plays the music on your disks. Point it at your folders — an internal drive, an external SSD, a share mounted from the NAS — and it builds one library out of all of them: cover art, tags, search, playlists, and standard transport controls. No account, no upload, no subscription, and no network access at all.

It is built for people who treat their music as a collection: the ones who tag their own files, who keep a playlist for every mood, and who would rather not create an account to play a FLAC.

---

## Get riff

**There is nothing to download yet.** riff is pre-1.0 and no packaged build has been published, so the only way to run it is to [build it from source](#building-from-source) — a stable Rust toolchain and, on Linux, the development headers listed in [CONTRIBUTING.md](CONTRIBUTING.md). The everyday path works; the rough edges are named below.

**First run, in three moves:**

1. Click **Add folder** at the foot of the sidebar and pick the directory your music lives in. Add as many as you have.
2. Let the first scan finish — a few seconds for a folder, longer for a big archive, and the library is usable the whole time.
3. Press play.

## What it's like

**Your files stay your files.** riff indexes what is on disk. It never copies, converts, renames, or reorganizes anything, and removing a library folder deletes only the index entries — not a single byte of your collection.

**It notices what you add.** Turn on folder watching and a new album you drop into a directory indexes itself a couple of seconds later. Delete a file and it leaves the index too, so search never points at something that isn't there.

**It opens instantly.** The library is indexed once and kept in a local store, so a launch lands on a browsable collection instead of re-walking every drive you own.

**Browse the way you think.** Artists into albums into tracks. Genres into artists into albums into tracks. Or the Folders view, which mirrors your disk exactly. One search box — `Ctrl+K` — finds an artist, an album, or a track from anywhere in the app.

**Playlists that hold their own.** Name one, add tracks from any listing, reorder it, rename it. A track whose file has gone missing stays visible, struck through, and is skipped on playback rather than breaking the list.

**Lists that build themselves.** The sidebar's discovery lists are computed on your machine from what you have actually listened to. [docs/product/features.md](docs/product/features.md) is the canonical feature catalog for them.

**Made for continuous listening.** Gapless playback across an album, and opt-in `ReplayGain` so a quiet folk pressing and a hot master sit at the same level inside the same playlist.

**Fix a tag without leaving the app.** Right-click a track and correct its title, artist, album, genre, year, or track number. riff writes the tag back into the file and updates the library without a rescan; an album's readout edits its whole set of tracks at once.

**Compilations stay where you put them.** Albums group by *album artist*, so a various-artists collection reads as one album instead of scattering across the library.

**Out of the way.** Playback carries on with the window hidden, and the tray menu holds play/pause, next, and previous. The whole app works from the keyboard, with a visible focus ring and a high-contrast theme.

**Plays:** MP3 · FLAC · AAC (M4A) · Opus · OGG Vorbis · WAV

<p align="center">
  <img src="https://github.com/user-attachments/assets/b4eafcde-66cc-41f5-9273-762f39457cee" alt="A playlist called Night Drive open in riff, with the queue panel showing what is up next" width="440">
  &nbsp;&nbsp;
  <img src="https://github.com/user-attachments/assets/6835b10e-54f6-4032-92fd-e11976a011d3" alt="riff's Now Playing view: large cover art, full metadata, and the up-next list" width="440">
</p>

## Who riff is not for

Saying this plainly saves everyone a conversation:

- **No streaming.** No Spotify, no online catalog, no internet radio, no HTTP streams.
- **No cloud and no sync.** Your library lives on one machine. A NAS share works only because your operating system shows it as a folder.
- **No accounts, no telemetry, no phone-home.** There is nothing to turn off, because none of it exists.
- **No artwork or metadata lookup.** Tags and covers come from your files, the way you wrote them.
- **No DRM files, no CDs, no conversion.** riff decodes; it does not encode, rip, or burn.

And what a curator will miss on day one, because it is genuinely not built yet: the full catalog of what ships and what is deliberately deferred lives in [docs/product/features.md](docs/product/features.md), § Deferred / Future.

## Your listening stays yours

Everything riff knows, it keeps in one file in your own application-data folder: the indexed library, your playlists, your play history, and your settings. Nothing leaves your machine, and nothing needs to. Delete that file and riff starts over from your folders — the collection on disk is the record, not the database.

## Platform notes

| Platform | What is different |
|---|---|
| **Windows, macOS** | System tray: closing the window keeps playback running, and the tray menu carries transport and Quit. Native folder picker. |
| **Linux** | No tray, by design — closing the window quits. Adding a folder is a validated text field with autocomplete rather than a native dialog. |

## Building from source

riff is a Rust workspace. With a recent stable toolchain:

```bash
git clone https://github.com/strawberry-choco/riff
cd riff
cargo run -p riff-gui
```

## Contributing

**[CONTRIBUTING.md](CONTRIBUTING.md) is the front door** — it has the prerequisites, the exact build and verify commands, the pull request checklist, and the honest warning that the committed golden-image baselines are byte-exact and can fail on different hardware.

Bug reports and feature requests go to [the issue tracker](https://github.com/strawberry-choco/riff/issues). [docs/README.md](docs/README.md) indexes the product specs, the architecture reference, and the engineering conventions the project holds itself to.

The design has a hard boundary worth knowing about: riff stays offline. Features that would need a server, an account, or a network call are out of scope, and [docs/product/decisions/001-offline-first.md](docs/product/decisions/001-offline-first.md) records why.

## License

riff is licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).

It ships with the [Inter](crates/riff-gui/assets/fonts/Inter-LICENSE-OFL.txt) typeface (SIL Open Font License) and [Lucide](crates/riff-gui/assets/icons/LICENSE-Lucide.txt) icons (ISC license). Both are embedded in the binary, so the fonts and icons travel with the executable.
