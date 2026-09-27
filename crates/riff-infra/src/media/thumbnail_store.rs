//! [`ThumbnailCache`] over the filesystem, below `<data_local_dir>/covers/`.
//!
//! Every entry's name comes from [`thumbnail_path::cache_path`], so a lookup is
//! one `stat` of the cover Source and one `open` of the derived name — there is
//! no index, and nothing to keep in sync with one. See the trait's contract in
//! `riff-persistence` for why there is neither a content hash nor an eviction
//! policy.

use crate::media::thumbnail_path::{SourceFingerprint, cache_path};
use riff_persistence::errors::StoreError;
use riff_persistence::thumbnail::{EncodedThumbnail, ThumbnailBox, ThumbnailCache};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// The suffix of a half-written entry. [`FileThumbnailCache::store`] writes
/// there first and renames onto the final name, so a finished entry is always
/// complete — and so anything found with this suffix is debris from a crash.
const PARTIAL_SUFFIX: &str = "tmp";

/// The file-backed Thumbnail cache.
pub struct FileThumbnailCache {
    root: PathBuf,
}

impl FileThumbnailCache {
    /// Rooted at `covers/` beside the Application Store, matching where
    /// `riff.sqlite3` lives (`crate::store::default_store_path`).
    ///
    /// Creating the handle touches nothing: the directory appears with the first
    /// entry and goes away with [`ThumbnailCache::clear`].
    pub fn new() -> Result<Self, StoreError> {
        directories::ProjectDirs::from("", "", "riff")
            .map(|dirs| Self::with_root(dirs.data_local_dir().join("covers")))
            .ok_or_else(|| {
                StoreError::InvalidOperation(
                    "no data-local directory is available on this platform".to_string(),
                )
            })
    }

    /// The same cache below an explicit root. Every test of this adapter uses
    /// this constructor, because [`Self::new`] resolves the user's real data
    /// directory.
    #[must_use]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl ThumbnailCache for FileThumbnailCache {
    /// One `stat` of the Source to rebuild the name, one `open` of that name.
    ///
    /// Every failure here is a miss, not an error: an absent Source, an absent
    /// entry, an entry whose container cannot be read, and a Source too old for
    /// this name all answer `None`, and the caller resolves as it would have
    /// without a cache.
    fn load(&self, source: &Path, box_: ThumbnailBox) -> Option<EncodedThumbnail> {
        let fingerprint = SourceFingerprint::of(source)?;
        let entry = cache_path(&self.root, source, &fingerprint, box_);
        let bytes = std::fs::read(&entry).ok()?;
        // The stored image's own pixel size, read out of the bytes already in
        // hand — no second `open`, and no assumption that it equals `box_`: a
        // Source smaller than the requested box is stored at its own size under
        // the requested name.
        let format = image::guess_format(&bytes).ok()?;
        let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format)
            .into_dimensions()
            .ok()?;
        Some(EncodedThumbnail {
            bytes,
            width,
            height,
        })
    }

    /// Write the entry atomically, and sweep the debris a previous crash left.
    ///
    /// The caller treats an `Err` as "not cached" and carries on: a full or
    /// read-only disk costs a re-decode, never a failed resolution.
    fn store(
        &self,
        source: &Path,
        box_: ThumbnailBox,
        thumbnail: &EncodedThumbnail,
    ) -> Result<(), StoreError> {
        let fingerprint = SourceFingerprint::of(source).ok_or_else(|| {
            StoreError::InvalidOperation(format!(
                "cover source {} cannot be stated",
                source.display()
            ))
        })?;
        let entry = cache_path(&self.root, source, &fingerprint, box_);
        let dir = entry
            .parent()
            .ok_or_else(|| StoreError::InvalidOperation("entry has no fanout dir".to_string()))?;
        std::fs::create_dir_all(dir).map_err(|e| {
            tracing::warn!("Could not create the Thumbnail directory {dir:?}: {e}");
            StoreError::InvalidOperation(format!("could not create {}: {e}", dir.display()))
        })?;
        sweep_partial_writes(dir);

        // Same directory, then rename: atomic on POSIX, and `std::fs::rename`
        // maps to `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` on Windows. So a
        // reader either sees the whole entry or nothing, and two workers writing
        // this one entry at once write identical bytes to the same name — which
        // is why there is no mutex here.
        let partial = entry.with_extension(PARTIAL_SUFFIX);
        std::fs::write(&partial, &thumbnail.bytes).map_err(|e| {
            tracing::warn!("Could not write the partial Thumbnail {partial:?}: {e}");
            StoreError::InvalidOperation(format!("could not write {}: {e}", partial.display()))
        })?;
        std::fs::rename(&partial, &entry).map_err(|e| {
            tracing::warn!("Could not commit the Thumbnail {partial:?} -> {entry:?}: {e}");
            let _ = std::fs::remove_file(&partial);
            StoreError::InvalidOperation(format!("could not commit {}: {e}", entry.display()))
        })
    }

    /// Remove every cached Thumbnail. There is no eviction, so this is the only
    /// reclaim — which is why it is a user action rather than a policy.
    fn clear(&self) -> Result<(), StoreError> {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            // Already empty is already clear.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => {
                tracing::warn!(
                    "Could not clear the Thumbnail cache at {:?}: {e}",
                    self.root
                );
                Err(StoreError::InvalidOperation(format!(
                    "could not clear {}: {e}",
                    self.root.display()
                )))
            }
        }
    }
}

/// Delete entries a crashed `store` left behind.
///
/// Because nothing ever evicts, debris would be permanent: this sweep is the one
/// concession to housekeeping the design makes. It runs on a directory the
/// caller just created or opened for writing, so it is bounded by one fanout
/// bucket, and a failure to delete is not worth reporting — the entry being
/// written is unaffected.
fn sweep_partial_writes(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .ends_with(&format!(".{PARTIAL_SUFFIX}"))
        }) {
            let _ = std::fs::remove_file(&path);
        }
    }
}
