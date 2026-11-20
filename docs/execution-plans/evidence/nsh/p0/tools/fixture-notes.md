# NSH P0 fixture notes

Synthetic music library for the frontend-rewrite feasibility spike. Everything
here is generated. Nothing reads or writes the user's real music library or the
live Application Store in the OS data-local dir. The generator only writes under
its `--root`, which defaults to `.scratch/nsh-p0/library`.

## Regenerate

```bash
python .scratch/nsh-p0/make_fixture.py                          # 50,000 tracks, 500 albums
python .scratch/nsh-p0/make_fixture.py --tracks 200 --albums 4 --root .scratch/nsh-p0/smoke
python .scratch/nsh-p0/verify_fixture.py                        # independent check of the full tree
python .scratch/nsh-p0/verify_fixture.py --root .scratch/nsh-p0/smoke --tracks 200 --albums 4
```

Flags. `--tracks N --albums N` (N divisible by albums), `--seconds S` (audio
length per track, default 3), `--workers W` (write threads, default 8),
`--root PATH`. The verifier takes the same shape flags plus `--sample`.

Reruns converge. Each file is compared by content against the deterministic
payload and kept when identical, else replaced via a temp file and `os.replace`.
A rerun with a smaller shape deletes the surplus `*.wav` and `cover.jpg` files
and prunes empty directories, so smoke and full runs can share one root. Files
that match neither pattern are never touched.

## Tag format: WAV with an ID3v2.3 tag in an `id3 ` RIFF chunk

The container is canonical PCM WAV, 8 kHz mono 16-bit, the same byte layout as
the repo's established fixture helper
(`crates/riff-backend/examples/scan_bench.rs:33-55`,
`crates/riff-infra/tests/adapter_tests.rs:47`). The scanner indexes `.wav`
(`crates/riff-persistence/src/store.rs:30`). The tag rides in a RIFF chunk with
fourcc `id3 `, placed between `fmt ` and `data`.

Why ID3v2 and not RIFF INFO. RIFF INFO has no album-artist key at all
(lofty-0.25.4 `src/tag/item.rs:356-378`, the `RIFF_INFO_MAP`), and the store keys
albums on `PRIMARY KEY (album_artist, title)`
(`crates/riff-infra/src/store/sqlite.rs:194`). An INFO-only fixture would index
500 albums under a NULL album artist. ID3v2 carries every field the spike needs.

