# 005: Native Folder Picker on macOS/Windows, Text Input on Linux

**Status**: Accepted
**Date**: 2026-07-31

## Context

Adding a library path requires a folder picker. The `rfd` crate provides native file dialogs on macOS and Windows but does not reliably provide them on Linux — the native-dialog dependency stack varies by distribution and desktop environment.

## Decision

Use `rfd`'s native folder picker on macOS and Windows. On Linux, use a plain text input field for the user to type or paste the directory path.

This decision is enforced via conditional compilation: the native-picker path is compiled only on macOS and Windows; the text-input path is compiled on Linux.

## Consequences

**Positive**:
- Best UX on macOS and Windows: familiar native dialog that the user already knows.
- Linux implementation is reliable: no dependency on a specific desktop environment or library.
- Simpler Linux binary: fewer platform-specific dependencies.
- Clear platform separation: the two code paths do not interfere with each other.

**Negative**:
- Linux users must type paths by hand — less convenient than a graphical picker.
- Two different add-library experiences across platforms; the user guide must explain both.
- Linux users may not know their music directory's absolute path.

## Related Documents

- [Features](./features.md) — Music Library Management.
- [User Guide](./user-guide.md) — "Adding a music library" section.
- [Platform Support](../technical/platform-support.md).
