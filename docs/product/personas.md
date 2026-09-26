# Target Users

riff is not trying to serve everyone who listens to audio. It is built for people whose music is a collection of files they own and organize themselves, and who want a player that respects that. The three personas below describe the users riff is designed around: what they are trying to do, and what frustrates them about the players they have tried. The feature set that meets them is catalogued in [./features.md](./features.md); the product framing behind these choices is in [./overview.md](./overview.md).

## The Music Listener / Collector

Maya has been accumulating music for twenty years. Her collection lives in several places at once: a main library on her internal drive, a freshly ripped backlog on an external SSD, and a large archive on a NAS share mounted over the network. Formats are mixed — FLAC for serious listening, MP3 from older purchases, a growing pile of Opus files from recent downloads. She adds new albums every week, usually by copying folders in from another machine, and she occasionally deletes or reorganizes material she no longer wants.

**Goals**

- See her whole collection in one place, regardless of which physical drive a file lives on.
- Add and remove library locations visually, without editing config files or typing paths by hand.
- Have new music appear in the player without a manual "refresh" ritual every time she copies files in.
- Browse by artist and album when she is exploring, and by folder when she knows exactly where something is.
- Keep listening with the window out of the way while she works.

**Frustrations**

- Players that assume one music folder and fall apart when the collection spans drives.
- Having to trigger a full rescan by hand after every download, or living with stale entries for files she deleted months ago.
- Cloud-first players that treat local files as a second-class afterthought, or demand an account and an upload step for music she already owns.
- Libraries that take minutes to appear after launch because the player re-walks every disk on startup.

## The Minimalist

Jonas wants a music player the way he wants a text editor: small, fast, and quiet. He does not have a cloud music subscription and does not want one; his music is a modest, well-ordered folder that he backs up himself. He is deeply uninterested in creating accounts, accepting telemetry, or watching a player download metadata he never asked for. Every previous player he tried either nagged him about a premium tier or came bundled with services he had to disable one by one.

**Goals**

- Play local files with standard controls and no ceremony.
- A single small binary he can install, update, and remove cleanly.
- Fast startup and low resource use on modest hardware.
- No network activity, no accounts, no "smart" features that phone home.

**Frustrations**

- Players that are really storefronts or service clients with a playback feature attached.
- Bloated dependency stacks and Electron-scale memory use for what should be a simple task.
- Being unable to tell what a program is doing on the network, or being sure it is doing nothing.

## The Archivist

Priya's collection is a preservation project. Everything is FLAC or lossless, ripped carefully and tagged by hand: album artist on compilations, correct track numbers, genre and year filled in, front cover embedded in every file. She has strong opinions about metadata fidelity and has been burned by players that silently ignored her tags, misgrouped compilations under the wrong artist, or displayed the wrong artwork because a stray `folder.jpg` outranked the embedded image she curated.

**Goals**

- Lossless formats played correctly, with no unnecessary processing in the signal path.
- Tags honored exactly as written — especially album artist for compilations and multi-artist albums.
- Embedded cover art treated as authoritative, with sensible fallbacks for the directories where she stores artwork as files.
- An index that reflects the collection faithfully and can be rebuilt deterministically from the files themselves.

**Frustrations**

- Players that group albums by track artist and scatter compilations across the library.
- Artwork resolution that picks a random image from the folder over the embedded picture she embedded on purpose.
- Opaque library databases that drift out of sync with the files and cannot be regenerated from scratch.
- Players that choke on large FLAC files or read them entirely into memory.

**How riff helps**

Two of Priya's concerns are answered by the implementation itself rather than by a feature toggle. Decoding is streaming and packet-based, so even very large lossless files play with bounded memory instead of being read whole. And cover art resolution is deterministic: embedded metadata always wins, then a case-insensitive filesystem fallback in a fixed, documented name priority order, so her curated artwork is what gets displayed. Everything else she cares about — the tag set that is read and written, album grouping by album artist, and the index that reflects the collection — is catalogued in [./features.md](./features.md).

## Common ground

Different as they are, all three personas share the traits riff optimizes for: they own their music as files, they prefer local control to cloud convenience, and they want a player that is fast, honest about what it does, and quiet about everything else. If a prospective user's first question is "which streaming services does it support?", riff is not for them — and [./overview.md](./overview.md) says so plainly. If their first question is "will it play my FLAC files exactly as I tagged them?", it is.
