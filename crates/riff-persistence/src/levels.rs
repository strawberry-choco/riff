//! The ONE freshness policy behind every cached read model level.
//!
//! A *level* is one cached answer inside a [`GenerationCache`] bundle: the
//! artist list, one folder's subtree ids, one genre's counts. Every level used
//! to spell out the same procedure by hand -- observe the generation, serve
//! the cached answer while it is current, otherwise load, commit, and answer --
//! in six different shapes across the collection capability, so a fix to how a
//! bounded read stays fresh had to land once per spelling. [`GenerationCache::level`]
//! is that procedure, written once.
//!
//! A level declares itself; it does not implement staleness:
//!
//! ```ignore
//! self.cache.level(
//!     &(),
//!     |bundle| bundle.artists.clone(),
//!     || loader().map(|rows| Arc::<[Artist]>::from(rows)),
//!     |bundle, artists| {
//!         bundle.artists = Some(Arc::clone(&artists));
//!         artists
//!     },
//! )
//! ```
//!
//! What the module owns, and therefore what a level cannot get wrong:
//!
//! * The generation is observed exactly once per call and used for both the
//!   freshness check and the commit stamp, so a commit racing the load cannot
//!   split one logical read across two generations.
//! * A failed load leaves the previous entry untouched. The error propagates
//!   and the next frame retries; a caller that serves cached rows on error
//!   reads through [`GenerationCache::peek`] deliberately, as the last-good
//!   fallback rather than as an answer.
//! * The epoch value never escapes: it is local to this call.

use crate::store::GenerationCache;

impl<K, V> GenerationCache<K, V>
where
    K: Clone + PartialEq,
    V: Default,
{
    /// Serve this cache's level for `key`.
    ///
    /// * `read` picks the level's answer out of the cached bundle; `None`
    ///   means this level has not been loaded yet at the current generation.
    /// * `load` produces the raw answer, usually one Application Store query.
    ///   It runs before anything is written, so a failure leaves the previous
    ///   bundle untouched and the next frame retries.
    /// * `commit` writes the loaded value into the bundle slot and returns the
    ///   caller's answer, which is the same type `read` yields so a cache hit
    ///   and a load answer identically.
    ///
    /// Serves the cached answer when the bundle is still stamped with the
    /// generation this call observes and `read` finds the level inside it;
    /// otherwise loads, commits through that same stamp, and answers it.
    pub fn level<A, B, E>(
        &mut self,
        key: &K,
        read: impl FnOnce(&V) -> Option<A>,
        load: impl FnOnce() -> Result<B, E>,
        commit: impl FnOnce(&mut V, B) -> A,
    ) -> Result<A, E> {
        let epoch = self.observe();
        if self.holds(epoch, key)
            && let Some(cached) = read(self.peek().expect("holds implies an entry"))
        {
            return Ok(cached);
        }

        // Load first, commit later: a failing query leaves the previous entry
        // exactly as it was, so the next frame retries instead of serving a
        // half-written bundle.
        let fresh = load()?;
        Ok(commit(self.slot(epoch, key), fresh))
    }
}
