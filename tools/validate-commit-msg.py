#!/usr/bin/env python3
"""Validate riff commit messages against the Conventional Commits profile.

One rule implementation, three entry points:

    validate-commit-msg.py <file>       validate a message file (git commit-msg hook)
    validate-commit-msg.py --stdin      validate a message from standard input (CI)
    validate-commit-msg.py --self-test  run the built-in table of known cases

Exit codes: 0 accepted, 1 rejected (or a self-test mismatch), 2 usage/IO error.
"""

import re
import sys

ALLOWED_TYPES = (
    "feat", "fix", "perf", "refactor", "deps", "revert",
    "build", "ci", "chore", "docs", "test", "style",
)

MAX_HEADER_CHARS = 72
MAX_BODY_LINE_CHARS = 80

HEADER_RE = re.compile(r"^([a-zA-Z]+)(?:\(([^)]*)\))?(!)?: (.*)$")
SCOPE_RE = re.compile(r"^[a-z0-9]+(?:[._/-][a-z0-9]+)*$")
FOOTER_TOKEN_RE = re.compile(r"^[A-Za-z][A-Za-z0-9-]*: ")
BREAKING_CANONICAL_RE = re.compile(r"^BREAKING[ -]CHANGE: \S")
BREAKING_NEAR_MISS_RE = re.compile(r"^(?:breaking\s*:|breaking[-_ ]?change\s*:)", re.IGNORECASE)

# git-generated markers pass through untouched: merge, revert, autosquash, WIP
PASSTHROUGH_PREFIXES = (
    "Merge ",
    'Revert "',
    "fixup! ",
    "squash! ",
    "amend! ",
    "WIP:",
    "WIP ",
)

ALLOWED_TYPES_LINE = ", ".join(ALLOWED_TYPES)

# The self-test seam: (case name, message, expected verdict). Cases assert the
# observable behaviour only — accepted or rejected — never how a rule is
# implemented. Any mismatch names the failing case and prints expected vs actual.
SELF_TEST_CASES = [
    # one accepted example per allowed type
    ("type feat", "feat: add commit-message validator\n", True),
    ("type fix", "fix: stop the scan worker from spinning\n", True),
    ("type perf", "perf: reuse the decoded cover buffer\n", True),
    ("type refactor", "refactor: split the cover cache in two\n", True),
    ("type deps", "deps: bump egui to 0.35.1\n", True),
    ("type revert", "revert: drop the thumbnail cache sweep\n", True),
    ("type build", "build: pin the release profile to LTO\n", True),
    ("type ci", "ci: run clippy with warnings as errors\n", True),
    ("type chore", "chore: refresh the issue tracker labels\n", True),
    ("type docs", "docs: describe the threading model\n", True),
    ("type test", "test: cover the cover-cache decision half\n", True),
    ("type style", "style: align the theme token names\n", True),
    # the type set is closed
    ("type outside the set", "feature: add a thing\n", False),
    ("uppercase type is unknown", "Feat: add a thing\n", False),
    # header shape and bounds
    ("header at exactly 72 characters", "fix: " + "a" * 67, True),
    ("header at 73 characters", "fix: " + "a" * 68, False),
    ("trailing period on description", "fix: done.\n", False),
    ("empty description", "feat:\n", False),
    ("blank description", "feat:   \n", False),
    ("uppercase scope", "feat(UI): add a panel\n", False),
    ("scope with separators", "feat(ui/cover_cache): split the halves\n", True),
    ("empty scope", "feat(): add a panel\n", False),
    ("header without a type prefix", "add the validator\n", False),
    # breaking change markers
    ("breaking with exclamation mark", "feat(api)!: change the AudioOutput port\n", True),
    ("breaking with exclamation mark, no scope", "feat!: change the port\n", True),
    (
        "breaking with footer",
        "feat: rework the queue\n\nBREAKING CHANGE: PlaybackQueue::push now returns a receipt\n",
        True,
    ),
    (
        "breaking with hyphenated footer",
        "feat: rework the queue\n\nBREAKING-CHANGE: PlaybackQueue::push now returns a receipt\n",
        True,
    ),
    (
        "breaking with both marker and footer",
        "feat!: drop the legacy cache\n\nBREAKING CHANGE: library_cache.json is never read\n",
        True,
    ),
    (
        "breaking near-miss marker BREAKING:",
        "feat: rework the queue\n\nBREAKING: PlaybackQueue::push changed\n",
        False,
    ),
    (
        "breaking near-miss marker Breaking Change:",
        "feat: rework the queue\n\nBreaking Change: queue API changed\n",
        False,
    ),
    (
        "breaking near-miss marker BREAKING_CHANGE:",
        "feat: rework the queue\n\nBREAKING_CHANGE: queue API changed\n",
        False,
    ),
    (
        "breaking footer with empty description",
        "feat: rework the queue\n\nBREAKING CHANGE:\n",
        False,
    ),
    # git-generated markers pass through unvalidated
    ("merge marker", "Merge branch 'feature/cover-cache'\n", True),
    ("revert marker", 'Revert "feat: add the animation"\n', True),
    ("fixup marker", "fixup! feat: add the validator\n", True),
    ("squash marker", "squash! feat: add the validator\n", True),
    ("amend marker", "amend! feat: add the validator\n", True),
    ("WIP with colon", "WIP: scanning the library\n", True),
    ("WIP with space", "WIP scanning the library\n", True),
    # body rules
    (
        "body without blank-line separator",
        "feat: add the validator\nThis is the body without a separator.\n",
        False,
    ),
    ("body line at exactly 80 characters", "feat: bounded body\n\n" + "b" * 80, True),
    ("body line at 81 characters", "feat: bounded body\n\n" + "b" * 81, False),
    # the dependency bot's prefixes
    ("dependabot cargo prefix", "build(deps): bump foo from 1.2 to 1.3\n", True),
    ("dependabot actions prefix", "ci: bump actions/checkout from v4 to v5\n", True),
    # defensive input handling
    ("empty message", "", False),
    ("whitespace-only message", "\n  \n\t\n", False),
    (
        "comment lines and scissors line are stripped",
        "# ------------------------ >8 ------------------------\n"
        "# On branch cc/01-validator\n"
        "feat: real message\n"
        "# Please enter the commit message for your changes.\n",
        True,
    ),
    ("CRLF tolerated", "feat: crlf tolerated\r\n\r\nbody line here\r\n", True),
    ("leading blank lines ignored", "\n\n\nfeat: leading blank lines\n", True),
    ("trailing blank lines ignored", "feat: trailing blank lines\n\n\n\n", True),
]


