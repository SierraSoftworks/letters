//! Decoded images, and the decoder for a document's image assets.
//!
//! Decoding is separate from drawing so the shell can do it after first paint: a display list names an
//! asset, and a backend draws it once its [`Image`] is in the [`Images`] it is given.

use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ::image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use vello_cpu::peniko::ImageAlphaType;
use vello_cpu::{PixelMetadata, Pixmap};

/// The longest side an image may have: the rasterisers address pixels with 16 bits.
pub const MAX_SIDE: u32 = u16::MAX as u32;
/// The most pixels an image may have, which caps a decoded image at 256 MB.
pub const MAX_PIXELS: u64 = 1 << 26;

/// Why an image could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG, JPEG, GIF or WebP.
    Unsupported,
    /// Larger than [`MAX_SIDE`] or [`MAX_PIXELS`].
    TooLarge,
    /// Damaged or truncated data.
    Invalid(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("not a PNG, JPEG, GIF or WebP image"),
            Self::TooLarge => f.write_str("the image is too large"),
            Self::Invalid(reason) => write!(f, "the image is damaged: {reason}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<::image::ImageError> for ImageError {
    fn from(error: ::image::ImageError) -> Self {
        match error {
            ::image::ImageError::Unsupported(_) => Self::Unsupported,
            ::image::ImageError::Limits(_) => Self::TooLarge,
            other => Self::Invalid(other.to_string()),
        }
    }
}

/// Decoded pixels, ready to draw. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Image {
    pub(crate) pixels: Arc<Pixmap>,
    /// Unique to these pixels for the life of the process, so a renderer can tell a replaced asset.
    pub(crate) id: u64,
    /// The file the pixels were decoded from, when it is a JPEG that shows the same picture as it stands.
    /// A PDF stores that rather than the pixels, which for a photo are many times larger.
    #[cfg(feature = "pdf")]
    pub(crate) jpeg: Option<Arc<[u8]>>,
}

impl Image {
    /// Decodes a PNG, JPEG, GIF (first frame) or WebP, upright according to its EXIF orientation.
    pub fn decode(bytes: &[u8]) -> Result<Self, ImageError> {
        let mut reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| ImageError::Invalid(error.to_string()))?;
        let Some(format) = reader.format() else { return Err(ImageError::Unsupported) };
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_SIDE);
        limits.max_image_height = Some(MAX_SIDE);
        reader.limits(limits);

        let mut decoder = reader.into_decoder()?;
        let (width, height) = decoder.dimensions();
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(ImageError::TooLarge);
        }
        let orientation = decoder.orientation()?;
        let mut image = DynamicImage::from_decoder(decoder)?;
        image.apply_orientation(orientation);
        let image = image.into_rgba8();
        #[cfg_attr(not(feature = "pdf"), expect(unused_mut, reason = "only the PDF backend keeps the file"))]
        let mut image = Self::from_rgba8(image.width(), image.height(), image.into_raw())?;
        #[cfg(feature = "pdf")]
        if format == ::image::ImageFormat::Jpeg && orientation == ::image::metadata::Orientation::NoTransforms {
            image.jpeg = Some(bytes.into());
        }
        #[cfg(not(feature = "pdf"))]
        let _ = format;
        Ok(image)
    }

    /// Wraps straight (not premultiplied) RGBA8 pixels, row by row from the top left.
    pub fn from_rgba8(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ImageError> {
        let pixels = u64::from(width) * u64::from(height);
        if width > MAX_SIDE || height > MAX_SIDE || pixels > MAX_PIXELS {
            return Err(ImageError::TooLarge);
        }
        if pixels == 0 || rgba.len() as u64 != pixels * 4 {
            return Err(ImageError::Invalid(format!("{} bytes for {width} by {height} pixels", rgba.len())));
        }
        // Rebuilt so the capacity is a whole number of pixels, which the pixmap requires.
        let rgba = rgba.into_boxed_slice().into_vec();
        let metadata = PixelMetadata::new(ImageAlphaType::Alpha, true);
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let pixels = Arc::new(Pixmap::from_parts(rgba, width as u16, height as u16, metadata));
        Ok(Self {
            pixels,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            #[cfg(feature = "pdf")]
            jpeg: None,
        })
    }

    pub fn width(&self) -> u32 {
        self.pixels.width().into()
    }

    pub fn height(&self) -> u32 {
        self.pixels.height().into()
    }
}

/// The decoded images a display list may draw, by asset entry name.
#[derive(Clone, Debug, Default)]
pub struct Images {
    images: HashMap<String, Image>,
}

impl Images {
    pub fn insert(&mut self, asset: impl Into<String>, image: Image) {
        self.images.insert(asset.into(), image);
    }

    pub fn remove(&mut self, asset: &str) -> Option<Image> {
        self.images.remove(asset)
    }

    pub fn get(&self, asset: &str) -> Option<&Image> {
        self.images.get(asset)
    }
}
