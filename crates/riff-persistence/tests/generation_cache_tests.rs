//! The canonical [`GenerationCache`] — the ONE implementation of the Session
//! Projection staleness contract (ADR 0002), keyed on the [`StoreGeneration`]
//! counter it sits beside. These tests pin the primitive's lifecycle at its
//! public interface: `observe` → `store` → `loaded_at` → `peek` →
//! `invalidate` → re-`store` across a bumped generation.

use riff_persistence::store::{GenerationCache, StoreGeneration};

#[test]
fn cache_lifecycle_across_a_generation_bump() {
    let generation = StoreGeneration::new();
    let mut cache = GenerationCache::<(), i32>::new(generation.clone());

    // A fresh cache observes the counter and holds nothing.
    let epoch = cache.observe();
    assert_eq!(epoch, generation.current());
    assert!(!cache.loaded_at(epoch), "a fresh cache starts empty");
    assert_eq!(cache.peek(), None, "nothing is cached yet");

    // Committing a load stamps exactly the observed epoch.
    cache.store(epoch, (), 7);
    assert!(cache.loaded_at(epoch), "the entry holds at its epoch");
    assert!(cache.holds(epoch, &()));
    assert_eq!(cache.peek(), Some(&7));

    // A committed mutation bumps the generation: nothing holds at the NEW
    // epoch, while the stale-but-present value stays readable — the retry
    // fallback the whole contract exists for.
    generation.bump();
    let moved = cache.observe();
    assert_ne!(moved, epoch);
    assert!(
        !cache.loaded_at(moved),
        "nothing is loaded at the new epoch"
    );
    assert!(
        cache.loaded_at(epoch),
        "the entry remembers the epoch it was loaded at"
    );
    assert_eq!(cache.peek(), Some(&7), "stale-but-present beats blank");

    // invalidate drops the entry unconditionally.
    cache.invalidate();
    assert_eq!(cache.peek(), None);
    assert!(!cache.loaded_at(epoch));

    // The retry re-stores at the new epoch and the cache is fresh again.
    cache.store(moved, (), 9);
    assert!(cache.loaded_at(moved));
    assert_eq!(cache.peek(), Some(&9));
}

#[test]
fn slot_hands_out_a_fresh_default_when_the_epoch_or_key_moved() {
    let generation = StoreGeneration::new();
    let mut cache = GenerationCache::<String, Vec<i32>>::new(generation.clone());
    let epoch = generation.current();
    cache.store(epoch, "a".to_string(), vec![1]);

    // Same epoch, different key: not a hit, and the slot reinitializes —
    // another key never sees the first key's rows.
    assert!(!cache.holds(epoch, &"b".to_string()));
    let slot = cache.slot(epoch, &"b".to_string());
    assert!(slot.is_empty(), "the stale value is dropped, not served");
    *slot = vec![2];
    assert_eq!(cache.peek(), Some(&vec![2]));

    // A moved epoch starts an EMPTY slot too: re-stamping the old value
    // would present stale rows as fresh at the new generation.
    generation.bump();
    let moved = generation.current();
    let slot = cache.slot(moved, &"b".to_string());
    assert_eq!(
        *slot,
        Vec::<i32>::new(),
        "the old generation's rows dropped"
    );
    assert!(cache.loaded_at(moved));
}

#[test]
fn take_value_and_peek_work_regardless_of_epoch() {
    let generation = StoreGeneration::new();
    let mut cache = GenerationCache::<(), String>::new(generation.clone());
    cache.store(generation.current(), (), "good".to_string());
    generation.bump();

    // take_value steals the cached value whatever it is stamped with (the
    // fetch-then-swap merge path reuses prior-generation rows).
    assert_eq!(
        cache.take_value(),
        Some("good".to_string()),
        "stale-but-present value is handed out"
    );
    assert_eq!(cache.peek(), None, "take leaves the cache empty");
}
