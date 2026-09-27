use riff_library::app::errors::LibraryError;
use riff_library::app::traits::{CoverLoader, DecodedCover, RequestedSize};
use riff_persistence::thumbnail::EncodedThumbnail;
use riff_persistence::track::CoverSource;
use std::io::Cursor;

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
    /// Obtain the bytes and hand them to [`decode_thumbnail`] — the whole of
    /// this port that is about *where art comes from*.
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
        decode_thumbnail(bytes, size).map(Some)
    }

    /// Delegates to the crate's one [`decode_thumbnail`] — the cached route and
    /// the source route share it rather than imitating each other.
    fn decode_thumbnail(
        &self,
        bytes: &[u8],
        size: RequestedSize,
    ) -> Result<DecodedCover, LibraryError> {
        decode_thumbnail(bytes, size)
    }

    fn encode_thumbnail(&self, cover: &DecodedCover) -> Result<EncodedThumbnail, LibraryError> {
        encode_thumbnail(cover)
    }
}

/// Decode already-obtained bytes and fit them into `size`.
///
/// The whole of the old `load_cover` body from the decode step onward, split out
/// so the Thumbnail cache can hand back stored bytes without inventing a second
/// implementation of this — one decode, one format allowlist, one resample rule,
/// whichever route the bytes arrived by.
///
/// Do the actual decode — read the container, decode it, convert to RGBA8 — so
/// the result crossing the port boundary is pixels rather than still-encoded
/// bytes. The features enabled here are exactly JPEG and PNG; any other
/// container is reported as unsupported.
pub fn decode_thumbnail(bytes: &[u8], size: RequestedSize) -> Result<DecodedCover, LibraryError> {
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
    Ok(DecodedCover {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

/// JPEG quality for a stored Thumbnail.
///
/// The rung is chosen to keep the design's size budget — about 2.5 KB at 56×56,
/// 15 KB at 200×200 and 65 KB at 512×512 — and lossy storage is acceptable *at
/// all* only because a Thumbnail is **terminal**: the pixels are written from the
/// source's own decode and are never the input to a larger rung. A ladder built
/// by re-deriving 200×200 from a 56×56 JPEG would compound generations, and this
/// design does not do that.
const THUMBNAIL_JPEG_QUALITY: u8 = 90;

/// Encode a decoded Cover into the bytes the Thumbnail cache stores.
///
/// **The alpha decision, made deliberately rather than by accident.** JPEG has
/// no alpha channel, but `DecodedCover::rgba` is unpremultiplied RGBA and egui
/// consumes it unpremultiplied — so storing alpha-carrying art as JPEG would bake
/// in a background the app never chose, in a place where the real surface colour
/// is what shows through today. Rather than flatten over a invented colour, or
/// drop the Cover from the cache, a Cover with any non-opaque pixel is stored
/// losslessly as PNG. That keeps every rung's rendering faithful; the cost is a
/// bigger entry for the minority of Covers that carry alpha, and a container the
/// cache already has to sniff from the bytes rather than trust a name for.
///
/// `Err` only for pixels that do not fill the dimensions they claim, which is a
/// broken `DecodedCover` rather than a cache failure.
pub fn encode_thumbnail(cover: &DecodedCover) -> Result<EncodedThumbnail, LibraryError> {
    let opaque = cover.rgba.chunks_exact(4).all(|pixel| pixel[3] == u8::MAX);
    let mut out = Cursor::new(Vec::new());

    if opaque {
        let mut rgb = Vec::with_capacity(cover.rgba.len() / 4 * 3);
        for pixel in cover.rgba.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        let frame = image::RgbImage::from_raw(cover.width, cover.height, rgb).ok_or_else(|| {
            LibraryError::CoverLoad(format!(
                "cover claims {}x{} but carries {} bytes",
                cover.width,
                cover.height,
                cover.rgba.len()
            ))
        })?;
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, THUMBNAIL_JPEG_QUALITY)
            .encode_image(&frame)
            .map_err(|e| LibraryError::CoverLoad(format!("Thumbnail encode error: {e}")))?;
    } else {
        let frame = image::RgbaImage::from_raw(cover.width, cover.height, cover.rgba.clone())
            .ok_or_else(|| {
                LibraryError::CoverLoad(format!(
                    "cover claims {}x{} but carries {} bytes",
                    cover.width,
                    cover.height,
                    cover.rgba.len()
                ))
            })?;
        frame
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| LibraryError::CoverLoad(format!("Thumbnail encode error: {e}")))?;
    }

    Ok(EncodedThumbnail {
        bytes: out.into_inner(),
        width: cover.width,
        height: cover.height,
    })
}
