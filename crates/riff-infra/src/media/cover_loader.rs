use riff_library::app::errors::LibraryError;
use riff_library::infra::ports::{CoverImage, CoverImageFormat, CoverLoader};
use riff_persistence::track::CoverSource;

/// Map the image crate's detected container format onto the port's own enum.
/// The cover-decoding features enabled here are exactly JPEG and PNG; any
/// other container cannot be decoded and is reported as unsupported.
fn cover_format(format: image::ImageFormat) -> Option<CoverImageFormat> {
    match format {
        image::ImageFormat::Jpeg => Some(CoverImageFormat::Jpeg),
        image::ImageFormat::Png => Some(CoverImageFormat::Png),
        _ => None,
    }
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
    /// Hand out the still-encoded image bytes plus their detected container
    /// format, so the decode happens once on the UI thread when the texture
    /// is built.
    fn load_cover(&self, source: &CoverSource) -> Result<Option<CoverImage>, LibraryError> {
        let bytes: &[u8] = match source {
            CoverSource::Embedded(data) => data,
            CoverSource::Filesystem(path) => &std::fs::read(path)
                .map_err(|e| LibraryError::CoverLoad(format!("Cover read error: {e}")))?,
            CoverSource::None => return Ok(None),
        };
        let format = image::guess_format(bytes)
            .map_err(|e| LibraryError::CoverLoad(format!("Image format error: {e}")))?;
        let format = cover_format(format).ok_or_else(|| {
            LibraryError::CoverLoad(format!("Unsupported cover image format: {format:?}"))
        })?;
        Ok(Some(CoverImage {
            data: bytes.to_vec(),
            format,
        }))
    }
}
