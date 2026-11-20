//! Repo invariants the compiler cannot see. The Linux apt list is ONE fact
//! written in three places (the shared composite action and the two contributor
//! guides), and both workflows reach it through that action rather than a
//! private copy.
//!
//! The token sweeps police source, not prose, so before N1 nothing checked the
//! apt list: it could drift silently across `ci.yml`, `release.yml`,
//! `CONTRIBUTING.md` and `docs/engineering/development-setup.md`, and a comment
//! promising the copies matched was the only guard. N1 folds the list into
//! `.github/actions/linux-deps/action.yml`; these two tests make the remaining
//! copies a checked fact instead of a promise.
//!
//! The reader is line-based and honest about the shapes it judges. It joins
//! backslash continuations into one logical command, drops everything before
//! `apt-get install` (the `sudo apt-get update &&` prefix) and filters flag
//! tokens such as `-y`. That normalises both the action's one-package-per-line
//! block and the guides' several-packages-per-line block to the same token set.
//! A source that grows a shape the reader cannot normalise comes back empty or
//! unequal, so the test fails loudly rather than passing vacuously.

use std::collections::BTreeSet;
use std::path::Path;

/// Read a workspace-relative file, failing loudly with its path if it moves.
/// A guard that cannot read its subject must fail, never pass by reading nothing.
fn read_workspace_file(workspace: &Path, relative: &str) -> String {
    let path = workspace.join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the repo invariant source `{relative}` must be readable ({}): {error}",
            path.display()
        )
    })
}

/// The package names one source installs, normalised across the list's shapes.
///
/// Finds the single `apt-get install` command, joins its backslash continuations,
/// then takes the whitespace-separated tokens after the marker and drops flags
/// (anything starting with `-`, such as `-y`). The `sudo apt-get update &&`
/// prefix sits before the marker and is discarded with it.
fn apt_packages(text: &str) -> BTreeSet<String> {
    let mut lines = text.lines();
    let first = lines
        .find(|line| line.contains("apt-get install"))
        .expect("the source must carry an `apt-get install` command");

    // Join continuations: a line ending in `\` runs into the next. `find` left
    // the iterator positioned just after the command's first line.
    let mut command = first.trim_end().to_owned();
    while command.ends_with('\\') {
        command.truncate(command.len() - 1);
        let next = lines
            .next()
            .expect("a trailing backslash must be followed by a continuation line");
        command.push(' ');
        command.push_str(next.trim());
    }

    let (_prefix, packages) = command
        .split_once("apt-get install")
        .expect("the joined command must still carry `apt-get install`");
    packages
        .split_whitespace()
        .filter(|token| !token.starts_with('-'))
        .map(str::to_owned)
        .collect()
}

/// The action and the two guides must install the exact same package set. The
/// set is compared, not the order or the line layout, so each source keeps its
/// own readable formatting while the fact stays single-sourced.
#[test]
fn the_linux_apt_list_is_one_fact_in_three_places() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let sources = [
        ".github/actions/linux-deps/action.yml",
        "CONTRIBUTING.md",
        "docs/engineering/development-setup.md",
    ];

    let mut parsed = Vec::new();
    for relative in sources {
        let text = read_workspace_file(&workspace, relative);
        assert_eq!(
            text.matches("apt-get install").count(),
            1,
            "`{relative}` must carry exactly one `apt-get install` command (the ONE \
             list); a second command or a renamed one means the fact split again"
        );
        let packages = apt_packages(&text);
        assert!(
            !packages.is_empty(),
            "the parse of `{relative}` found no packages after `apt-get install`, so \
             it may be reading nothing: the continuation reader no longer matches \
             the list it judges"
        );
        parsed.push((relative, packages));
    }

    let (first_name, first) = &parsed[0];
    for (name, set) in &parsed[1..] {
        assert_eq!(
            set,
            first,
            "the Linux apt list drifted: `{name}` and `{first_name}` disagree.\n\
             only in `{name}`: {:?}\nonly in `{first_name}`: {:?}",
            set.difference(first).collect::<Vec<_>>(),
            first.difference(set).collect::<Vec<_>>(),
        );
    }
}

/// Both workflows install through the shared action and carry no list of their
/// own. A workflow that still runs `apt-get install` directly has reintroduced
/// the second copy the action exists to remove.
#[test]
fn both_workflows_use_the_shared_action() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for relative in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
        let text = read_workspace_file(&workspace, relative);
        assert!(
            text.contains("uses: ./.github/actions/linux-deps"),
            "`{relative}` must install Linux dependencies through the shared action \
             (`uses: ./.github/actions/linux-deps`), not a private copy of the list"
        );
        assert!(
            !text.contains("apt-get install"),
            "`{relative}` still carries an `apt-get install` command: the list must \
             live only in the shared action, so installing packages directly here \
             has drifted back to a second copy"
        );
    }
}
