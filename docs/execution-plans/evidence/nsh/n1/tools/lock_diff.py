"""Compare two Cargo.lock files and fail when the golden render path moves.

Adding a workspace member must leave the egui golden baselines' renderer on the
exact versions they were authored against. Three kinds of difference are kept
apart, because they mean different things:

- duplicate: the old version survives and a second one appears beside it. Two
  semver-incompatible 0.x majors coexist. Not a churn.
- replaced: an old version disappears. A churn. Fatal when the crate is in the
  golden render path; otherwise reported for a human to explain.
- removed: a package left the graph entirely. Always fatal.

    python lock_diff.py <before.toml> <after.toml>

Exit 0 when no golden-path crate was replaced and nothing was removed.
Exit 1 otherwise. Non-golden replacements are printed, not fatal.
"""

import collections
import pathlib
import sys

GOLDEN = [
    "egui", "eframe", "egui_kittest", "egui_extras", "egui-wgpu", "egui-winit",
    "wgpu", "naga", "wgpu-core", "wgpu-hal", "wgpu-types", "tiny-skia",
    "tiny-skia-path", "image", "resvg", "usvg", "fontdb", "rustybuzz",
    "ttf-parser", "ab_glyph", "owned_ttf_parser", "epaint", "emath", "ecolor",
    "winit",
]


def parse(path):
    versions = collections.defaultdict(set)
    name = None
    for line in pathlib.Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("name = "):
            name = line.split('"')[1]
        elif line.startswith("version = ") and name:
            versions[name].add(line.split('"')[1])
            name = None
    return versions


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    before, after = parse(sys.argv[1]), parse(sys.argv[2])

    added = sorted(set(after) - set(before))
    removed = sorted(set(before) - set(after))
    duplicates, replaced = [], []
    for n in sorted(set(before) & set(after)):
        if before[n] == after[n]:
            continue
        if before[n] <= after[n]:
            duplicates.append(n)
        else:
            replaced.append((n, sorted(before[n] - after[n]), sorted(after[n])))
    golden = [n for n, _, _ in replaced if n in GOLDEN]

    print(f"before={sum(len(v) for v in before.values())} after={sum(len(v) for v in after.values())}")
    print(f"added={len(added)} removed={len(removed)} duplicates={len(duplicates)} replaced={len(replaced)}")
    print("golden-relevant replacements:", golden or "NONE")
    print("duplicates (old version survives):", duplicates or "NONE")
    for n, lost, gained in replaced:
        print(f"  REPLACED {n}: lost {lost} now {gained}")
    print("removed:", removed or "NONE")

    if removed or golden:
        print("STOP: the golden render path moved or a package left the graph")
        return 1
    if replaced:
        print("REVIEW: non-render crates were replaced; explain each before merging")
    print("PASS: golden render path untouched")
    return 0


if __name__ == "__main__":
    sys.exit(main())