def clean_message(raw):
    """Return the message as a list of lines with git noise removed."""
    lines = [line[:-1] if line.endswith("\r") else line for line in raw.split("\n")]
    # comment lines (including the git scissors line) are editor decoration
    lines = [line for line in lines if not line.lstrip().startswith("#")]
    while lines and not lines[0].strip():
        del lines[0]
    while lines and not lines[-1].strip():
        del lines[-1]
    return lines


def footer_paragraph(body_lines):
    """Return the body's last paragraph if it is footer-shaped, else None."""
    paragraphs = []
    current = []
    for line in body_lines:
        if line.strip():
            current.append(line)
        elif current:
            paragraphs.append(current)
            current = []
    if current:
        paragraphs.append(current)
    if not paragraphs:
        return None
    last = paragraphs[-1]
    first = last[0]
    footer_shaped = (
        FOOTER_TOKEN_RE.match(first)
        or BREAKING_CANONICAL_RE.match(first)
        or BREAKING_NEAR_MISS_RE.match(first)
    )
    return last if footer_shaped else None


def near_miss_reason(paragraph_lines):
    for line in paragraph_lines:
        if BREAKING_CANONICAL_RE.match(line):
            continue
        if BREAKING_NEAR_MISS_RE.match(line):
            return (
                "near-miss breaking-change marker %r: use 'BREAKING CHANGE: <description>' "
                "or 'BREAKING-CHANGE: <description>', or '!' after the type/scope" % line.strip()
            )
    return None


def validate(raw):
    """Return None if the message is accepted, else the rejection reason."""
    lines = clean_message(raw)
    if not lines:
        return "commit message is empty (or only comments and blank lines)"

    header = lines[0]
    if header.startswith(PASSTHROUGH_PREFIXES):
        return None

    if len(header) > MAX_HEADER_CHARS:
        return "header is %d characters (limit is %d)" % (len(header), MAX_HEADER_CHARS)
    match = HEADER_RE.match(header)
    if match is None:
        return "header must look like 'type(scope)!: description'; allowed types: " + ALLOWED_TYPES_LINE
    type_name, scope, _bang, description = match.groups()
    if type_name not in ALLOWED_TYPES:
        return "unknown type '%s'; allowed types: %s" % (type_name, ALLOWED_TYPES_LINE)
    if scope is not None and not SCOPE_RE.match(scope):
        return (
            "invalid scope '%s': scope must be lowercase alphanumerics, "
            "with '-', '_', '/', '.' as separators" % scope
        )
    if not description.strip():
        return "description must not be empty"
    if description.endswith("."):
        return "description must not end with a period"

    body = lines[1:]
    if body:
        if body[0].strip():
            return "body must be separated from the header by a blank line"
        for index, line in enumerate(body):
            if len(line) > MAX_BODY_LINE_CHARS:
                return "body line %d is %d characters (limit is %d)" % (
                    index + 1, len(line), MAX_BODY_LINE_CHARS,
                )
        footer = footer_paragraph(body)
        if footer is not None:
            return near_miss_reason(footer)
    return None


def self_test():
    failures = 0
    for name, message, expected in SELF_TEST_CASES:
        reason = validate(message)
        accepted = reason is None
        if accepted != expected:
            failures += 1
            print("FAIL %s" % name, file=sys.stderr)
            print("  expected: %s" % ("accept" if expected else "reject"), file=sys.stderr)
            print(
                "  actual:   %s" % ("accept" if accepted else "reject: %s" % reason),
                file=sys.stderr,
            )
    total = len(SELF_TEST_CASES)
    if failures:
        print("self-test failed: %d of %d cases mismatched" % (failures, total), file=sys.stderr)
        return 1
    print("self-test passed: %d cases" % total)
    return 0


def run_and_report(raw):
    reason = validate(raw)
    if reason is not None:
        print("invalid commit message: %s" % reason, file=sys.stderr)
        return 1
    return 0


def main(argv):
    if len(argv) != 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    mode = argv[1]
    if mode == "--self-test":
        return self_test()
    if mode == "--stdin":
        return run_and_report(sys.stdin.read())
    try:
        with open(mode, "r", encoding="utf-8", errors="replace") as fh:
            raw = fh.read()
    except OSError as exc:
        print("error: cannot read %s: %s" % (mode, exc), file=sys.stderr)
        return 2
    return run_and_report(raw)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
