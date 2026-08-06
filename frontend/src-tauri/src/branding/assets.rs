//! The logo file: taking a copy the app owns, and reading it back for embedding.
//!
//! The user picks a logo from anywhere on disk. Referencing that path directly
//! would mean a deliverable exported six months later silently loses its logo
//! because the file moved, was renamed, or lived on a volume that is no longer
//! mounted. So the pick is copied into the app data directory once, and only that
//! copy is ever recorded or read.

use std::path::{Path, PathBuf};

use super::rules::{logo_data_uri, sniff_logo, LogoFormat, MAX_LOGO_BYTES};

/// The folder inside app data that holds the branding assets.
pub const BRANDING_DIR: &str = "branding";
/// The stem of the stored copy. One logo, overwritten on each pick, so a
/// long-lived install does not accumulate abandoned images.
pub const LOGO_STEM: &str = "logo";

/// The directory the branding assets live in, created if needed.
pub fn branding_dir(app_data_dir: &Path) -> Result<PathBuf, String> {
    let dir = app_data_dir.join(BRANDING_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {}", dir.display(), e))?;
    Ok(dir)
}

/// Reads, validates, and copies a picked logo into app data.
///
/// Validation is by content: the bytes have to actually be a PNG, JPEG, GIF,
/// WebP, or SVG, and small enough to inline into every exported document. The
/// stored file is named from the sniffed format, not from the original extension.
pub fn install_logo(app_data_dir: &Path, source: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(source)
        .map_err(|e| format!("Could not read {}: {}", source.display(), e))?;

    if bytes.is_empty() {
        return Err("That file is empty.".to_string());
    }
    if bytes.len() > MAX_LOGO_BYTES {
        return Err(format!(
            "That logo is {:.1} MB. Keep it under {} MB: it is embedded in every exported document.",
            bytes.len() as f64 / (1024.0 * 1024.0),
            MAX_LOGO_BYTES / (1024 * 1024)
        ));
    }
    let format = sniff_logo(&bytes)
        .ok_or_else(|| "That is not an image the app can embed. Use a PNG, JPEG, GIF, WebP, or SVG.".to_string())?;

    let dir = branding_dir(app_data_dir)?;

    // Remove any previous copy in another format so only one logo file survives.
    for candidate in [
        LogoFormat::Png,
        LogoFormat::Jpeg,
        LogoFormat::Gif,
        LogoFormat::Webp,
        LogoFormat::Svg,
    ] {
        let stale = dir.join(format!("{}.{}", LOGO_STEM, candidate.extension()));
        if stale.exists() && candidate != format {
            let _ = std::fs::remove_file(&stale);
        }
    }

    let destination = dir.join(format!("{}.{}", LOGO_STEM, format.extension()));
    std::fs::write(&destination, &bytes)
        .map_err(|e| format!("Could not save the logo to {}: {}", destination.display(), e))?;
    Ok(destination)
}

/// Deletes the stored copy. Failures are logged, not fatal: the row's path is
/// cleared either way, so the export stops using it.
pub fn remove_logo(app_data_dir: &Path) {
    let Ok(dir) = branding_dir(app_data_dir) else {
        return;
    };
    for candidate in [
        LogoFormat::Png,
        LogoFormat::Jpeg,
        LogoFormat::Gif,
        LogoFormat::Webp,
        LogoFormat::Svg,
    ] {
        let path = dir.join(format!("{}.{}", LOGO_STEM, candidate.extension()));
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("[Branding] could not remove {}: {}", path.display(), e);
            }
        }
    }
}

