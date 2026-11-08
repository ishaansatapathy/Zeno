//! Images that visitors see (course thumbnails, creator avatars).
//!
//! Uploads are untrusted input, so nothing a creator sends is ever stored or served as sent:
//! the bytes are sniffed (the declared content type is not believed), decoded under hard size
//! limits, rotated according to their EXIF orientation, cropped, scaled down and re-encoded as a
//! fresh JPEG. The re-encode drops every piece of metadata (EXIF location, comments, embedded
//! thumbnails) and anything that was not pixels, so a polyglot file or a script hidden in a
//! chunk cannot survive. Formats that can carry active content or animation bombs (SVG, GIF)
//! are refused outright.
//!
//! Stored objects are named by the hash of their content, so a URL never changes meaning and can
//! be cached forever.

use std::io::Cursor;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderMap, StatusCode};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// Largest upload accepted, in bytes.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Largest width or height decoded. A 4096 x 4096 RGBA decode is 64 MiB, which is the most one
/// request is allowed to cost.
const MAX_SIDE: u32 = 4096;
const MAX_ALLOC: u64 = 128 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;
/// Decodes running at once. Further uploads wait their turn instead of multiplying memory use.
static DECODE_SLOTS: Semaphore = Semaphore::const_new(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    CourseThumbnail,
    Avatar,
}

impl ImageKind {
    /// The stored size (never upscaled) as width x height.
    fn target(self) -> (u32, u32) {
        match self {
            ImageKind::CourseThumbnail => (1280, 720),
            ImageKind::Avatar => (512, 512),
        }
    }

    /// Smaller sources look bad once cropped and shown large, so they are refused.
    fn minimum(self) -> (u32, u32) {
        match self {
            ImageKind::CourseThumbnail => (320, 180),
            ImageKind::Avatar => (128, 128),
        }
    }

    pub fn key(self, owner: Uuid, content_hash: &str) -> String {
        match self {
            ImageKind::CourseThumbnail => format!("images/courses/{owner}/{content_hash}.jpg"),
            ImageKind::Avatar => format!("images/avatars/{owner}/{content_hash}.jpg"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImageError {
    /// More bytes than [`MAX_IMAGE_BYTES`].
    TooLarge,
    /// Not a JPEG, PNG or WebP, whatever the client called it.
    UnsupportedType,
    /// Looks like an image but cannot be decoded (truncated, corrupt, over the limits).
    Invalid,
    /// Decodes, but is smaller than the minimum for this use.
    TooSmall { width: u32, height: u32 },
    /// A bug or a panicking worker; never the client's fault.
    Failed,
}

impl From<ImageError> for ApiError {
    fn from(error: ImageError) -> Self {
        let (status, code, message) = match error {
            ImageError::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "image_too_large",
                format!(
                    "Images can be at most {} MB.",
                    MAX_IMAGE_BYTES / (1024 * 1024)
                ),
            ),
            ImageError::UnsupportedType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_image",
                "Upload a JPEG, PNG or WebP image.".to_owned(),
            ),
            ImageError::Invalid => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_image",
                format!("The image is damaged or larger than {MAX_SIDE} x {MAX_SIDE} pixels."),
            ),
            ImageError::TooSmall { width, height } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "image_too_small",
                format!("The image is {width} x {height} pixels, which is too small."),
            ),
            ImageError::Failed => {
                return ApiError::Internal(anyhow::anyhow!("image processing failed"))
            }
        };
        ApiError::Http {
            status,
            code,
            message,
        }
    }
}

/// A finished image, ready to store.
#[derive(Debug)]
pub struct Processed {
    pub jpeg: Vec<u8>,
    /// First 128 bits of the SHA-256 of `jpeg`, in hex.
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
}

/// Content types a client may declare. Checked first only to fail fast; the bytes decide.
pub fn declared_type_is_acceptable(content_type: Option<&str>) -> bool {
    let Some(raw) = content_type else {
        return false;
    };
    let essence = raw.split(';').next().unwrap_or("").trim();
    essence.eq_ignore_ascii_case("image/jpeg")
        || essence.eq_ignore_ascii_case("image/png")
        || essence.eq_ignore_ascii_case("image/webp")
}

/// Pure and CPU-bound; call through [`process_async`] from request handlers.
pub fn process(bytes: &[u8], kind: ImageKind) -> Result<Processed, ImageError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ImageError::Invalid)?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)
    ) {
        return Err(ImageError::UnsupportedType);
    }
    let mut reader = reader;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);

    let mut decoder = reader.into_decoder().map_err(|_| ImageError::Invalid)?;
    let orientation = decoder.orientation().map_err(|_| ImageError::Invalid)?;
    let (width, height) = decoder.dimensions();
    let (min_w, min_h) = kind.minimum();
    if width < min_w || height < min_h {
        return Err(ImageError::TooSmall { width, height });
    }
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|_| ImageError::Invalid)?;
    decoded.apply_orientation(orientation);

    let flat = flatten_on_white(&decoded);
    let (target_w, target_h) = kind.target();
    let cropped = crop_to_aspect(flat, target_w, target_h);
    let scaled = if cropped.width() > target_w {
        imageops::resize(&cropped, target_w, target_h, FilterType::Lanczos3)
    } else {
        cropped
    };

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&scaled)
        .map_err(|_| ImageError::Failed)?;
    let digest = Sha256::digest(&jpeg);
    let content_hash = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok(Processed {
        jpeg,
        content_hash,
        width: scaled.width(),
        height: scaled.height(),
    })
}

