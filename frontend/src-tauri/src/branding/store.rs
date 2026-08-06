//! Reading the branding row.
//!
//! Split out of `mod.rs` so the scratch compile harness can mount the real file
//! rather than a copy of these two functions.

use sqlx::SqlitePool;

use super::rules::Branding;
use crate::database::repositories::branding::BrandingRepository;

/// The branding in force. A missing row or a read failure yields the default,
/// which is "unbranded" — the safe answer, because it leaves exports exactly as
/// they were.
pub async fn load(pool: &SqlitePool) -> Branding {
    match BrandingRepository::get(pool).await {
        Ok(Some(row)) => Branding {
            firm_name: row.firm_name,
            logo_path: row.logo_path.filter(|p| !p.trim().is_empty()),
            footer_text: row.footer_text,
            accent_hex: row.accent_hex,
            include_logo: row.include_logo,
            include_footer: row.include_footer,
        },
        Ok(None) => Branding::default(),
        Err(e) => {
            log::warn!("[Branding] could not read branding ({}); exporting unbranded", e);
            Branding::default()
        }
    }
}

/// The branding to apply to an export, or None when nothing is configured.
///
/// Every export path calls this rather than `load`, so "no firm name means no
/// branding" is decided in one place instead of at each call site.
pub async fn for_export(pool: &SqlitePool) -> Option<Branding> {
    let branding = load(pool).await;
    branding.is_configured().then_some(branding)
}
