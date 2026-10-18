//! HEIC and HEIF, through genzo-heif when the `heic` feature is on.

use crate::{DecodeError, DecodeOptions, DecodedImage, FileFormat, Preview};

#[cfg(feature = "heic")]
pub(crate) fn decode(data: &[u8], options: DecodeOptions) -> Result<DecodedImage, DecodeError> {
    use crate::Pixels;

    let img = genzo_heif::decode(
        data,
        genzo_heif::Options {
            threads: options.threads,
        },
    )
    .map_err(corrupt)?;
    let color = encoding(img.color);
    Ok(DecodedImage {
        format: FileFormat::Heif,
        width: img.width,
        height: img.height,
        source_bits: img.source_bits,
        has_alpha: img.has_alpha,
        pixels: Pixels::Rgba16 {
            data: img.rgba,
            color,
        },
        orientation: 1,
        exif: img.exif,
    })
}

/// libheif's color profile as a color encoding.
#[cfg(feature = "heic")]
fn encoding(profile: genzo_heif::ColorProfile) -> crate::ColorEncoding {
    use crate::ColorEncoding;
    use genzo_heif::ColorProfile;
    match profile {
        // HEIF: no profile means sRGB.
        ColorProfile::None => ColorEncoding::Srgb,
        ColorProfile::Icc(icc) => ColorEncoding::Icc(icc),
        // H.273: primaries 1 = BT.709/sRGB, 12 = P3 D65; transfer 13 = sRGB.
        ColorProfile::Nclx {
            primaries: 1,
            transfer: 13,
            ..
        } => ColorEncoding::Srgb,
        ColorProfile::Nclx {
            primaries: 12,
            transfer: 13,
            ..
        } => ColorEncoding::DisplayP3,
        ColorProfile::Nclx {
            primaries,
            transfer,
            matrix,
            full_range,
        } => ColorEncoding::Nclx {
            primaries,
            transfer,
            matrix,
            full_range,
        },
    }
}

#[cfg(feature = "heic")]
pub(crate) fn decode_preview(data: &[u8]) -> Result<Option<Preview>, DecodeError> {
    Ok(genzo_heif::decode_thumbnail(data)
        .map_err(corrupt)?
        .map(|t| Preview {
            width: t.width,
            height: t.height,
            rgba: t.rgba,
            // libheif applies the thumbnail item's own rotation and mirror,
            // as for the main image.
            orientation: 1,
            color: encoding(t.color),
        }))
}

#[cfg(feature = "heic")]
fn corrupt(e: genzo_heif::Error) -> DecodeError {
    DecodeError::Corrupt {
        format: FileFormat::Heif,
        message: e.message,
    }
}

#[cfg(not(feature = "heic"))]
pub(crate) fn decode(_: &[u8], _: DecodeOptions) -> Result<DecodedImage, DecodeError> {
    Err(DecodeError::NotBuiltIn(FileFormat::Heif, "heic"))
}

#[cfg(not(feature = "heic"))]
pub(crate) fn decode_preview(_: &[u8]) -> Result<Option<Preview>, DecodeError> {
    Err(DecodeError::NotBuiltIn(FileFormat::Heif, "heic"))
}

#[cfg(all(test, feature = "heic"))]
mod tests {
    use crate::{ColorEncoding, DecodeOptions, FileFormat, Pixels, decode, decode_preview};

    fn sample(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../heif/vendor/libheif/tests/data")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn heic_goes_through_the_common_api() {
        let img = decode(&sample("rainbow-451x461.heic"), DecodeOptions::default()).unwrap();
        assert_eq!(img.format, FileFormat::Heif);
        assert_eq!((img.width, img.height), (451, 461));
        assert_eq!(img.orientation, 1);
        let Pixels::Rgba16 { data, color } = &img.pixels else {
            panic!("expected RGBA, got {:?}", img.pixels);
        };
        assert_eq!(data.len(), 451 * 461 * 4);
        // This file carries an ICC profile (GIMP's built-in sRGB).
        let ColorEncoding::Icc(icc) = color else {
            panic!("expected an ICC profile, got {color:?}");
        };
        assert_eq!(&icc[36..40], b"acsp", "ICC signature");
    }

    #[test]
    fn missing_preview_is_not_an_error() {
        // The test file has no thumbnail; that is a normal case.
        assert!(decode_preview(&sample("rainbow-451x461.heic")).is_ok());
    }
}

#[cfg(all(test, not(feature = "heic")))]
mod tests {
    use crate::{DecodeError, DecodeOptions, decode};

    #[test]
    fn says_heic_is_not_built_in() {
        let err = decode(b"\0\0\0\x18ftypheic\0\0\0\0", DecodeOptions::default()).unwrap_err();
        assert!(matches!(err, DecodeError::NotBuiltIn(..)));
        assert!(err.to_string().contains("not included in this build"));
    }
}
