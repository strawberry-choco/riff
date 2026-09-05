// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    // --- Playlists (Task 4.2) ---------------------------------------------------

    #[test]
    fn test_playlist_id_new_slugs_name_and_differs_per_name() {
        let a = PlaylistId::new("My Mix!");
        let b = PlaylistId::new("Other Mix");
        assert!(a.0.starts_with("my-mix"));
        assert!(b.0.starts_with("other-mix"));
        assert_ne!(a, b);
        // A name with no alphanumerics falls back to a stable slug.
        assert!(PlaylistId::new("!!!").0.starts_with("playlist"));
    }

    #[test]
    fn test_app_state_new() {
        let playback = PlaybackSession::default();
        let library = LibrarySession::default();
        assert_eq!(playback.playback_state, PlaybackState::Stopped);
        assert!(crate::test_utils::float_close(playback.current_volume, 1.0));
        assert!(library.library_paths.is_empty());
        assert!(library.library_statuses.is_empty());
        assert!(library.watch_states.is_empty());
    }

    #[test]
    fn test_playback_queue_operations() {
        let mut queue = PlaybackQueue::default();
        let track1 = TrackId("track1.mp3".to_string());
        let track2 = TrackId("track2.mp3".to_string());

        // Test initial state
        assert!(queue.current_track().is_none());
        assert!(queue.advance().is_none());
        assert!(queue.previous().is_none());

        // Test adding tracks: the first append makes its track current.
        queue.append(track1.clone());
        queue.append(track2.clone());

        assert_eq!(queue.tracks.len(), 2);
        assert_eq!(queue.current_index, Some(0));
        assert_eq!(queue.current_track(), Some(&track1));

        // Test next track
        assert_eq!(queue.advance(), Some(&track2));

        // Test previous track
        assert_eq!(queue.previous(), Some(&track1));
    }

    #[test]
    fn test_playback_state_display() {
        // `PlaybackState` derives `Debug` (not `Display`); the Debug rendering
        // of each unit variant is its name, which is what this test verifies.
        assert_eq!(format!("{:?}", PlaybackState::Stopped), "Stopped");
        assert_eq!(format!("{:?}", PlaybackState::Playing), "Playing");
        assert_eq!(format!("{:?}", PlaybackState::Paused), "Paused");
    }

    #[test]
    fn test_track_id_from_path() {
        let track_id = TrackId::from_path(&std::path::PathBuf::from("path/to/track.mp3"));
        assert_eq!(track_id.0, "path/to/track.mp3");
    }

    #[test]
    fn test_track_display_methods() {
        let track = Track {
            id: TrackId("test.mp3".to_string()),
            file_path: std::path::PathBuf::from("test.mp3"),
            metadata: crate::domain::TrackMetadata::default(),
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            favorite: false,
            search_text: String::new(),
        };

        assert_eq!(track.metadata.display_artist(), "Unknown Artist");
        // With no title tag, `display_title` falls back to the file stem
        // ("test" for "test.mp3").
        assert_eq!(track.metadata.display_title(&track.file_path), "test");
        assert_eq!(track.metadata.display_album(), "Unknown Album");
    }

    // --- PlaybackQueue: empty / single-track behavior -----------------------

    #[test]
    fn test_empty_queue_returns_none_everywhere() {
        let mut queue = PlaybackQueue::default();
        assert!(queue.current_track().is_none());
        assert!(queue.advance().is_none());
        assert!(queue.previous().is_none());
        assert!(queue.upcoming(5).is_empty());
    }

    #[test]
    fn test_single_track_queue_navigation() {
        let track = TrackId("only.mp3".to_string());
        let mut queue = PlaybackQueue::new(vec![track.clone()]);
        queue.current_index = Some(0);

        assert_eq!(queue.current_track(), Some(&track));
        // Without repeat, there is nowhere to advance to.
        assert!(queue.advance().is_none());
        // `previous` at the first track has nowhere to go either; the
        // position stays.
        assert!(queue.previous().is_none());
        assert_eq!(queue.current_index, Some(0));
        // Nothing follows the only track.
        assert!(queue.upcoming(3).is_empty());
    }

    #[test]
    fn test_queue_new_constructor_defaults() {
        let queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
        ]);
        assert_eq!(queue.tracks.len(), 2);
        // The constructor starts a non-empty queue at its first track.
        assert_eq!(queue.current_index, Some(0));
        assert!(!queue.shuffle);
        assert_eq!(queue.repeat, RepeatMode::None);
        assert!(queue.shuffled_indices.is_empty());
        assert!(queue.shuffle_history.is_empty());
    }

    // --- PlaybackQueue: append / insert / remove ----------------------------

    #[test]
    fn test_append_and_insert_next_ordering() {
        let a = TrackId("a.mp3".to_string());
        let b = TrackId("b.mp3".to_string());
        let c = TrackId("c.mp3".to_string());
        let d = TrackId("d.mp3".to_string());

        let mut queue = PlaybackQueue::default();
        queue.append(a.clone());
        queue.append(b.clone());
        // The first append made `a` current; `insert_next` inserts after it.
        queue.insert_next(c.clone());
        assert_eq!(queue.tracks, vec![a.clone(), c.clone(), b.clone()]);

        queue.insert_next(d.clone());
        assert_eq!(queue.tracks, vec![a, d, c, b]);
        // Inserting does not move the current index.
        assert_eq!(queue.current_index, Some(0));
    }

    #[test]
    fn test_remove_before_current_shifts_index() {
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ]);
        queue.current_index = Some(2); // current = c

        queue.remove(0);
        assert_eq!(queue.tracks.len(), 2);
        assert_eq!(queue.current_index, Some(1));
        assert_eq!(queue.current_track(), Some(&TrackId("c.mp3".to_string())));
    }

    #[test]
    fn test_remove_current_falls_back_to_previous() {
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ]);
        queue.current_index = Some(1); // current = b

        queue.remove(1);
        assert_eq!(queue.tracks.len(), 2);
        // Removing the current track keeps the position sensible: it falls
        // back to the previous track.
        assert_eq!(queue.current_index, Some(0));
        assert_eq!(queue.current_track(), Some(&TrackId("a.mp3".to_string())));
    }

    #[test]
    fn test_remove_after_current_keeps_index() {
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ]);
        queue.current_index = Some(0); // current = a

        queue.remove(2);
        assert_eq!(queue.tracks.len(), 2);
        assert_eq!(queue.current_index, Some(0));
        assert_eq!(queue.current_track(), Some(&TrackId("a.mp3".to_string())));
    }

    #[test]
    fn test_remove_out_of_bounds_is_noop() {
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
        ]);
        queue.current_index = Some(1);

        queue.remove(99);
        assert_eq!(queue.tracks.len(), 2);
        assert_eq!(queue.current_index, Some(1));
    }

    // --- PlaybackQueue: repeat / previous ------------------------------------

    #[test]
    fn test_next_wraps_at_end_with_repeat_all() {
        let a = TrackId("a.mp3".to_string());
        let mut queue = PlaybackQueue::new(vec![a.clone(), TrackId("b.mp3".to_string())]);
        queue.current_index = Some(1); // at the end
        queue.repeat = RepeatMode::All;

        assert_eq!(queue.advance(), Some(&a));
        assert_eq!(queue.current_index, Some(0));
    }

    #[test]
    fn test_next_stops_at_end_without_repeat() {
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
        ]);
        queue.current_index = Some(1); // at the end
        assert_eq!(queue.repeat, RepeatMode::None);

        assert!(queue.advance().is_none());
        // Current position is left untouched.
        assert_eq!(queue.current_index, Some(1));
    }

    #[test]
    fn test_repeat_one_does_not_wrap_queue() {
        // `RepeatMode::One` is honored by the playback engine, not by the
        // queue: `next()` treats it like `None` and stops at the end.
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
        ]);
        queue.current_index = Some(1);
        queue.repeat = RepeatMode::One;

        assert!(queue.advance().is_none());
        assert_eq!(queue.repeat, RepeatMode::One);
    }

    #[test]
    fn test_repeat_all_wraps_single_track_queue_to_itself() {
        // A one-track queue with repeat-all keeps yielding the same track
        // instead of stopping at the end.
        let a = TrackId("only.mp3".to_string());
        let mut queue = PlaybackQueue::new(vec![a.clone()]);
        queue.current_index = Some(0);
        queue.repeat = RepeatMode::All;

        assert_eq!(queue.advance(), Some(&a));
        assert_eq!(queue.current_index, Some(0));
        // Wrapping is repeatable, not a one-shot.
        assert_eq!(queue.advance(), Some(&a));
        assert_eq!(queue.current_index, Some(0));
    }

    #[test]
    fn test_advance_walks_queue_in_order_until_end() {
        // Without repeat, `advance` plays tracks strictly in queue order from
        // the current position, then stops at the end.
        let a = TrackId("a.mp3".to_string());
        let b = TrackId("b.mp3".to_string());
        let c = TrackId("c.mp3".to_string());
        let mut queue = PlaybackQueue::new(vec![a.clone(), b.clone(), c.clone()]);
        queue.current_index = Some(0);

        assert_eq!(queue.advance(), Some(&b));
        assert_eq!(queue.advance(), Some(&c));
        assert!(queue.advance().is_none());
        // The position stays on the last track after the queue is exhausted.
        assert_eq!(queue.current_index, Some(2));
        assert_eq!(queue.current_track(), Some(&c));
    }

    #[test]
    fn test_previous_at_index_zero_stays_put() {
        let a = TrackId("a.mp3".to_string());
        let mut queue = PlaybackQueue::new(vec![a.clone(), TrackId("b.mp3".to_string())]);
        queue.current_index = Some(0);

        // `previous` at the first track has nowhere to go; the position
        // stays put.
        assert!(queue.previous().is_none());
        assert_eq!(queue.current_index, Some(0));
    }

    #[test]
    fn test_queue_new_starts_at_the_first_track() {
        let a = TrackId("a.mp3".to_string());
        let queue = PlaybackQueue::new(vec![a.clone(), TrackId("b.mp3".to_string())]);
        assert_eq!(queue.current_index, Some(0));
        assert_eq!(queue.current_track(), Some(&a));
    }

    #[test]
    fn test_toggle_repeat_cycles_none_all_one() {
        let mut queue = PlaybackQueue::default();
        assert_eq!(queue.repeat, RepeatMode::None);

        queue.toggle_repeat();
        assert_eq!(queue.repeat, RepeatMode::All);
        queue.toggle_repeat();
        assert_eq!(queue.repeat, RepeatMode::One);
        queue.toggle_repeat();
        assert_eq!(queue.repeat, RepeatMode::None);
    }

    // --- PlaybackQueue: shuffle / clear / upcoming ---------------------------

    #[test]
    fn test_set_shuffle_false_clears_shuffle_state() {
        let mut queue = PlaybackQueue::new(vec![
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ]);
        queue.current_index = Some(0);
        queue.set_shuffle(true);
        assert!(queue.shuffle);
        // Lazy regeneration: the order builds on the next advance, not on
        // enabling shuffle.
        assert!(queue.shuffled_indices.is_empty());
        assert!(queue.advance().is_some());
        assert!(!queue.shuffled_indices.is_empty());

        queue.set_shuffle(false);
        assert!(!queue.shuffle);
        assert!(queue.shuffled_indices.is_empty());
        assert!(queue.shuffle_history.is_empty());
    }

    #[test]
    fn test_shuffle_excludes_current_from_shuffled_indices() {
        let mut queue = PlaybackQueue::new((0..4).map(|i| TrackId(format!("t{i}.mp3"))).collect());
        queue.current_index = Some(2);
        queue.set_shuffle(true);
        assert!(queue.advance().is_some()); // builds the order, consumes its head

        // Deterministic invariant (order itself is random): the current
        // index at build time (2) is never part of the order, and the
        // consumed head plus the remaining order cover every other index
        // exactly once.
        assert!(!queue.shuffled_indices.contains(&2));
        let mut seen: Vec<usize> = vec![queue.current_index.unwrap()];
        seen.extend(queue.shuffled_indices.iter().copied());
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 3]);
    }

    #[test]
    fn test_shuffle_preserves_multiset() {
        // Shuffle must not drop or duplicate tracks. The exact order is random
        // (never asserted), but starting track + everything `next()` yields
        // must equal the original multiset.
        let ids: Vec<TrackId> = (0..8).map(|i| TrackId(format!("track{i}.mp3"))).collect();
        let mut queue = PlaybackQueue::new(ids.clone());
        queue.current_index = Some(0);
        let starting = queue.current_track().cloned().unwrap();

        queue.set_shuffle(true);

        let mut visited: Vec<TrackId> = Vec::new();
        while let Some(t) = queue.advance() {
            visited.push(t.clone());
            // Safety valve: the loop must terminate after the 7 other tracks.
            assert!(
                visited.len() <= ids.len(),
                "shuffle produced more tracks than the queue contains"
            );
        }

        assert_eq!(visited.len(), ids.len() - 1);
        let mut seen = visited;
        seen.push(starting);
        seen.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected = ids;
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(seen, expected);
    }

    #[test]
    fn test_advance_follows_seeded_order_front_to_back() {
        // The shuffle order is consumed strictly front-to-back (O(1)
        // `pop_front`, allocation plan 4.4): a hand-seeded order pins exact
        // navigation without any randomness.
        let mut queue = PlaybackQueue::new((1..=4).map(|i| TrackId(format!("t{i}.mp3"))).collect());
        queue.current_index = Some(0);
        queue.shuffle = true;
        queue.shuffled_indices = std::collections::VecDeque::from(vec![3, 1, 2]);

        assert_eq!(queue.advance(), Some(&TrackId("t4.mp3".to_string())));
        assert_eq!(queue.advance(), Some(&TrackId("t2.mp3".to_string())));
        assert_eq!(queue.advance(), Some(&TrackId("t3.mp3".to_string())));
        // Exhausted with repeat off: no further advance.
        assert_eq!(queue.advance(), None);
    }

    #[test]
    fn test_append_under_shuffle_defers_regeneration_until_advance() {
        // Lazy regeneration (allocation plan 4.4): a mutation marks the
        // order dirty instead of reshuffling immediately; the next advance
        // rebuilds it once. The rebuilt order must be a valid permutation
        // of every non-current track — including the just-appended one.
        let mut queue = PlaybackQueue::new((0..3).map(|i| TrackId(format!("t{i}.mp3"))).collect());
        queue.current_index = Some(0);
        queue.set_shuffle(true);
        let stale_len = queue.shuffled_indices.len();

        queue.append(TrackId("t3.mp3".to_string()));
        // The stale order survives until the rebuild (upcoming may briefly
        // reflect it); the dirty flag deferred the work.
        assert_eq!(queue.shuffled_indices.len(), stale_len);

        let mut visited: Vec<TrackId> = Vec::new();
        while let Some(t) = queue.advance() {
            visited.push(t.clone());
            assert!(visited.len() <= 3, "shuffle yielded more tracks than exist");
        }
        visited.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            visited,
            vec![
                TrackId("t1.mp3".to_string()),
                TrackId("t2.mp3".to_string()),
                TrackId("t3.mp3".to_string()),
            ]
        );
    }

    #[test]
    fn test_remove_under_shuffle_rebuilds_on_next_advance() {
        let mut queue = PlaybackQueue::new((0..4).map(|i| TrackId(format!("t{i}.mp3"))).collect());
        queue.current_index = Some(0);
        queue.set_shuffle(true);

        queue.remove(1); // drop t1

        let mut visited: Vec<TrackId> = Vec::new();
        while let Some(t) = queue.advance() {
            visited.push(t.clone());
            assert!(visited.len() <= 2, "removed track resurrected in shuffle");
        }
        visited.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            visited,
            vec![TrackId("t2.mp3".to_string()), TrackId("t3.mp3".to_string()),]
        );
    }

    #[test]
    fn test_shuffle_repeat_all_wraps_with_a_fresh_order() {
        let mut queue = PlaybackQueue::new((0..4).map(|i| TrackId(format!("t{i}.mp3"))).collect());
        queue.current_index = Some(0);
        queue.repeat = RepeatMode::All;
        queue.set_shuffle(true);

        for _ in 0..3 {
            assert!(queue.advance().is_some());
        }
        // Exhausted with repeat-all: the order regenerates and playback
        // continues instead of stopping.
        assert!(queue.advance().is_some());
    }

    #[test]
    fn test_append_many_matches_repeated_append() {
        // Batch enqueue (allocation plan 4.3) must be behaviorally
        // equivalent to appending one-by-one: identical final track list,
        // and under shuffle both drain the identical multiset.
        let ids: Vec<TrackId> = (0..6).map(|i| TrackId(format!("t{i}.mp3"))).collect();

        let mut batched = PlaybackQueue::new(vec![ids[0].clone()]);
        batched.current_index = Some(0);
        batched.set_shuffle(true);
        batched.append_many(ids[1..].to_vec());

        let mut looped = PlaybackQueue::new(vec![ids[0].clone()]);
        looped.current_index = Some(0);
        looped.set_shuffle(true);
        for id in &ids[1..] {
            looped.append(id.clone());
        }

        assert_eq!(batched.tracks, looped.tracks);

        let drain = |queue: &mut PlaybackQueue| {
            let mut visited: Vec<TrackId> = Vec::new();
            while let Some(t) = queue.advance() {
                visited.push(t.clone());
            }
            visited.sort_by(|a, b| a.0.cmp(&b.0));
            visited
        };
        assert_eq!(drain(&mut batched), drain(&mut looped));
    }

    #[test]
    fn test_clear_resets_queue() {
        let mut queue = PlaybackQueue::new((0..3).map(|i| TrackId(format!("t{i}.mp3"))).collect());
        queue.current_index = Some(1);
        queue.set_shuffle(true);

        queue.clear();
        assert!(queue.tracks.is_empty());
        assert_eq!(queue.current_index, None);
        assert!(queue.shuffled_indices.is_empty());
        assert!(queue.shuffle_history.is_empty());
    }

    #[test]
    fn test_upcoming_returns_following_tracks() {
        let ids: Vec<TrackId> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| TrackId(format!("{s}.mp3")))
            .collect();
        let mut queue = PlaybackQueue::new(ids.clone());

        // `new` starts at the first track, so upcoming follows from there.
        assert_eq!(queue.upcoming(2), vec![&ids[1], &ids[2]]);

        queue.current_index = Some(1);
        assert_eq!(queue.upcoming(2), vec![&ids[2], &ids[3]]);
        // Requests past the end are clamped, not padded.
        assert_eq!(queue.upcoming(10), vec![&ids[2], &ids[3]]);
    }

    // --- TrackId / PlaybackState / RepeatMode / PlaybackPosition -------------

    #[test]
    fn test_track_id_equality_and_hash() {
        let path = std::path::PathBuf::from("music/song.mp3");
        let id1 = TrackId::from_path(&path);
        let id2 = TrackId::from_path(&path);
        assert_eq!(id1, id2);

        // Equal ids must hash identically to be usable as map keys.
        let mut map = std::collections::HashMap::new();
        map.insert(id1.clone(), 1);
        assert_eq!(map.get(&id2), Some(&1));

        let other = TrackId::from_path(&std::path::PathBuf::from("music/other.mp3"));
        assert_ne!(id1, other);
    }

    #[test]
    fn test_playback_state_variants_are_distinct() {
        assert_ne!(PlaybackState::Stopped, PlaybackState::Playing);
        assert_ne!(PlaybackState::Playing, PlaybackState::Paused);
        assert_ne!(PlaybackState::Stopped, PlaybackState::Paused);

        // `Copy` semantics: assigning does not move.
        let state = PlaybackState::Playing;
        let copied = state;
        assert_eq!(state, copied);
    }

    #[test]
    fn test_repeat_mode_default_is_none() {
        assert_eq!(RepeatMode::default(), RepeatMode::None);
    }

    #[test]
    fn test_playback_position_default() {
        let position = PlaybackPosition::default();
        assert_eq!(position.current, std::time::Duration::ZERO);
        assert_eq!(position.total, None);
    }

    // --- TrackMetadata display / search --------------------------------------

    #[test]
    fn test_track_metadata_display_title_fallbacks() {
        let mut metadata = TrackMetadata::default();

        // No title, no usable file stem -> "Unknown".
        assert_eq!(
            metadata.display_title(&std::path::PathBuf::new()),
            "Unknown"
        );

        // No title -> file stem with underscores replaced by spaces.
        assert_eq!(
            metadata.display_title(&std::path::PathBuf::from("my_song_title.mp3")),
            "my song title"
        );

        // An explicit title always wins over the path fallback.
        metadata.title = Some("Real Title".to_string());
        assert_eq!(
            metadata.display_title(&std::path::PathBuf::from("ignored.mp3")),
            "Real Title"
        );
    }

    #[test]
    fn test_track_metadata_display_album_artist_fallback() {
        let mut metadata = TrackMetadata::default();
        // Nothing set -> falls through to "Unknown Artist".
        assert_eq!(metadata.display_album_artist(), "Unknown Artist");

        // No album artist -> falls back to the track artist.
        metadata.artist = Some("Track Artist".to_string());
        assert_eq!(metadata.display_album_artist(), "Track Artist");

        // An explicit album artist wins.
        metadata.album_artist = Some("Various Artists".to_string());
        assert_eq!(metadata.display_album_artist(), "Various Artists");
    }

    // --- SmartPlaylistKind + Track play-history fields (REQ-ML-009) --------

    #[test]
    fn test_smart_playlist_kind_display_names() {
        assert_eq!(SmartPlaylistKind::Favorites.display_name(), "Favorites");
        assert_eq!(
            SmartPlaylistKind::RecentlyAdded.display_name(),
            "Recently Added"
        );
        assert_eq!(SmartPlaylistKind::MostPlayed.display_name(), "Most Played");
        assert_eq!(
            SmartPlaylistKind::RecentlyPlayed.display_name(),
            "Recently Played"
        );
        assert_eq!(
            SmartPlaylistKind::NeverPlayed.display_name(),
            "Never Played"
        );
        assert_eq!(SmartPlaylistKind::LostGems.display_name(), "Lost Gems");
    }

    #[test]
    fn test_smart_playlist_kind_all_enumerates_every_kind_exactly_once() {
        let all = SmartPlaylistKind::ALL;
        assert_eq!(all.len(), 6);
        assert!(all.contains(&SmartPlaylistKind::Favorites));
        assert!(all.contains(&SmartPlaylistKind::RecentlyAdded));
        assert!(all.contains(&SmartPlaylistKind::MostPlayed));
        assert!(all.contains(&SmartPlaylistKind::RecentlyPlayed));
        assert!(all.contains(&SmartPlaylistKind::NeverPlayed));
        assert!(all.contains(&SmartPlaylistKind::LostGems));
    }

    #[test]
    fn test_track_metadata_search_text_is_lowercased() {
        let metadata = TrackMetadata {
            title: Some("My Song".to_string()),
            artist: Some("The Artist".to_string()),
            album: Some("Greatest Hits".to_string()),
            album_artist: Some("Various".to_string()),
            ..Default::default()
        };
        assert_eq!(
            metadata.search_text(),
            "my song the artist greatest hits various"
        );

        // Missing fields contribute empty segments.
        assert_eq!(TrackMetadata::default().search_text(), "   ");
    }
}
