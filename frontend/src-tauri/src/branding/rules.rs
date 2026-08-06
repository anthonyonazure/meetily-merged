//! Pure branding logic: what a valid accent colour is, how a logo becomes an
//! inline data URI, and the derived colours a document needs.
//!
//! No I/O, so every decision here is unit-testable, including the image sniffing
//! (which is deliberately magic-byte based rather than extension based: a file
//! the user renamed to `.png` should still be embedded with its real type).

use serde::{Deserialize, Serialize};

/// The default accent: the Ledger theme's ink. Chosen so an operator who turns
/// branding on without picking a colour gets something that looks deliberate.
pub const DEFAULT_ACCENT: &str = "#23252b";

/// Logos above this are refused: they are embedded inline in every exported
/// document, and a multi-megabyte data URI makes a print page crawl.
pub const MAX_LOGO_BYTES: usize = 2 * 1024 * 1024;

/// The branding actually in force for an export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Branding {
    pub firm_name: String,
    /// The copy inside the app data directory, never the file the user picked.
    pub logo_path: Option<String>,
    pub footer_text: String,
    pub accent_hex: String,
    pub include_logo: bool,
    pub include_footer: bool,
}

impl Default for Branding {
    fn default() -> Self {
        Self {
            firm_name: String::new(),
            logo_path: None,
            footer_text: String::new(),
            accent_hex: DEFAULT_ACCENT.to_string(),
            include_logo: true,
            include_footer: true,
        }
    }
}

impl Branding {
    /// Whether an export should be branded at all.
    ///
    /// The firm name is the switch. With no firm name configured the export paths
    /// render exactly as they did before this feature existed, which is what makes
    /// it safe to ship inert.
    pub fn is_configured(&self) -> bool {
        !self.firm_name.trim().is_empty()
    }

    /// The logo to embed, if there is one and it is wanted.
    pub fn logo_to_embed(&self) -> Option<&str> {
        if !self.include_logo {
            return None;
        }
        self.logo_path.as_deref().filter(|p| !p.trim().is_empty())
    }

    /// The footer line to print, if there is one and it is wanted.
    pub fn footer_to_print(&self) -> Option<&str> {
        if !self.include_footer {
            return None;
        }
        let text = self.footer_text.trim();
        (!text.is_empty()).then_some(text)
    }

    /// The accent, always a usable `#rrggbb`.
    pub fn accent(&self) -> String {
        normalize_hex(&self.accent_hex).unwrap_or_else(|| DEFAULT_ACCENT.to_string())
    }
}

/// Normalizes `#abc`, `abc`, `#AABBCC`, `aabbcc` to lowercase `#aabbcc`.
/// Returns None for anything that is not a hex colour, so a junk value falls back
/// to the default rather than being interpolated into a stylesheet.
pub fn normalize_hex(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('#').trim();
    if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match trimmed.len() {
        3 => {
            let mut out = String::with_capacity(7);
            out.push('#');
            for c in trimmed.chars() {
                out.push(c.to_ascii_lowercase());
                out.push(c.to_ascii_lowercase());
            }
            Some(out)
        }
        6 => Some(format!("#{}", trimmed.to_ascii_lowercase())),
        _ => None,
    }
}

/// Relative luminance of a `#rrggbb`, used to decide whether text on the accent
/// should be light or dark.
pub fn luminance(hex: &str) -> Option<f64> {
    let normalized = normalize_hex(hex)?;
    let bytes = normalized.trim_start_matches('#');
    let r = u8::from_str_radix(&bytes[0..2], 16).ok()? as f64 / 255.0;
    let g = u8::from_str_radix(&bytes[2..4], 16).ok()? as f64 / 255.0;
    let b = u8::from_str_radix(&bytes[4..6], 16).ok()? as f64 / 255.0;
    // The sRGB coefficients. Good enough for "is this dark?", which is all this
    // is asked.
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b)
}

/// A readable text colour to sit on the accent.
pub fn contrasting_ink(accent_hex: &str) -> &'static str {
    match luminance(accent_hex) {
        Some(l) if l > 0.55 => "#1a1c21",
        _ => "#ffffff",
    }
}

/// DOCX wants a bare `RRGGBB` with no hash.
pub fn docx_color(accent_hex: &str) -> String {
    normalize_hex(accent_hex)
        .unwrap_or_else(|| DEFAULT_ACCENT.to_string())
        .trim_start_matches('#')
        .to_string()
}

/// Image types a logo may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
}

impl LogoFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Svg => "svg",
        }
    }
}