/// Reads a stored logo as a `data:` URI for inlining.
///
/// A missing or unreadable file returns None rather than an error: an export
/// should lose its logo, not fail, if the app data copy has been tampered with.
pub fn logo_as_data_uri(logo_path: &str) -> Option<String> {
    let path = Path::new(logo_path);
    if !path.exists() {
        log::warn!("[Branding] logo {} is missing; exporting without it", logo_path);
        return None;
    }
    match std::fs::read(path) {
        Ok(bytes) => {
            let uri = logo_data_uri(&bytes);
            if uri.is_none() {
                log::warn!(
                    "[Branding] logo {} is no longer a usable image; exporting without it",
                    logo_path
                );
            }
            uri
        }
        Err(e) => {
            log::warn!("[Branding] could not read logo {} ({}); exporting without it", logo_path, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02];

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn a_valid_logo_is_copied_into_app_data_under_its_real_format() {
        let app_data = temp_dir();
        let source_dir = temp_dir();
        // Deliberately the wrong extension: the copy must be named from content.
        let source = source_dir.path().join("company-mark.jpeg");
        std::fs::write(&source, PNG).unwrap();

        let stored = install_logo(app_data.path(), &source).unwrap();
        assert_eq!(stored.file_name().unwrap(), "logo.png");
        assert!(stored.starts_with(app_data.path().join(BRANDING_DIR)));
        assert_eq!(std::fs::read(&stored).unwrap(), PNG);
    }

    #[test]
    fn the_copy_survives_the_original_being_deleted() {
        let app_data = temp_dir();
        let source_dir = temp_dir();
        let source = source_dir.path().join("logo.png");
        std::fs::write(&source, PNG).unwrap();

        let stored = install_logo(app_data.path(), &source).unwrap();
        std::fs::remove_file(&source).unwrap();

        // The whole reason the copy exists.
        assert!(logo_as_data_uri(stored.to_str().unwrap()).is_some());
    }

    #[test]
    fn a_second_pick_in_a_different_format_replaces_the_first() {
        let app_data = temp_dir();
        let source_dir = temp_dir();

        let png_source = source_dir.path().join("a.png");
        std::fs::write(&png_source, PNG).unwrap();
        let png_stored = install_logo(app_data.path(), &png_source).unwrap();
        assert!(png_stored.exists());

        let svg_source = source_dir.path().join("b.svg");
        std::fs::write(&svg_source, b"<svg></svg>").unwrap();
        let svg_stored = install_logo(app_data.path(), &svg_source).unwrap();

        assert_eq!(svg_stored.file_name().unwrap(), "logo.svg");
        assert!(!png_stored.exists(), "the old format must not linger");
    }

    #[test]
    fn a_non_image_is_refused_with_a_readable_reason() {
        let app_data = temp_dir();
        let source_dir = temp_dir();
        let source = source_dir.path().join("notes.pdf");
        std::fs::write(&source, b"%PDF-1.7 fake").unwrap();

        let error = install_logo(app_data.path(), &source).unwrap_err();
        assert!(error.contains("not an image"), "was: {}", error);
    }

    #[test]
    fn an_empty_file_is_refused() {
        let app_data = temp_dir();
        let source_dir = temp_dir();
        let source = source_dir.path().join("empty.png");
        std::fs::write(&source, b"").unwrap();
        assert!(install_logo(app_data.path(), &source).unwrap_err().contains("empty"));
    }

    #[test]
    fn an_oversized_logo_is_refused_before_it_reaches_app_data() {
        let app_data = temp_dir();
        let source_dir = temp_dir();
        let source = source_dir.path().join("huge.png");
        let mut bytes = PNG.to_vec();
        bytes.resize(MAX_LOGO_BYTES + 10, 0);
        std::fs::write(&source, &bytes).unwrap();

        let error = install_logo(app_data.path(), &source).unwrap_err();
        assert!(error.contains("under 2 MB"), "was: {}", error);
        assert!(!app_data.path().join(BRANDING_DIR).join("logo.png").exists());
    }

    #[test]
    fn removing_the_logo_clears_every_stored_format() {
        let app_data = temp_dir();
        let dir = branding_dir(app_data.path()).unwrap();
        std::fs::write(dir.join("logo.png"), PNG).unwrap();
        std::fs::write(dir.join("logo.svg"), b"<svg></svg>").unwrap();

        remove_logo(app_data.path());
        assert!(!dir.join("logo.png").exists());
        assert!(!dir.join("logo.svg").exists());
    }

    #[test]
    fn a_missing_logo_path_yields_no_uri_rather_than_an_error() {
        assert_eq!(logo_as_data_uri("/nonexistent/logo.png"), None);
    }

    #[test]
    fn a_corrupted_stored_logo_yields_no_uri() {
        let app_data = temp_dir();
        let dir = branding_dir(app_data.path()).unwrap();
        let path = dir.join("logo.png");
        std::fs::write(&path, b"this is no longer a png").unwrap();
        assert_eq!(logo_as_data_uri(path.to_str().unwrap()), None);
    }

    #[test]
    fn a_missing_source_file_is_reported_not_panicked_on() {
        let app_data = temp_dir();
        let error = install_logo(app_data.path(), Path::new("/nonexistent/logo.png")).unwrap_err();
        assert!(error.contains("Could not read"));
    }
}