/// Runs [`process`] off the async threads and at most a few at a time.
pub async fn process_async(bytes: Vec<u8>, kind: ImageKind) -> Result<Processed, ImageError> {
    let _slot = DECODE_SLOTS
        .acquire()
        .await
        .map_err(|_| ImageError::Failed)?;
    tokio::task::spawn_blocking(move || process(&bytes, kind))
        .await
        .map_err(|_| ImageError::Failed)?
}

/// Transparent pixels become white: JPEG has no alpha, and black backgrounds would make
/// transparent PNG logos unreadable.
fn flatten_on_white(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let alpha = u32::from(a);
        let blend = |channel: u8| -> u8 {
            ((u32::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        };
        out.put_pixel(x, y, image::Rgb([blend(r), blend(g), blend(b)]));
    }
    out
}

/// The largest centred region with the target's aspect ratio.
fn crop_to_aspect(image: RgbImage, target_w: u32, target_h: u32) -> RgbImage {
    let (w, h) = image.dimensions();
    // Compare w/h with target_w/target_h without floating point.
    let wide = u64::from(w) * u64::from(target_h) > u64::from(h) * u64::from(target_w);
    let (crop_w, crop_h) = if wide {
        (
            (u64::from(h) * u64::from(target_w) / u64::from(target_h)) as u32,
            h,
        )
    } else {
        (
            w,
            (u64::from(w) * u64::from(target_h) / u64::from(target_w)) as u32,
        )
    };
    let (crop_w, crop_h) = (crop_w.clamp(1, w), crop_h.clamp(1, h));
    if (crop_w, crop_h) == (w, h) {
        return image;
    }
    imageops::crop_imm(&image, (w - crop_w) / 2, (h - crop_h) / 2, crop_w, crop_h).to_image()
}

/// The address a browser should load `key` from.
///
/// With `IMAGE_BASE_URL` set (a CDN in front of the bucket) the URL is permanent and cacheable.
/// Otherwise it is a presigned link to the private bucket, good for an hour.
pub fn public_url(state: &AppState, key: &str) -> String {
    match &state.config.image_base_url {
        Some(base) => format!("{base}/{key}"),
        None => state
            .store
            .presign_get(key, Duration::from_secs(3600))
            .to_string(),
    }
}

/// Stores a finished image under its content-addressed key and returns that key.
pub async fn store(
    state: &AppState,
    kind: ImageKind,
    owner: Uuid,
    image: Processed,
) -> Result<String, ApiError> {
    let key = kind.key(owner, &image.content_hash);
    state
        .store
        .put_bytes(&key, image.jpeg, "image/jpeg")
        .await
        .map_err(|err| ApiError::Internal(err.into()))?;
    Ok(key)
}

/// Removes an image nobody refers to any more. A failure is only logged: the object is
/// unreachable either way and the next sweep of the prefix removes it.
pub async fn discard(state: &AppState, key: &str) {
    if let Err(err) = state.store.delete(key).await {
        tracing::warn!(%key, error = %err, "could not delete an unused image");
    }
}

/// The request body limit for image upload routes (the rest of the API allows far less).
pub fn upload_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_IMAGE_BYTES)
}

