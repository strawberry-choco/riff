#!/usr/bin/env python3
"""Rerunnable synthetic music library for the NSH P0 feasibility spike.

Generates PCM WAV tracks carrying ID3v2.3 tags in an `id3 ` RIFF chunk, plus
one `cover.jpg` per album directory. Every name and byte is a pure function of
the run parameters and the item indices, so a rerun converges to the same tree.
See fixture-notes.md for the reader-source citations behind the format choices.

Only writes under --root. Never touches the real library or the app store.
"""

import argparse
import colorsys
import io
import math
import os
import struct
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO_ROOT = next(p for p in Path(__file__).resolve().parents if (p / "crates" / "riff-backend").is_dir())
DEFAULT_ROOT = REPO_ROOT / ".scratch" / "nsh-p0" / "library"

SAMPLE_RATE = 8000
CHANNELS = 1
BITS = 16
BYTES_PER_SECOND = SAMPLE_RATE * CHANNELS * BITS // 8
ALBUMS_PER_ARTIST = 5
COVER_SIZE = 300

ARTIST_ADJ = (
    "Paper", "Amber", "Velvet", "Northern", "Glass", "Quiet", "Silver", "Hollow",
    "Crimson", "Wandering", "Static", "Golden", "Midnight", "Pale", "Iron",
    "Silent", "Electric", "Fading", "Wild", "Slow",
)
ARTIST_NOUN = (
    "Lanterns", "Oaks", "Tides", "Signals", "Machines", "Gardens", "Foxes",
    "Harbors", "Fields", "Engines", "Sparrows", "Circuits", "Rivers", "Antennas",
    "Orchards", "Compasses", "Wolves", "Satellites", "Meadows", "Radios",
)
ALBUM_ADJ = (
    "Slow", "Analog", "Harbor", "Night", "Static", "Wide", "Low", "Amber",
    "Drift", "Zenith", "Neon", "Quiet", "Broken", "Endless", "Soft", "Distant",
    "Warm", "Cold", "Hidden", "Open", "Last", "First", "Deep", "Still", "Bright",
)
ALBUM_NOUN = (
    "Current", "Hearts", "Lights", "Bloom", "Fields", "Open", "Tide", "Glow",
    "Decay", "Motion", "Seasons", "Rooms", "Wires", "Shores", "Hours", "Maps",
    "Voices", "Windows", "Roads", "Stars", "Letters", "Echoes", "Colors", "Dreams",
)
TRACK_ADJ = (
    "Driftwood", "Wide", "Low", "Amber", "Neon", "Quiet", "Broken", "Endless",
    "Soft", "Distant", "Warm", "Cold", "Hidden", "Open", "Last", "First",
    "Deep", "Still", "Bright", "Fading",
)
TRACK_NOUN = (
    "Open", "Tide", "Glow", "Decay", "Motion", "Season", "Room", "Wire",
    "Shore", "Hour", "Map", "Voice", "Window", "Road", "Star", "Letter",
    "Echo", "Color", "Dream", "Signal",
)
GENRES = (
    "Ambient", "Indie Rock", "Jazz", "Electronic", "Folk",
    "Classical", "Hip-Hop", "Shoegaze", "Post-Rock", "Dream Pop",
)
ALBUM_TONES = (200, 250, 320, 400, 500)


