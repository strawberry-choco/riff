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
