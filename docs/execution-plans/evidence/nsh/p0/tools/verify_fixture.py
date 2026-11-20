#!/usr/bin/env python3
"""Independent check of a make_fixture.py tree. Shares no code with the writer.

Parses ID3v2.3 tags and RIFF chunks straight from the bytes, cross-checks every
tag field against the file's path, decodes every cover with PIL, and asserts
exact track/album/artist counts. Exits non-zero on the first failed assertion.
"""

import argparse
import hashlib
import random
import struct
import sys
from pathlib import Path

REPO_ROOT = next(p for p in Path(__file__).resolve().parents if (p / "crates" / "riff-backend").is_dir())
DEFAULT_ROOT = REPO_ROOT / ".scratch" / "nsh-p0" / "library"
ALBUMS_PER_ARTIST = 5
COVER_SIZE = 300


def read_syncsafe(b):
    assert len(b) == 4 and all(x < 0x80 for x in b), "syncsafe byte has high bit set"
    return (b[0] << 21) | (b[1] << 14) | (b[2] << 7) | b[3]


def parse_id3v2(blob):
    """Minimal independent ID3v2.3 reader: text frames only, no unsync."""
    assert blob[:3] == b"ID3", "missing ID3 magic"
    assert blob[3] == 3, f"expected ID3v2.3, got v2.{blob[3]}"
    assert blob[5] & 0x80 == 0, "unsynchronisation not supported by this checker"
    assert blob[5] & 0x40 == 0, "extended header not supported by this checker"
    size = read_syncsafe(blob[6:10])
    assert 10 + size <= len(blob), "tag size exceeds chunk payload"
    frames = {}
    pos = 10
    while pos + 10 <= 10 + size:
        frame_id = blob[pos:pos + 4]
        if frame_id == b"\x00\x00\x00\x00":
            break
        frame_size = int.from_bytes(blob[pos + 4:pos + 8], "big")
        payload = blob[pos + 10:pos + 10 + frame_size]
        assert len(payload) == frame_size, "truncated frame"
        if frame_id.startswith(b"T"):
            encoding = payload[0]
            raw = payload[1:]
            if encoding == 0:
                text = raw.decode("latin-1")
            elif encoding == 1:
                text = raw.decode("utf-16")
            elif encoding == 2:
                text = raw.decode("utf-16-be")
            elif encoding == 3:
                text = raw.decode("utf-8")
            else:
                raise AssertionError(f"unknown text encoding {encoding}")
            frames[frame_id.decode("ascii")] = text.split("\x00")[0]
        pos += 10 + frame_size
    return frames


def parse_wav(blob):
    """Walk RIFF chunks; return (fmt fields, id3 payload, data size, order)."""
    assert blob[:4] == b"RIFF" and blob[8:12] == b"WAVE", "not a RIFF/WAVE file"
    assert struct.unpack_from("<I", blob, 4)[0] == len(blob) - 8, "RIFF size mismatch"
    fmt = None
    tag = None
    data_size = None
    order = []
    pos = 12
    while pos + 8 <= len(blob):
        fourcc = blob[pos:pos + 4]
        size = struct.unpack_from("<I", blob, pos + 4)[0]
        payload = blob[pos + 8:pos + 8 + size]
        assert len(payload) == size, f"truncated {fourcc!r} chunk"
        order.append(fourcc)
        if fourcc == b"fmt ":
            fmt = struct.unpack("<HHIIHH", payload[:16])
        elif fourcc in (b"id3 ", b"ID3 "):
            tag = payload
        elif fourcc == b"data":
            data_size = size
        pos += 8 + size + (size % 2)
    assert fmt is not None, "no fmt chunk"
    assert tag is not None, "no id3 chunk"
    assert data_size is not None, "no data chunk"
    assert order.index(b"fmt ") < order.index(b"data"), "fmt must precede data"
    return fmt, tag, data_size


