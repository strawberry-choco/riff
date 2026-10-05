//! `ReplayGain` measurement domain math.
//!
//! The measurement standard is `ReplayGain` 2.0 — BS.1770 integrated
//! loudness against a −18 LUFS reference (ADR 0013). One decode of a Track
//! yields its track values; the Album aggregate is pure math over the
//! members' measurements plus their durations, so it lives here beside the
//! reference constant rather than inside any adapter or store read.
//!
//! The aggregate is the **energy-weighted** mean of the members' loudnesses,
//! weighted by duration: a three-minute opener contributes three times the
//! energy of a one-minute interlude, exactly as the Album would play as one
//! program. The album peak is the maximum of the member peaks — the loudest
//! sample anywhere in the Album is what clipping prevention must respect.

use crate::app::traits::TrackLoudness;

/// The `ReplayGain` 2.0 reference loudness: a Track measuring this many
/// LUFS plays at unity gain.
pub const REPLAYGAIN_REFERENCE_LUFS: f64 = -18.0;

/// The BS.1770 offset between linear energy and loudness: a channel of mean
/// square energy `E` measures `−0.691 + 10·log10(E)` LUFS. The aggregate
/// works in energy space, so the constant travels with the formula.
const LOUDNESS_ENERGY_OFFSET_DB: f64 = -0.691;

/// The gain a Track with no loudness verdict falls back to — digital silence,
/// whose gated BS.1770 measurement is −inf LUFS and whose gain would otherwise
/// come back infinite. `0 dB` is the one value that provably neither amplifies
/// nor attenuates audio the measurement could not hear, and it is the value
/// every tag form and every other player can carry: an infinite one reads
/// `inf dB` to nothing, saturates Opus's Q7.8 integer, and would weigh a
/// silent member's duration against no sound in [`album_aggregate`].
pub const REPLAYGAIN_FALLBACK_GAIN_DB: f32 = 0.0;

/// The peak counterpart of [`REPLAYGAIN_FALLBACK_GAIN_DB`] for a measurement
/// that reported none: full scale, the conservative bound, since a cap of
/// `1.0 / peak` at `1.0` cannot let an amplified sample clip. A measured peak
/// of exactly `0.0` is a real fact about silent audio, not a missing one, so
/// the measurement keeps it rather than substituting this.
pub const REPLAYGAIN_FALLBACK_PEAK: f32 = 1.0;

/// An Album's aggregate `ReplayGain` values, written to every Track of the
/// Album: one fact shared by all its members.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlbumAggregate {
    /// Album gain in dB — the value that brings the whole Album, played
    /// back to back, to the reference loudness.
    pub album_gain_db: f32,
    /// Album peak as a linear ratio — the maximum of the member true peaks.
    pub album_peak: f32,
}

/// One measured member of an Album: its track measurement and its duration
/// in seconds (the energy weight).
pub type MeasuredMember = (TrackLoudness, f32);

/// The energy-weighted Album aggregate over the measured members: `None`
/// when no member carries a positive duration (there is nothing to
/// aggregate).
///
/// Members without a measurement are simply absent from `members` — the
/// caller decides which Tracks of the Album are measured, and the aggregate
/// describes exactly the set it is handed. A single-member Album
/// degenerates to that member's own track gain, the invariant the formula
/// must reduce to.
#[must_use]
pub fn album_aggregate(members: &[MeasuredMember]) -> Option<AlbumAggregate> {
    let mut energy = 0.0_f64;
    let mut weight = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut measured = false;
    for (loudness, duration) in members {
        if *duration <= 0.0 {
            continue;
        }
        measured = true;
        // Loudness → energy: `E = 10^((L + 0.691) / 10)`, weighted by the
        // member's duration. Track gain and loudness are two views of one
        // fact: `gain = reference − L`, so `L = reference − gain`.
        let loudness_lufs = REPLAYGAIN_REFERENCE_LUFS - f64::from(loudness.track_gain_db);
        energy += f64::from(*duration)
            * 10.0_f64.powf((loudness_lufs - LOUDNESS_ENERGY_OFFSET_DB) / 10.0);
        weight += f64::from(*duration);
        peak = peak.max(loudness.track_peak);
    }
    if !measured || weight <= 0.0 {
        return None;
    }
    let album_loudness = LOUDNESS_ENERGY_OFFSET_DB + 10.0 * (energy / weight).log10();
    Some(AlbumAggregate {
        // Narrowing the same way the reader narrows the store's REAL columns:
        // the value was computed from f32 inputs, so the f32 form is its
        // native width.
        #[allow(clippy::cast_possible_truncation)]
        album_gain_db: (REPLAYGAIN_REFERENCE_LUFS - album_loudness) as f32,
        album_peak: peak,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(gain_db: f32, peak: f32, duration_seconds: f32) -> MeasuredMember {
        (
            TrackLoudness {
                track_gain_db: gain_db,
                track_peak: peak,
            },
            duration_seconds,
        )
    }

    /// The worked example from the adapter tests, computed by hand from the
    /// formula: members at −9.03 LUFS for 2 s and −15.03 LUFS for 1 s
    /// aggregate to −10.28 LUFS → −7.72 dB of album gain, peak 0.5.
    #[test]
    fn aggregate_is_the_energy_weighted_mean_weighted_by_duration() {
        let album = album_aggregate(&[member(-8.9689, 0.5, 2.0), member(-2.9691, 0.25, 1.0)])
            .expect("measured members aggregate");
        assert!((album.album_gain_db - (-7.72)).abs() < 0.05);
        assert!((album.album_peak - 0.5).abs() < 1e-6);
    }

    /// One member degenerates to its own track gain — the formula's invariant.
    #[test]
    fn a_single_member_aggregates_to_its_own_track_gain() {
        let album = album_aggregate(&[member(-6.54, 0.75, 3.5)]).expect("one member aggregates");
        assert!((album.album_gain_db - (-6.54)).abs() < 1e-4);
        assert!((album.album_peak - 0.75).abs() < 1e-6);
    }

    /// A zero-duration member contributes no energy and no weight — it is
    /// as if it were never handed in.
    #[test]
    fn zero_duration_members_contribute_nothing() {
        assert!(album_aggregate(&[member(-6.54, 0.75, 0.0)]).is_none());
        let with_a_live_member =
            album_aggregate(&[member(-6.54, 0.75, 0.0), member(-6.54, 0.75, 2.0)])
                .expect("the live member aggregates");
        assert!((with_a_live_member.album_gain_db - (-6.54)).abs() < 1e-4);
    }

    #[test]
    fn no_members_aggregate_to_none() {
        assert!(album_aggregate(&[]).is_none());
    }

    /// The peak is the maximum over members even when the loudest peak
    /// belongs to the quietest (most attenuated) member.
    #[test]
    fn album_peak_is_the_max_over_members() {
        let album = album_aggregate(&[member(-3.0, 0.25, 1.0), member(-12.0, 0.9, 4.0)])
            .expect("measured members aggregate");
        assert!((album.album_peak - 0.9).abs() < 1e-6);
    }
}