The parse chain, each link read in the vendored source this session
(registry root `C:\Users\stink\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\`):

1. lofty's WAV reader parses `ID3 ` and `id3 ` chunks into an `Id3v2Tag`
   (`lofty-0.25.4/src/iff/wav/read.rs:97-114`).
2. WAV's primary tag type is `Id3v2`
   (`lofty-0.25.4/src/file/file_type.rs:140`), and riff's reader takes
   `primary_tag()` first (`crates/riff-infra/src/media/metadata_reader.rs:111-115`),
   so the ID3v2 tag wins over any INFO list.
3. Frame to ItemKey mapping (`lofty-0.25.4/src/tag/item.rs`): TALB 172, TIT2 176,
   TPE2 186, TPE1 188, TRCK 202, TDRC 206, TCON 239.
4. riff consumes exactly those ItemKeys
   (`crates/riff-infra/src/media/metadata_reader.rs:125-145`): TIT2 title, TPE1
   artist, TALB album, TPE2 album artist, TRCK track number, TCON genre. Year is
   the leading four digits of TDRC via `RecordingDate`
   (`metadata_reader.rs:151-155` and `year_from_text` at 78-89).
5. Duration comes from lofty's WAV properties, data-chunk length over byte rate
   (`lofty-0.25.4/src/iff/wav/properties.rs:257-260`, surfaced at
   `metadata_reader.rs:221-228`).

Playback safety. symphonia's RIFF chunk reader ignores unknown fourccs
(`symphonia-format-riff-0.6.1/src/common.rs:117-121`) and aligns chunks to even
boundaries (`common.rs:75-79`), so the `id3 ` chunk is skipped and the PCM
decodes. It requires `fmt ` before `data`
(`symphonia-format-riff-0.6.1/src/wave/mod.rs:152-155`); the writer emits that
order. symphonia is built with `features = ["all"]`
(`crates/riff-infra/Cargo.toml:37`).

Writer details that matter. ID3v2.3, not 2.4, so frame sizes are plain
big-endian u32 while the tag header size is syncsafe. Text frames use encoding
byte 0 (ISO-8859-1) and all names are ASCII. TRCK is a bare integer string,
never `N/total`, because riff parses it with `s.parse::<u32>()`
(`metadata_reader.rs:138`). TPOS is `1`. No unsynchronisation, no extended
header, no padding.

## Cover art: one `cover.jpg` per album directory

Discovery is folder-level, not embedded. For a track, the resolver reads
embedded art only when the Settings "Read embedded artwork" toggle allows it,
then falls back to probing the track's parent directory
(`crates/riff-library/src/app/cover_resolver.rs:79-95` and `178-184`) against
`TRACK_COVER_NAMES` (`cover_resolver.rs:13-26`), where `cover.jpg` is first
priority. A Folders-tree row probes its own directory against
`FOLDER_COVER_NAMES = ["cover.jpg", "cover.jpeg", "cover.png"]`
(`cover_resolver.rs:40`, `105-112`). Matching is case-insensitive
(`cover_resolver.rs:209-233`). The loader accepts only JPEG and PNG
(`crates/riff-infra/src/media/cover_loader.rs:90-97`).

Folder art was chosen over embedded APIC frames for three reasons. It works
with the embedded-art toggle off. It is 500 files instead of 50,000 embedded
copies. The Thumbnail cache keys on the source file, so all 100 tracks of an
album share one stored ladder (`cover_resolver.rs:123-130`).

Images are 300x300 JPEG quality 90, generated with Pillow (installed, version
12.3.0). Each album gets a golden-angle hue gradient and its 1-based album
number drawn centered, so all 500 are distinct by bytes and by content.

## Shape and naming

500 albums of 100 tracks, 5 albums per artist, 100 artists. Names are pure
functions of the indices over fixed word banks, with runtime asserts that
artist names, album titles and per-album track titles stay distinct. The first
version of the artist-name function dropped the adjective in one of its three
patterns and collapsed 100 artists to 72 directories. The verifier caught it,
the pattern now always includes the adjective-noun pair, and the asserts keep
that class of bug from shipping silently again.

Paths look like `library/Wandering Tides/Bright Motion/01 Cold Motion.wav`.
Track titles vary per album via an index offset, so rows show distinct strings.
Genre cycles 10 values per album, year spans 1972-2025 per album. Audio is a
quiet sine whose period divides 8000 Hz, tiled to length, so playback is
audible and the PCM is not silence.

## Verified this session

- Smoke run 200 tracks / 4 albums, then full run 50,000 / 500, both checked by
  `verify_fixture.py`, which shares no code with the writer.
- Counts by walking the tree: exactly 50,000 `.wav`, 500 album directories,
  100 artist directories, one `cover.jpg` per album, 100 tracks per album.
- 500 tracks byte-checked (both edge albums in full, 300 seeded-random middle
  tracks): an independent ID3v2.3 parser reads TIT2, TPE1, TPE2, TALB, TRCK,
  TPOS, TCON, TDRC back and each is cross-checked against the file's path.
  RIFF size, fmt fields, chunk order, data size and non-silent PCM asserted.
  mutagen is not installed, so the round-trip uses this hand-written parser,
  as the task permits.
- All 500 covers decoded by PIL as JPEG 300x300 with 500 distinct SHA-256
  hashes. Two covers also inspected visually (numbers 8 and 250).
- Total size 2,417,070,295 bytes (2.25 GiB): 2,411,168,174 audio plus 5,902,121
  cover bytes.
- Idempotence at full size: a second identical run kept all 50,500 files,
  wrote none, removed none. A shrink run on a scratch root removed exactly the
  surplus files and the shrunk tree passed verification.

## Numbers and their limiter

First full generation took 55.1 s wall (52.8 s in the write phase) with 8
threads; the identical rerun took 24.8 s because it reads and compares 2.25 GiB
instead of writing it. The bound is NTFS small-file create/write latency plus
the 2.25 GiB of payload, not Python compute: the sine tables and tags are
microseconds per track. These are generator timings only. They say nothing
about riff's scan throughput, which the spike measures separately.

## Still pending

End-to-end verification through riff's own scan and playback is NOT done. This
session may not run cargo, so the tag mapping is proven by the reader-source
citations above plus the independent byte round-trip, and decodability by the
symphonia chunk-handling citations plus the repo's own precedent that this
exact PCM layout scans and plays. ffmpeg is not installed, so no third-party
decode check was possible. The spike's bridge step must point the real scan
pipeline at `.scratch/nsh-p0/library` and confirm 50,000 indexed rows with
titles, artists, albums, track numbers, durations, and 500 resolved covers.