def artist_name(artist_index):
    adj = ARTIST_ADJ[artist_index % len(ARTIST_ADJ)]
    noun = ARTIST_NOUN[(artist_index // len(ARTIST_ADJ)) % len(ARTIST_NOUN)]
    partner = ARTIST_NOUN[(artist_index // len(ARTIST_ADJ) + 7) % len(ARTIST_NOUN)]
    pattern = artist_index % 3
    if pattern == 0:
        return f"The {adj} {noun}"
    if pattern == 1:
        return f"{adj} {noun}"
    return f"{adj} {noun} & {partner}"


def album_title(album_index):
    adj = ALBUM_ADJ[album_index % len(ALBUM_ADJ)]
    noun = ALBUM_NOUN[(album_index // len(ALBUM_ADJ)) % len(ALBUM_NOUN)]
    return f"{adj} {noun}"


def track_title(album_index, track_index):
    slot = (track_index + album_index * 13) % (len(TRACK_ADJ) * len(TRACK_NOUN))
    return f"{TRACK_ADJ[slot % len(TRACK_ADJ)]} {TRACK_NOUN[slot // len(TRACK_ADJ)]}"


def album_genre(album_index):
    return GENRES[album_index % len(GENRES)]


def album_year(album_index):
    return 1972 + (album_index * 7) % 54


def syncsafe(n):
    return bytes(((n >> 21) & 0x7F, (n >> 14) & 0x7F, (n >> 7) & 0x7F, n & 0x7F))


def id3v2_tag(frames):
    """ID3v2.3 tag, ISO-8859-1 text frames, no unsynchronisation, no padding."""
    body = bytearray()
    for frame_id, text in frames:
        payload = b"\x00" + text.encode("latin-1")
        body += frame_id.encode("ascii")
        body += struct.pack(">I", len(payload))
        body += b"\x00\x00"
        body += payload
    return b"ID3\x03\x00\x00" + syncsafe(len(body)) + body


def riff_chunk(fourcc, payload):
    chunk = fourcc + struct.pack("<I", len(payload)) + payload
    if len(payload) % 2:
        chunk += b"\x00"
    return chunk


def wav_bytes(tag, pcm):
    fmt = struct.pack(
        "<HHIIHH", 1, CHANNELS, SAMPLE_RATE, BYTES_PER_SECOND,
        CHANNELS * BITS // 8, BITS,
    )
    body = b"WAVE" + riff_chunk(b"fmt ", fmt) + riff_chunk(b"id3 ", tag) + riff_chunk(b"data", pcm)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def album_pcm(album_index, seconds):
    """A quiet sine whose period divides the sample rate, tiled to length."""
    tone = ALBUM_TONES[album_index % len(ALBUM_TONES)]
    period = SAMPLE_RATE // tone
    one = struct.pack(
        f"<{period}h",
        *(int(2500 * math.sin(2 * math.pi * i / period)) for i in range(period)),
    )
    samples = seconds * SAMPLE_RATE
    return (one * (samples // period + 1))[: samples * 2]


def album_cover(album_index):
    from PIL import Image, ImageDraw, ImageFont

    hue = (album_index * 137.508) % 360
    top = colorsys.hsv_to_rgb(hue / 360, 0.55, 0.95)
    bottom = colorsys.hsv_to_rgb(hue / 360, 0.75, 0.45)
    img = Image.new("RGB", (COVER_SIZE, COVER_SIZE))
    draw = ImageDraw.Draw(img)
    for y in range(COVER_SIZE):
        mix = y / (COVER_SIZE - 1)
        row = tuple(int(255 * (a + (b - a) * mix)) for a, b in zip(top, bottom))
        draw.line([(0, y), (COVER_SIZE, y)], fill=row)
    label = str(album_index + 1)
    font = ImageFont.load_default(size=140)
    left, top_y, right, bottom_y = draw.textbbox((0, 0), label, font=font)
    x = (COVER_SIZE - (right - left)) / 2 - left
    y = (COVER_SIZE - (bottom_y - top_y)) / 2 - top_y
    draw.text((x + 3, y + 3), label, font=font, fill=(20, 20, 20))
    draw.text((x, y), label, font=font, fill=(250, 250, 250))
    return img


def write_converged(path, payload):
    """Keep an identical existing file, else replace atomically. Returns action."""
    if path.exists():
        if path.read_bytes() == payload:
            return "kept"
        action = "rewritten"
    else:
        action = "written"
    tmp = path.with_name(path.name + f".tmp{os.getpid()}")
    tmp.write_bytes(payload)
    os.replace(tmp, path)
    return action


def remove_stale(root, expected):
    """Delete fixture-pattern files the plan no longer names, prune empty dirs."""
    removed = 0
    for dirpath, dirnames, filenames in os.walk(root, topdown=False):
        for name in filenames:
            path = Path(dirpath) / name
            if path in expected:
                continue
            if name.lower().endswith(".wav") or name.lower() == "cover.jpg" \
                    or ".wav.tmp" in name.lower() or ".jpg.tmp" in name.lower():
                path.unlink()
                removed += 1
        directory = Path(dirpath)
        if directory != root and not any(directory.iterdir()):
            directory.rmdir()
    return removed


def generate(root, tracks, albums, seconds, workers):
    per_album = tracks // albums
    artists = (albums + ALBUMS_PER_ARTIST - 1) // ALBUMS_PER_ARTIST
    expected = set()
    seen_artists = set()
    seen_albums = set()
    counts = {"written": 0, "kept": 0, "rewritten": 0}
    total_bytes = 0
    cover_bytes = 0

    start = time.perf_counter()
    audio_start = start
    with ThreadPoolExecutor(max_workers=workers) as pool:
        for g in range(albums):
            artist = artist_name(g // ALBUMS_PER_ARTIST)
            title = album_title(g)
            seen_artists.add(artist)
            assert title not in seen_albums, f"album title collision at {g}: {title}"
            seen_albums.add(title)
            album_dir = root / artist / title
            album_dir.mkdir(parents=True, exist_ok=True)

            cover_path = album_dir / "cover.jpg"
            buf = io.BytesIO()
            album_cover(g).save(buf, format="JPEG", quality=90)
            cover = buf.getvalue()
            expected.add(cover_path)
            counts[write_converged(cover_path, cover)] += 1
            cover_bytes += len(cover)

            pcm = album_pcm(g, seconds)
            jobs = []
            titles = set()
            for t in range(per_album):
                number = t + 1
                name = track_title(g, t)
                assert name not in titles, f"track title collision in album {g}: {name}"
                titles.add(name)
                tag = id3v2_tag((
                    ("TIT2", name),
                    ("TPE1", artist),
                    ("TPE2", artist),
                    ("TALB", title),
                    ("TRCK", str(number)),
                    ("TPOS", "1"),
                    ("TCON", album_genre(g)),
                    ("TDRC", str(album_year(g))),
                ))
                path = album_dir / f"{number:02d} {name}.wav"
                expected.add(path)
                jobs.append((path, wav_bytes(tag, pcm)))
            for action, size in pool.map(lambda job: (write_converged(*job), len(job[1])), jobs):
                counts[action] += 1
                total_bytes += size
    audio_elapsed = time.perf_counter() - audio_start

    assert len(seen_artists) == artists, \
        f"artist name collisions: {len(seen_artists)} distinct for {artists} artists"
    removed = remove_stale(root, expected)
    elapsed = time.perf_counter() - start

    print(f"root:            {root}")
    print(f"tracks:          {tracks} ({per_album} per album)")
    print(f"albums:          {albums}")
    print(f"artists:         {artists}")
    print(f"covers:          {albums} (cover.jpg, {COVER_SIZE}x{COVER_SIZE} JPEG)")
    print(f"file actions:    {counts} (tracks and covers)")
    print(f"stale removed:   {removed}")
    print(f"audio bytes:     {total_bytes:,} ({total_bytes / 2**30:.2f} GiB)")
    print(f"cover bytes:     {cover_bytes:,}")
    print(f"total bytes:     {total_bytes + cover_bytes:,}")
    print(f"audio phase:     {audio_elapsed:.1f} s")
    print(f"elapsed:         {elapsed:.1f} s")
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--tracks", type=int, default=50_000)
    parser.add_argument("--albums", type=int, default=500)
    parser.add_argument("--seconds", type=int, default=3,
                        help="audio duration per track (8 kHz mono 16-bit PCM)")
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()

    if args.tracks <= 0 or args.albums <= 0 or args.seconds <= 0:
        parser.error("--tracks, --albums and --seconds must be positive")
    if args.tracks % args.albums:
        parser.error("--tracks must be divisible by --albums")
    if args.albums > len(ALBUM_ADJ) * len(ALBUM_NOUN):
        parser.error(f"--albums must be at most {len(ALBUM_ADJ) * len(ALBUM_NOUN)}")
    if args.tracks // args.albums > len(TRACK_ADJ) * len(TRACK_NOUN):
        parser.error("tracks per album exceeds the distinct-title bank")

    root = args.root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    generate(root, args.tracks, args.albums, args.seconds, args.workers)


if __name__ == "__main__":
    sys.exit(main())