def check_track(path, artist, album, seconds):
    blob = path.read_bytes()
    fmt, tag_blob, data_size = parse_wav(blob)
    assert fmt == (1, 1, 8000, 16000, 2, 16), f"{path}: unexpected fmt {fmt}"
    assert data_size == seconds * 16000, f"{path}: data size {data_size}"

    stem = path.stem
    number_text, _, title_from_name = stem.partition(" ")
    assert number_text.isdigit(), f"{path}: filename lacks track number"
    frames = parse_id3v2(tag_blob)
    for key in ("TIT2", "TPE1", "TPE2", "TALB", "TRCK", "TPOS", "TCON", "TDRC"):
        assert key in frames and frames[key], f"{path}: missing/empty {key}"
    assert frames["TIT2"] == title_from_name, f"{path}: title vs filename"
    assert frames["TPE1"] == artist, f"{path}: artist vs directory"
    assert frames["TPE2"] == artist, f"{path}: album artist vs directory"
    assert frames["TALB"] == album, f"{path}: album vs directory"
    assert frames["TRCK"] == str(int(number_text)), f"{path}: track number vs filename"
    assert "/" not in frames["TRCK"], f"{path}: TRCK must parse as a bare u32"
    assert frames["TPOS"] == "1", f"{path}: disc number"
    year = frames["TDRC"]
    assert len(year) == 4 and year.isdigit() and 1972 <= int(year) <= 2025, \
        f"{path}: year {year!r}"

    samples = struct.unpack_from(f"<{min(4000, data_size // 2)}h", blob,
                                  len(blob) - data_size)
    assert max(abs(s) for s in samples) > 1000, f"{path}: audio is silent"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--tracks", type=int, default=50_000)
    parser.add_argument("--albums", type=int, default=500)
    parser.add_argument("--seconds", type=int, default=3)
    parser.add_argument("--sample", type=int, default=300,
                        help="random tracks to byte-check beyond the first/last album")
    args = parser.parse_args()
    root = args.root.resolve()
    per_album = args.tracks // args.albums

    album_dirs = []
    artist_dirs = set()
    wav_count = 0
    total_bytes = 0
    for artist_dir in sorted(p for p in root.iterdir() if p.is_dir()):
        for album_dir in sorted(p for p in artist_dir.iterdir() if p.is_dir()):
            wavs = sorted(album_dir.glob("*.wav"))
            covers = list(album_dir.glob("cover.jpg"))
            assert len(wavs) == per_album, \
                f"{album_dir}: {len(wavs)} tracks, expected {per_album}"
            assert len(covers) == 1, f"{album_dir}: {len(covers)} cover.jpg files"
            album_dirs.append((artist_dir.name, album_dir, wavs))
            artist_dirs.add(artist_dir.name)
            wav_count += len(wavs)
            total_bytes += sum(p.stat().st_size for p in wavs)
            total_bytes += covers[0].stat().st_size

    assert wav_count == args.tracks, f"track count {wav_count} != {args.tracks}"
    assert len(album_dirs) == args.albums, f"album count {len(album_dirs)} != {args.albums}"
    expected_artists = (args.albums + ALBUMS_PER_ARTIST - 1) // ALBUMS_PER_ARTIST
    assert len(artist_dirs) == expected_artists, \
        f"artist count {len(artist_dirs)} != {expected_artists}"

    from PIL import Image

    hashes = set()
    for artist, album_dir, _ in album_dirs:
        cover = album_dir / "cover.jpg"
        with Image.open(cover) as img:
            img.load()
            assert img.format == "JPEG", f"{cover}: format {img.format}"
            assert img.size == (COVER_SIZE, COVER_SIZE), f"{cover}: size {img.size}"
        hashes.add(hashlib.sha256(cover.read_bytes()).hexdigest())
    assert len(hashes) == args.albums, "covers are not all distinct"

    checked = []
    for artist, album_dir, wavs in (album_dirs[0], album_dirs[-1]):
        for wav in wavs:
            check_track(wav, artist, album_dir.name, args.seconds)
            checked.append(wav)
    middle = [w for _, _, wavs in album_dirs[1:-1] for w in wavs]
    rng = random.Random(20261120)
    extra = min(args.sample, len(middle))
    for wav in rng.sample(middle, extra):
        check_track(wav, wav.parent.parent.name, wav.parent.name, args.seconds)
        checked.append(wav)

    print(f"root:            {root}")
    print(f"tracks:          {wav_count} (exact)")
    print(f"albums:          {len(album_dirs)} (exact)")
    print(f"artists:         {len(artist_dirs)} (exact)")
    print(f"covers:          {len(hashes)} distinct JPEG {COVER_SIZE}x{COVER_SIZE}")
    print(f"tracks byte-checked: {len(checked)} (first+last album fully, {extra} random)")
    print(f"total bytes:     {total_bytes:,} ({total_bytes / 2**30:.2f} GiB)")
    print("PASS: tags round-trip through an independent ID3v2.3 parser, "
          "WAV structure and counts verified")


if __name__ == "__main__":
    sys.exit(main())