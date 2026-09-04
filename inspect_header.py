import os

out = r"C:\projects\riff\header_dump.txt"
lines = []
for p in [r"C:\Windows\Minidump\090426-18078-01.dmp",
          r"C:\Windows\Minidump\082926-17562-01.dmp"]:
    try:
        sz = os.path.getsize(p)
        with open(p, 'rb') as f:
            head = f.read(64)
        lines.append(f"=== {p} ===")
        lines.append(f"  size: {sz} bytes")
        lines.append(f"  first 16 hex: {head[:16].hex()}")
        lines.append(f"  first 16 ascii: {head[:16]!r}")
    except Exception as e:
        lines.append(f"=== {p} === ERROR: {e}")

with open(out, "w", encoding="utf-8") as f:
    f.write("\n".join(lines))
