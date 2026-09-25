//! The historical path for the artwork placeholder tile.
//!
//! The tile, its cache key, and its eviction moved into [`crate::ui::artwork`]
//! with the artwork presentation primitive (component-layer issue 13), which is
//! the single owner now. This module only re-exports them so code written
//! against `ui::cover_placeholder::` keeps resolving.

pub use crate::ui::artwork::{
    PLACEHOLDER_KEY, evict_generated, lookup_cover_texture, placeholder_cache_key,
    placeholder_image,
};
