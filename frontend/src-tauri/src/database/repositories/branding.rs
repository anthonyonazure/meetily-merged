//! Branding persistence: the single `branding` row.

use crate::database::models::BrandingRow;
use sqlx::SqlitePool;

pub struct BrandingRepository;

impl BrandingRepository {
    /// Reads the row. None only if the migration seed did not run; callers
    /// substitute defaults, which means "unbranded".
    pub async fn get(pool: &SqlitePool) -> Result<Option<BrandingRow>, sqlx::Error> {
        sqlx::query_as::<_, BrandingRow>(
            "SELECT firm_name, logo_path, footer_text, accent_hex, include_logo, include_footer
             FROM branding WHERE id = 1",
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn save(
        pool: &SqlitePool,
        firm_name: &str,
        logo_path: Option<&str>,
        footer_text: &str,
        accent_hex: &str,
        include_logo: bool,
        include_footer: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO branding
                 (id, firm_name, logo_path, footer_text, accent_hex, include_logo, include_footer)
             VALUES (1, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 firm_name = excluded.firm_name,
                 logo_path = excluded.logo_path,
                 footer_text = excluded.footer_text,
                 accent_hex = excluded.accent_hex,
                 include_logo = excluded.include_logo,
                 include_footer = excluded.include_footer",
        )
        .bind(firm_name)
        .bind(logo_path)
        .bind(footer_text)
        .bind(accent_hex)
        .bind(include_logo)
        .bind(include_footer)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Updates only the logo path, so picking a logo does not require the caller
    /// to round-trip every other field.
    pub async fn set_logo_path(
        pool: &SqlitePool,
        logo_path: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO branding (id, logo_path) VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET logo_path = excluded.logo_path",
        )
        .bind(logo_path)
        .execute(pool)
        .await?;
        Ok(())
    }
}
