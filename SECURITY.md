# Security Policy

## Reporting a vulnerability

**Use [GitHub's private vulnerability reporting](https://github.com/strawberry-choco/riff/security/advisories/new)** — the Security tab on the repository, then **Report a vulnerability**. It opens a private advisory that only the maintainer can read, which leaves room for a fix before anything is public.

**Do not open a public issue for a suspected vulnerability.** A public issue discloses the details to everyone before a fix exists, and issues are indexed and cached by third parties. Send it privately, and let the maintainer decide when it becomes public.

This route exists because riff parses untrusted input by design: it decodes the media files you point it at, reads and rewrites their tags, and decodes their embedded cover images. A bug in a decoder, in the tag writer, or in image parsing is a plausible attack surface, and riff also writes back into your files, so a malformed-file bug can do damage as well as disclose.

If the **Report a vulnerability** button or the link above is missing, private reporting is switched off for this repository, and the fallback is the maintainer's GitHub profile: use **Report abuse** on [github.com/strawberry-choco](https://github.com/strawberry-choco), mark it clearly as a security report, and say that the private channel appeared to be unavailable. That form is not designed for vulnerability reports, but it reaches only the maintainer, which is what matters here. Please do not fall back to a public issue.

## What to include

- **Affected version or commit.** riff is pre-1.0 and no packaged build has been published, so this is the commit you built: `git rev-parse --short HEAD`. There are no release tags yet, so `git describe` will not name a version.
- **Platform.** Operating system and version, and the audio backend in use if it is relevant (ALSA, WASAPI, CoreAudio).
- **Reproduction steps.** What you did, what file or library you pointed riff at, and how reliably it reproduces. A small file that triggers it is the single most useful thing you can bring.
- **Impact.** What you observed: a crash, a hang, memory corruption, an out-of-bounds write into one of your media files, or a hang on a hostile network or mounted share. If you do not know, say so rather than guessing.
- **Any log output or panic report** you captured along the way.

Please do not attach a real media file you care about. A minimal, synthetic file that triggers the same behavior is worth more and costs less.

## Supported versions

| Version | Supported |
|---|---|
| `0.2.x` — the current in-development series (`0.2.0` is the current version) | Yes |
| `master` | Yes |
| Any earlier series | No |
| Any fork | No |

riff has not reached 1.0 and has no published release cadence. Fixes land on `master`, and there is no backport policy: an older series will not receive a patch. If you are on a fork, the fix will land here and you will need to merge it yourself.

## What to expect

An acknowledgement that the report arrived, and then investigation. There is no guaranteed response time and no service-level agreement — this is a small project maintained in the open, not a vendor with a security response team. If a report turns out to be a bug rather than a vulnerability, it will be handled as a bug, and the advisory will be closed with an explanation.

## Security-relevant architecture

riff performs no network activity of any kind (see [docs/product/decisions/001-offline-first.md](docs/product/decisions/001-offline-first.md)), so there is no remote attack surface. The untrusted input is whatever is on your disk, and the third-party code that touches it all lives in `riff-infra`:

- **`symphonia`** decodes audio containers and codecs. This is the largest attack surface by volume: a decoder is a large piece of code running on bytes chosen by someone else.
- **`lofty`** reads *and writes* tags. The write path is the one that can modify your files, so a parsing bug here is the most likely to lose data.
- **`image`** decodes embedded cover art (JPEG and PNG).
- **`walkdir`** and **`notify`** enumerate and watch the filesystem. Paths and directory names are attacker-influenced input if the music lives on a share someone else writes to.
- **`rusqlite`** (bundled SQLite) stores the index. It is fed values read out of the media files above, so anything the readers accept becomes database input.

Beyond memory-safety, treat any library folder riff watches as a trust boundary: the threat model that matters is a hostile file in your collection, or a hostile filesystem share, not a hostile network.