/// Identifies a logo by its content, not its filename.
pub fn sniff_logo(bytes: &[u8]) -> Option<LogoFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(LogoFormat::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(LogoFormat::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(LogoFormat::Gif);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(LogoFormat::Webp);
    }
    // SVG is text; look past a BOM and any leading whitespace or XML prolog.
    let head_len = bytes.len().min(512);
    let head = String::from_utf8_lossy(&bytes[..head_len]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")) {
        return Some(LogoFormat::Svg);
    }
    None
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
///
/// Hand-rolled rather than pulled from a crate on purpose: `base64` is only in
/// the lock file transitively, and adding a direct dependency here would mean
/// editing Cargo.toml and the lock for thirty lines of arithmetic that can be
/// tested against the RFC 4648 vectors.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(BASE64_ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64_ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// A `data:` URI for an exported document. Self-contained by design: the HTML
/// export must not reference a local file that will not travel with it.
pub fn logo_data_uri(bytes: &[u8]) -> Option<String> {
    let format = sniff_logo(bytes)?;
    if bytes.len() > MAX_LOGO_BYTES {
        return None;
    }
    Some(format!(
        "data:{};base64,{}",
        format.mime(),
        base64_encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colours_normalize_to_lowercase_six_digit() {
        assert_eq!(normalize_hex("#AABBCC").as_deref(), Some("#aabbcc"));
        assert_eq!(normalize_hex("aabbcc").as_deref(), Some("#aabbcc"));
        assert_eq!(normalize_hex(" #ABC ").as_deref(), Some("#aabbcc"));
        assert_eq!(normalize_hex("f00").as_deref(), Some("#ff0000"));
    }

    #[test]
    fn junk_colours_are_rejected_not_interpolated() {
        assert_eq!(normalize_hex(""), None);
        assert_eq!(normalize_hex("red"), None);
        assert_eq!(normalize_hex("#12345"), None);
        assert_eq!(normalize_hex("#gggggg"), None);
        // The case that matters: a stylesheet injection attempt is not a colour.
        assert_eq!(normalize_hex("red; } body { display:none"), None);
    }

    #[test]
    fn a_junk_accent_falls_back_to_the_default() {
        let branding = Branding {
            accent_hex: "not a colour".to_string(),
            ..Branding::default()
        };
        assert_eq!(branding.accent(), DEFAULT_ACCENT);
    }

    #[test]
    fn dark_accents_get_light_ink_and_light_accents_get_dark_ink() {
        assert_eq!(contrasting_ink("#000000"), "#ffffff");
        assert_eq!(contrasting_ink("#23252b"), "#ffffff");
        assert_eq!(contrasting_ink("#ffffff"), "#1a1c21");
        assert_eq!(contrasting_ink("#ffe066"), "#1a1c21");
        // Unreadable input falls back to the safe pairing.
        assert_eq!(contrasting_ink("nonsense"), "#ffffff");
    }

    #[test]
    fn docx_colours_drop_the_hash() {
        assert_eq!(docx_color("#AABBCC"), "aabbcc");
        assert_eq!(docx_color("nonsense"), "23252b");
        // "bad" is three hex digits, so it really is the colour #bbaadd. Worth
        // pinning: it is the obvious "invalid" string to reach for in a test.
        assert_eq!(docx_color("bad"), "bbaadd");
    }

    #[test]
    fn branding_is_inert_until_a_firm_name_exists() {
        assert!(!Branding::default().is_configured());
        let named = Branding {
            firm_name: "Vortex MSP".to_string(),
            ..Branding::default()
        };
        assert!(named.is_configured());
        let blank = Branding {
            firm_name: "   ".to_string(),
            ..Branding::default()
        };
        assert!(!blank.is_configured());
    }

    #[test]
    fn the_include_switches_are_honoured() {
        let branding = Branding {
            firm_name: "Firm".to_string(),
            logo_path: Some("/tmp/logo.png".to_string()),
            footer_text: "Confidential".to_string(),
            include_logo: false,
            include_footer: false,
            ..Branding::default()
        };
        assert_eq!(branding.logo_to_embed(), None);
        assert_eq!(branding.footer_to_print(), None);

        let on = Branding {
            include_logo: true,
            include_footer: true,
            ..branding
        };
        assert_eq!(on.logo_to_embed(), Some("/tmp/logo.png"));
        assert_eq!(on.footer_to_print(), Some("Confidential"));
    }

    #[test]
    fn an_empty_footer_prints_nothing_even_when_enabled() {
        let branding = Branding {
            footer_text: "   ".to_string(),
            include_footer: true,
            ..Branding::default()
        };
        assert_eq!(branding.footer_to_print(), None);
    }

    #[test]
    fn base64_matches_the_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_high_bytes_and_the_full_alphabet() {
        assert_eq!(base64_encode(&[0xFF, 0xFF, 0xFF]), "////");
        assert_eq!(base64_encode(&[0x00, 0x00, 0x00]), "AAAA");
        assert_eq!(base64_encode(&[0xFB, 0xFF]), "+/8=");
    }

    #[test]
    fn logos_are_identified_by_content_not_filename() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(sniff_logo(&png), Some(LogoFormat::Png));
        assert_eq!(sniff_logo(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(LogoFormat::Jpeg));
        assert_eq!(sniff_logo(b"GIF89a...."), Some(LogoFormat::Gif));

        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        assert_eq!(sniff_logo(&webp), Some(LogoFormat::Webp));

        assert_eq!(sniff_logo(b"<svg xmlns=\"...\"></svg>"), Some(LogoFormat::Svg));
        assert_eq!(
            sniff_logo(b"<?xml version=\"1.0\"?><svg></svg>"),
            Some(LogoFormat::Svg)
        );
    }

    #[test]
    fn a_non_image_is_refused() {
        assert_eq!(sniff_logo(b""), None);
        assert_eq!(sniff_logo(b"not an image at all"), None);
        // A PDF renamed to logo.png must not be embedded as one.
        assert_eq!(sniff_logo(b"%PDF-1.7"), None);
    }

    #[test]
    fn a_data_uri_carries_the_sniffed_mime_type() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let uri = logo_data_uri(&png).unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(uri.ends_with(&base64_encode(&png)));
    }

    #[test]
    fn an_oversized_logo_is_refused_rather_than_inlined() {
        let mut huge = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        huge.resize(MAX_LOGO_BYTES + 1, 0);
        assert_eq!(logo_data_uri(&huge), None);
    }

    #[test]
    fn format_metadata_is_stable() {
        assert_eq!(LogoFormat::Png.mime(), "image/png");
        assert_eq!(LogoFormat::Png.extension(), "png");
        assert_eq!(LogoFormat::Svg.mime(), "image/svg+xml");
        assert_eq!(LogoFormat::Jpeg.extension(), "jpg");
    }
}
