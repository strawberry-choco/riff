use riff_library::app::errors::LibraryError;
use riff_library::app::traits::{CoverLoader, DecodedCover, RequestedSize};
use riff_persistence::track::CoverSource;

/// The largest `(w, h)` preserving the source aspect ratio that fits inside
/// the box, clamped to the source dimensions so a small cover is never blown
/// up. Integer arithmetic throughout: the answer is a pixel count, and a
/// float round-trip would only add ways to be off by one.
fn fit_within(source_w: u32, source_h: u32, box_w: u32, box_h: u32) -> (u32, u32) {
    if source_w <= box_w && source_h <= box_h {
        return (source_w, source_h);
    }
    // The binding side is whichever limit is hit first; comparing the two
    // ratios crosswise decides that exactly, without dividing.
    if u64::from(source_w) * u64::from(box_h) >= u64::from(source_h) * u64::from(box_w) {
        (box_w, scale_within(source_h, box_w, source_w))
    } else {
        (scale_within(source_w, box_h, source_h), box_h)
    }
}

/// `value * numerator / denominator`, rounded to nearest and floored at one
/// pixel. Callers only ever scale down, so the quotient cannot exceed `value`.
fn scale_within(value: u32, numerator: u32, denominator: u32) -> u32 {
    let product = u64::from(value) * u64::from(numerator);
    let rounded = (product + u64::from(denominator) / 2) / u64::from(denominator);
    u32::try_from(rounded).unwrap_or(value).max(1)
}

/// [`CoverLoader`] implementation backed by the `image` crate.
pub struct ImageCoverLoader;

impl Default for ImageCoverLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageCoverLoader {
    pub fn new() -> Self {
        Self
    }
}

impl CoverLoader for ImageCoverLoader {
    /// Do the actual decode — read the container, decode it, convert to
    /// RGBA8 — so the result crossing the port boundary is pixels rather than
    /// still-encoded bytes. The features enabled here are exactly JPEG and
    /// PNG; any other container is reported as unsupported.
    fn load_cover(
        &self,
        source: &CoverSource,
        size: RequestedSize,
    ) -> Result<Option<DecodedCover>, LibraryError> {
        let bytes: &[u8] = match source {
            CoverSource::Embedded(data) => data,
            CoverSource::Filesystem(path) => &std::fs::read(path)
                .map_err(|e| LibraryError::CoverLoad(format!("Cover read error: {e}")))?,
            CoverSource::None => return Ok(None),
        };
        let format = image::guess_format(bytes)
            .map_err(|e| LibraryError::CoverLoad(format!("Image format error: {e}")))?;
        if !matches!(format, image::ImageFormat::Jpeg | image::ImageFormat::Png) {
            return Err(LibraryError::CoverLoad(format!(
                "Unsupported cover image format: {format:?}"
            )));
        }
        let decoded = image::load_from_memory_with_format(bytes, format)
            .map_err(|e| LibraryError::CoverLoad(format!("Image decode error: {e}")))?;
        let (source_w, source_h) = (decoded.width(), decoded.height());
        let (width, height) = fit_within(source_w, source_h, size.width, size.height);
        // Skipping the resample when the source already fits keeps a small
        // cover byte-identical to a plain decode.
        let resized = if (width, height) == (source_w, source_h) {
            decoded
        } else {
            decoded.resize(width, height, image::imageops::FilterType::Triangle)
        };
        let rgba = resized.to_rgba8();
        Ok(Some(DecodedCover {
            rgba: rgba.into_raw(),
            width,
            height,
        }))
    }
}