/// Turns the raw request body of an image upload into bytes, or the right error: `413` when it
/// is over the limit, `415` when the declared content type is not an image we accept.
pub fn read_upload(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Vec<u8>, ApiError> {
    let declared = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if !declared_type_is_acceptable(declared) {
        return Err(ImageError::UnsupportedType.into());
    }
    match body {
        Ok(bytes) if bytes.is_empty() => Err(ImageError::Invalid.into()),
        Ok(bytes) => Ok(bytes.to_vec()),
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            Err(ImageError::TooLarge.into())
        }
        Err(_) => Err(ImageError::Invalid.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};

    fn png(width: u32, height: u32, fill: Rgba<u8>) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, fill);
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_pixel(width, height, image::Rgb([20, 120, 220]));
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&image)
            .unwrap();
        out
    }

    fn decode(bytes: &[u8]) -> DynamicImage {
        image::load_from_memory_with_format(bytes, ImageFormat::Jpeg).unwrap()
    }

    #[test]
    fn a_large_png_becomes_a_16_by_9_jpeg() {
        let done = process(
            &png(2400, 1800, Rgba([10, 20, 30, 255])),
            ImageKind::CourseThumbnail,
        )
        .unwrap();
        assert_eq!((done.width, done.height), (1280, 720));
        assert_eq!(&done.jpeg[..3], [0xFF, 0xD8, 0xFF], "JPEG magic");
        assert_eq!(decode(&done.jpeg).width(), 1280);
        assert_eq!(done.content_hash.len(), 32);
    }

    #[test]
    fn avatars_are_centred_squares_and_small_sources_are_not_upscaled() {
        let done = process(&jpeg(600, 300), ImageKind::Avatar).unwrap();
        assert_eq!((done.width, done.height), (300, 300));
        let done = process(&jpeg(2000, 1000), ImageKind::Avatar).unwrap();
        assert_eq!((done.width, done.height), (512, 512));
    }

    #[test]
    fn identical_uploads_get_the_same_name_and_different_ones_do_not() {
        let a = process(
            &png(800, 450, Rgba([1, 2, 3, 255])),
            ImageKind::CourseThumbnail,
        )
        .unwrap();
        let b = process(
            &png(800, 450, Rgba([1, 2, 3, 255])),
            ImageKind::CourseThumbnail,
        )
        .unwrap();
        let c = process(
            &png(800, 450, Rgba([200, 2, 3, 255])),
            ImageKind::CourseThumbnail,
        )
        .unwrap();
        assert_eq!(a.content_hash, b.content_hash);
        assert_ne!(a.content_hash, c.content_hash);
        let owner = Uuid::nil();
        assert_eq!(
            ImageKind::Avatar.key(owner, &a.content_hash),
            format!("images/avatars/{owner}/{}.jpg", a.content_hash)
        );
    }

    #[test]
    fn transparency_is_flattened_onto_white() {
        let done = process(
            &png(640, 360, Rgba([0, 0, 0, 0])),
            ImageKind::CourseThumbnail,
        )
        .unwrap();
        let pixel = decode(&done.jpeg).to_rgb8().get_pixel(10, 10).0;
        assert!(pixel.iter().all(|c| *c > 245), "{pixel:?}");
    }

    #[test]
    fn what_the_bytes_are_decides_not_what_the_client_says() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        assert_eq!(
            process(svg, ImageKind::Avatar).unwrap_err(),
            ImageError::UnsupportedType
        );
        assert_eq!(
            process(b"GIF89a\x01\x00\x01\x00\x00\x00\x00;", ImageKind::Avatar).unwrap_err(),
            ImageError::UnsupportedType
        );
        assert_eq!(
            process(b"MZ\x90\x00 an executable", ImageKind::Avatar).unwrap_err(),
            ImageError::UnsupportedType
        );
        assert!(process(&[], ImageKind::Avatar).is_err());
    }

    #[test]
    fn damaged_and_truncated_files_are_refused_without_panicking() {
        let good = jpeg(800, 800);
        for cut in [3, 20] {
            assert!(
                process(&good[..cut], ImageKind::Avatar).is_err(),
                "cut at {cut}"
            );
        }
        // Decoders forgive a missing tail; what comes out is still a freshly encoded JPEG.
        for cut in [good.len() / 2, good.len() - 5] {
            if let Ok(done) = process(&good[..cut], ImageKind::Avatar) {
                assert_eq!(&done.jpeg[..2], [0xFF, 0xD8], "cut at {cut}");
            }
        }
        let mut garbled = good.clone();
        for byte in garbled.iter_mut().skip(100).step_by(7) {
            *byte = !*byte;
        }
        // Whether this decodes or not, it must not panic; if it decodes it is a fresh JPEG.
        if let Ok(done) = process(&garbled, ImageKind::Avatar) {
            assert_eq!(&done.jpeg[..2], [0xFF, 0xD8]);
        }
    }

    #[test]
    fn tiny_oversized_and_huge_images_are_refused() {
        assert_eq!(
            process(&png(100, 100, Rgba([1, 1, 1, 255])), ImageKind::Avatar).unwrap_err(),
            ImageError::TooSmall {
                width: 100,
                height: 100
            }
        );
        assert_eq!(
            process(
                &png(1000, 100, Rgba([1, 1, 1, 255])),
                ImageKind::CourseThumbnail
            )
            .unwrap_err(),
            ImageError::TooSmall {
                width: 1000,
                height: 100
            }
        );
        // A tiny file that claims a huge canvas (a decompression bomb) never gets decoded.
        assert_eq!(
            process(&png(5000, 5000, Rgba([0, 0, 0, 255])), ImageKind::Avatar).unwrap_err(),
            ImageError::Invalid
        );
        assert_eq!(
            process(&vec![0u8; MAX_IMAGE_BYTES + 1], ImageKind::Avatar).unwrap_err(),
            ImageError::TooLarge
        );
    }

    #[test]
    fn the_declared_type_is_only_a_quick_filter() {
        assert!(declared_type_is_acceptable(Some("image/png")));
        assert!(declared_type_is_acceptable(Some(
            "IMAGE/JPEG; charset=binary"
        )));
        assert!(declared_type_is_acceptable(Some("image/webp")));
        assert!(!declared_type_is_acceptable(Some("image/svg+xml")));
        assert!(!declared_type_is_acceptable(Some("image/gif")));
        assert!(!declared_type_is_acceptable(Some(
            "application/octet-stream"
        )));
        assert!(!declared_type_is_acceptable(None));
    }
}
