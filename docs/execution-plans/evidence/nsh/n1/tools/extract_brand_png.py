"""Extract the 1024x1024 brand PNG from riff.icns for the nsh app icon.

The icns container already holds a 1024x1024 raster of the mark as its ic10
chunk. Extracting it keeps the nsh icon byte-identical to the one macOS ships,
which release-and-packaging.md requires. Rasterizing mark.svg again would
produce a different anti-aliasing result and let the two copies drift.

Rerunnable. Writes only when the bytes differ, and verifies the PNG header
before and after the write.

    python extract_brand_png.py <icns> <out.png>
"""

import hashlib
import pathlib
import struct
import sys

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
WANT_CHUNK = b"ic10"
WANT_SIZE = 1024


def chunks(blob):
    if blob[:4] != b"icns":
        raise SystemExit("not an icns container")
    total = struct.unpack(">I", blob[4:8])[0]
    if total != len(blob):
        raise SystemExit(f"icns header length {total} != file length {len(blob)}")
    at = 8
    while at < len(blob):
        kind = blob[at : at + 4]
        size = struct.unpack(">I", blob[at + 4 : at + 8])[0]
        if size < 8:
            raise SystemExit(f"chunk {kind!r} declares length {size}")
        yield kind, blob[at + 8 : at + size]
        at += size


def png_size(png):
    if png[:8] != PNG_SIGNATURE:
        raise SystemExit("chunk is not a PNG")
    if png[12:16] != b"IHDR":
        raise SystemExit("first PNG chunk is not IHDR")
    width, height = struct.unpack(">II", png[16:24])
    return width, height


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    source = pathlib.Path(sys.argv[1])
    target = pathlib.Path(sys.argv[2])

    found = {kind: data for kind, data in chunks(source.read_bytes())}
    for kind, data in sorted(found.items()):
        label = f"{png_size(data)}" if data[:8] == PNG_SIGNATURE else "not png"
        print(f"chunk {kind.decode('ascii')} {len(data):>7} bytes  {label}")

    if WANT_CHUNK not in found:
        raise SystemExit(f"no {WANT_CHUNK.decode('ascii')} chunk in {source}")
    png = found[WANT_CHUNK]
    size = png_size(png)
    if size != (WANT_SIZE, WANT_SIZE):
        raise SystemExit(f"ic10 is {size}, want {(WANT_SIZE, WANT_SIZE)}")

    digest = hashlib.sha256(png).hexdigest()
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists() and target.read_bytes() == png:
        print(f"unchanged {target}")
    else:
        target.write_bytes(png)
        print(f"wrote {target}")

    written = target.read_bytes()
    if written != png:
        raise SystemExit("readback differs from the extracted chunk")
    if png_size(written) != (WANT_SIZE, WANT_SIZE):
        raise SystemExit("readback is not 1024x1024")
    print(f"sha256 {digest}")
    print(f"verified {len(written)} bytes, {png_size(written)}")


if __name__ == "__main__":
    main()