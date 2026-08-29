use crate::app::errors::LibraryError;
use crate::app::traits::CoverLoader;
use crate::domain::CoverSource;

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
    fn load_cover(
        &self,
        source: &CoverSource,
    ) -> Result<Option<crate::app::traits::CoverImage>, LibraryError> {
        match source {
            CoverSource::Embedded(data) => {
                let image = image::load_from_memory(data)
                    .map_err(|e| LibraryError::CoverLoad(format!("Image decode error: {e}")))?;
                let rgba = image.to_rgba8();
                let (width, height) = rgba.dimensions();
                Ok(Some(crate::app::traits::CoverImage {
                    width,
                    height,
                    rgba: rgba.into_raw(),
                }))
            }
            CoverSource::Filesystem(path) => {
                let image = image::open(path)
                    .map_err(|e| LibraryError::CoverLoad(format!("Image open error: {e}")))?;
                let rgba = image.to_rgba8();
                let (width, height) = rgba.dimensions();
                Ok(Some(crate::app::traits::CoverImage {
                    width,
                    height,
                    rgba: rgba.into_raw(),
                }))
            }
            CoverSource::None => Ok(None),
        }
    }
}

/// The library capability's cover port: hand out the still-encoded image
/// bytes plus their detected container format, so the decode happens once on
/// the UI thread when the texture is built.
impl riff_library::infra::ports::CoverLoader for ImageCoverLoader {
    fn load_cover(
        &self,
        source: &CoverSource,
    ) -> Result<
        Option<riff_library::infra::ports::CoverImage>,
        riff_library::app::errors::LibraryError,
    > {
        use riff_library::app::errors::LibraryError as LibraryErrorL;
        let bytes: &[u8] = match source {
            CoverSource::Embedded(data) => data,
            CoverSource::Filesystem(path) => &std::fs::read(path)
                .map_err(|e| LibraryErrorL::CoverLoad(format!("Cover read error: {e}")))?,
            CoverSource::None => return Ok(None),
        };
        let format = image::guess_format(bytes)
            .map_err(|e| LibraryErrorL::CoverLoad(format!("Image format error: {e}")))?;
        Ok(Some(riff_library::infra::ports::CoverImage {
            data: bytes.to_vec(),
            format,
        }))
    }
}
