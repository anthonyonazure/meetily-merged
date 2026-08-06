//! Tauri command surface for billable time and meeting cost.
//!
//! Validation here is where money mistakes are caught: a rate has to be a
//! positive, finite, plausible number, and clearing a rate is a separate,
//! explicit act (send null) from setting one. Nothing in this file ever
//! substitutes a default rate for a missing one.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::database::repositories::billing::{
    BillingReportRepository, BillingSettingsRepository, ClientBillingRepository,
    MeetingBillingOverridesRepository,
};
use crate::state::AppState;

use super::duration::{self, AttendeeSource};
use super::export;
use super::report::{self, BillingReport, MeetingInput};
use super::rules::{
    self, BillingSettings, ClientBilling, MeetingBillingOverride, MeetingCostEstimate,
    MinutesSource, RateSource, RowState, MAX_HOURLY_RATE, MAX_MINUTES_OVERRIDE,
    MAX_ROUNDING_MINUTES,
};

/// Trim and bound any operator-supplied string before it reaches the database.
fn bounded(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

/// A rate has to be a positive, finite, plausible number. `None` is a valid
/// answer meaning "no rate"; a zero is not, because a stored zero would look
/// exactly like free work to every reader downstream.
fn validate_rate(rate: Option<f64>) -> Result<Option<f64>, String> {
    match rate {
        None => Ok(None),
        Some(value) if !value.is_finite() => Err("That rate is not a number.".to_string()),
        Some(value) if value <= 0.0 => Err(
            "An hourly rate has to be greater than zero. Leave it empty if there is no rate yet."
                .to_string(),
        ),
        Some(value) if value > MAX_HOURLY_RATE => Err(format!(
            "That rate is above {:.0} an hour. Check for an extra digit.",
            MAX_HOURLY_RATE
        )),
        Some(value) => Ok(Some(rules::to_cents(value))),
    }
}

fn validate_increment(minutes: i64, label: &str) -> Result<i64, String> {
    if minutes < 0 {
        return Err(format!("{} cannot be negative.", label));
    }
    if minutes > MAX_ROUNDING_MINUTES {
        return Err(format!(
            "{} is capped at {} minutes.",
            label, MAX_ROUNDING_MINUTES
        ));
    }
    Ok(minutes)
}

/// Currency codes are three letters. Anything else is refused rather than stored,
/// because it ends up printed next to a number on an invoice.
fn validate_currency(currency: &str) -> Result<String, String> {
    let code = currency.trim().to_uppercase();
    if code.is_empty() {
        return Ok("USD".to_string());
    }
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err("Use a three-letter currency code, like USD or EUR.".to_string());
    }
    Ok(code)
}

// ---------------------------------------------------------------------------
// Workspace settings
// ---------------------------------------------------------------------------

/// Loads the workspace billing settings, substituting defaults if the seed row is
/// somehow missing. A missing row yields no rate, which is the safe answer.
pub async fn load_settings(pool: &sqlx::SqlitePool) -> BillingSettings {
    match BillingSettingsRepository::get(pool).await {
        Ok(Some(row)) => BillingSettings {
            default_hourly_rate: rules::sanitize_rate(row.default_hourly_rate),
            currency: if row.currency.trim().is_empty() {
                "USD".to_string()
            } else {
                row.currency
            },
            rounding_minutes: row.rounding_minutes.max(0),
            min_billable_minutes: row.min_billable_minutes.max(0),
            include_internal: row.include_internal,
        },
        Ok(None) => BillingSettings::default(),
        Err(e) => {
            log::warn!(
                "[Billing] could not read billing settings ({}); using defaults with no rate",
                e
            );
            BillingSettings::default()
        }
    }
}

#[tauri::command]
pub async fn billing_settings_get(state: State<'_, AppState>) -> Result<BillingSettings, String> {
    Ok(load_settings(state.db_manager.pool()).await)
}

#[derive(Debug, Deserialize)]
pub struct BillingSettingsInput {
    /// Null clears the workspace rate. A number sets it.
    #[serde(default)]
    pub default_hourly_rate: Option<f64>,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub rounding_minutes: i64,
    #[serde(default)]
    pub min_billable_minutes: i64,
    #[serde(default)]
    pub include_internal: bool,
}

#[tauri::command]
pub async fn billing_settings_set(
    state: State<'_, AppState>,
    input: BillingSettingsInput,
) -> Result<BillingSettings, String> {
    let pool = state.db_manager.pool();
    let rate = validate_rate(input.default_hourly_rate)?;
    let currency = validate_currency(&input.currency)?;
    let rounding = validate_increment(input.rounding_minutes, "The rounding increment")?;
    let minimum = validate_increment(input.min_billable_minutes, "The minimum billable increment")?;

    BillingSettingsRepository::save(
        pool,
        rate,
        &currency,
        rounding,
        minimum,
        input.include_internal,
    )
    .await
    .map_err(|e| format!("Failed to save billing settings: {}", e))?;
    Ok(load_settings(pool).await)
}

// ---------------------------------------------------------------------------
// Per-client rates
// ---------------------------------------------------------------------------

/// A client's billing configuration, with the workspace fallback spelled out so
/// the UI can show "inherits $150/h" without a second call.
#[derive(Debug, Serialize)]
pub struct ClientBillingView {
    pub client_id: String,
    /// Null means this client has no rate of its own.
    pub hourly_rate: Option<f64>,
    pub billable: bool,
    /// The rate actually in force, and where it came from.
    pub effective_rate: Option<f64>,
    pub effective_rate_source: RateSource,
    pub currency: String,
}

async fn client_billing_view(
    pool: &sqlx::SqlitePool,
    client_id: &str,
) -> Result<ClientBillingView, String> {
    let settings = load_settings(pool).await;
    let row = ClientBillingRepository::get(pool, client_id)
        .await
        .map_err(|e| format!("Failed to read the client's billing: {}", e))?;
    let client_rate = row.as_ref().and_then(|r| r.hourly_rate);
    let billable = row.as_ref().map(|r| r.billable).unwrap_or(true);
    let (effective_rate, effective_rate_source) =
        rules::resolve_rate(client_rate, settings.default_hourly_rate);

    Ok(ClientBillingView {
        client_id: client_id.to_string(),
        hourly_rate: rules::sanitize_rate(client_rate),
        billable,
        effective_rate,
        effective_rate_source,
        currency: settings.currency,
    })
}

#[tauri::command]
pub async fn client_billing_get(
    state: State<'_, AppState>,
    client_id: String,
) -> Result<ClientBillingView, String> {
    client_billing_view(state.db_manager.pool(), &client_id).await
}

#[tauri::command]
pub async fn client_billing_set(
    state: State<'_, AppState>,
    client_id: String,
    hourly_rate: Option<f64>,
    billable: Option<bool>,
) -> Result<ClientBillingView, String> {
    let pool = state.db_manager.pool();
    let rate = validate_rate(hourly_rate)?;
    ClientBillingRepository::set(pool, &client_id, rate, billable.unwrap_or(true))
        .await
        .map_err(|e| format!("Failed to save the client's billing: {}", e))?;
    client_billing_view(pool, &client_id).await
}

// ---------------------------------------------------------------------------
// Per-meeting overrides and the meeting chip
// ---------------------------------------------------------------------------

/// One meeting's billing picture: what it is worth, how its length was
/// established, and the separate internal-cost estimate.
#[derive(Debug, Serialize)]
pub struct MeetingBillingView {
    pub meeting_id: String,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    pub minutes: i64,
    pub minutes_source: MinutesSource,
    pub rounded_minutes: i64,
    pub rate: Option<f64>,
    pub rate_source: RateSource,
    /// Null whenever the meeting cannot be priced.
    pub amount: Option<f64>,
    pub state: RowState,
    pub currency: String,
    /// The stored override, so the editor opens on what is actually set.
    pub billable_override: Option<bool>,
    pub minutes_override: Option<i64>,
    pub note: String,
    /// The internal-cost estimate. Null when attendees or the workspace rate are
    /// unknown; never a guess.
    pub cost_estimate: Option<MeetingCostEstimate>,
    pub attendee_source: AttendeeSource,
}

async fn meeting_billing_view(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<MeetingBillingView, String> {
    let settings = load_settings(pool).await;

    let client = crate::database::repositories::client::MeetingClientsRepository::client_for_meeting(
        pool, meeting_id,
    )
    .await
    .map_err(|e| format!("Failed to read the meeting's client: {}", e))?;

    let client_billing = match client.as_ref() {
        Some(client) => ClientBillingRepository::get(pool, &client.id)
            .await
            .map_err(|e| format!("Failed to read the client's billing: {}", e))?
            .map(|row| ClientBilling {
                hourly_rate: row.hourly_rate,
                billable: row.billable,
            }),
        None => None,
    };

    let override_row = MeetingBillingOverridesRepository::get(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to read the meeting's billing override: {}", e))?;
    let meeting_override = MeetingBillingOverride {
        billable: override_row.as_ref().and_then(|r| r.billable),
        minutes_override: override_row.as_ref().and_then(|r| r.minutes_override),
    };

    let derived = duration::minutes_for_meeting(pool, meeting_id).await;
    let computation = rules::compute(
        derived.minutes,
        derived.source,
        &settings,
        client_billing,
        meeting_override,
    );

    let (attendees, attendee_source) = duration::attendee_count(pool, meeting_id).await;
    let cost_estimate = rules::estimate_meeting_cost(
        computation.minutes,
        attendees,
        settings.default_hourly_rate,
    );

    Ok(MeetingBillingView {
        meeting_id: meeting_id.to_string(),
        client_id: client.as_ref().map(|c| c.id.clone()),
        client_name: client.as_ref().map(|c| c.name.clone()),
        minutes: computation.minutes,
        minutes_source: computation.minutes_source,
        rounded_minutes: computation.rounded_minutes,
        rate: computation.rate,
        rate_source: computation.rate_source,
        amount: computation.amount,
        state: computation.state,
        currency: settings.currency,
        billable_override: meeting_override.billable,
        minutes_override: meeting_override.minutes_override,
        note: override_row.map(|r| r.note).unwrap_or_default(),
        cost_estimate,
        attendee_source,
    })
}

#[tauri::command]
pub async fn meeting_billing_get(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingBillingView, String> {
    meeting_billing_view(state.db_manager.pool(), &meeting_id).await
}

/// Marks a meeting billable or not, and/or replaces its billed minutes.
///
/// Passing null for both fields and an empty note removes the override entirely,
/// which is how the meeting goes back to inheriting.
#[tauri::command]
pub async fn meeting_billing_override_set(
    state: State<'_, AppState>,
    meeting_id: String,
    billable: Option<bool>,
    minutes_override: Option<i64>,
    note: Option<String>,
) -> Result<MeetingBillingView, String> {
    let pool = state.db_manager.pool();
    let note = bounded(&note.unwrap_or_default(), 500);

    if let Some(minutes) = minutes_override {
        if minutes < 0 {
            return Err("Billed minutes cannot be negative.".to_string());
        }
        if minutes > MAX_MINUTES_OVERRIDE {
            return Err("That is more billed time than a month of meetings.".to_string());
        }
    }

    if billable.is_none() && minutes_override.is_none() && note.is_empty() {
        MeetingBillingOverridesRepository::clear(pool, &meeting_id)
            .await
            .map_err(|e| format!("Failed to clear the meeting's billing override: {}", e))?;
    } else {
        MeetingBillingOverridesRepository::set(pool, &meeting_id, billable, minutes_override, &note)
            .await
            .map_err(|e| format!("Failed to save the meeting's billing override: {}", e))?;
    }

    meeting_billing_view(pool, &meeting_id).await
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

async fn build_report(
    pool: &sqlx::SqlitePool,
    from: &str,
    to: &str,
    client_id: Option<String>,
) -> Result<BillingReport, String> {
    let start = report::parse_boundary(from, false)?;
    let end = report::parse_boundary(to, true)?;
    if end < start {
        return Err("The end of the range is before its start".to_string());
    }

    let settings = load_settings(pool).await;

    let meetings = BillingReportRepository::all_meetings_with_client(pool, client_id.as_deref())
        .await
        .map_err(|e| format!("Failed to list meetings: {}", e))?;

    // Two bulk reads instead of two queries per meeting.
    let client_rates: std::collections::HashMap<String, ClientBilling> =
        ClientBillingRepository::list(pool)
            .await
            .map_err(|e| format!("Failed to read client rates: {}", e))?
            .into_iter()
            .map(|row| {
                (
                    row.client_id,
                    ClientBilling {
                        hourly_rate: row.hourly_rate,
                        billable: row.billable,
                    },
                )
            })
            .collect();

    let overrides: std::collections::HashMap<String, (MeetingBillingOverride, String)> =
        MeetingBillingOverridesRepository::all(pool)
            .await
            .map_err(|e| format!("Failed to read meeting overrides: {}", e))?
            .into_iter()
            .map(|row| {
                (
                    row.meeting_id,
                    (
                        MeetingBillingOverride {
                            billable: row.billable,
                            minutes_override: row.minutes_override,
                        },
                        row.note,
                    ),
                )
            })
            .collect();

    let mut inputs = Vec::new();
    for meeting in meetings {
        let created_at = meeting.created_at.0;
        if !report::in_range(created_at, start, end) {
            continue;
        }
        // Skip the length query for meetings the report will not show anyway.
        if meeting.client_id.is_none() && !settings.include_internal {
            continue;
        }
        let derived = duration::minutes_for_meeting(pool, &meeting.id).await;
        let (meeting_override, note) = overrides
            .get(&meeting.id)
            .cloned()
            .unwrap_or((MeetingBillingOverride::default(), String::new()));

        inputs.push(MeetingInput {
            meeting_id: meeting.id,
            title: meeting.title,
            created_at,
            client_billing: meeting
                .client_id
                .as_ref()
                .and_then(|id| client_rates.get(id).copied()),
            client_id: meeting.client_id,
            client_name: meeting.client_name,
            raw_minutes: derived.minutes,
            raw_minutes_source: derived.source,
            meeting_override,
            note,
        });
    }

    Ok(report::build(start, end, client_id, &settings, &inputs))
}

/// The billable-time report for a date range, optionally for one client.
#[tauri::command]
pub async fn billing_report(
    state: State<'_, AppState>,
    from: String,
    to: String,
    client_id: Option<String>,
) -> Result<BillingReport, String> {
    let client_id = client_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty());
    build_report(state.db_manager.pool(), &from, &to, client_id).await
}

#[derive(Debug, Serialize)]
pub struct BillingExportResult {
    pub folder: String,
    pub csv_path: String,
    pub markdown_path: String,
    pub rows: usize,
    pub billable_meetings: i64,
    pub excluded: i64,
}

/// Writes the report as both a CSV and an invoice-ready Markdown summary into a
/// folder the user picks. Same shape as the consent log export, deliberately: two
/// files, one machine-readable and one for a person.
#[tauri::command]
pub async fn billing_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    from: String,
    to: String,
    client_id: Option<String>,
) -> Result<BillingExportResult, String> {
    let client_id = client_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty());
    let pool = state.db_manager.pool();
    let report = build_report(pool, &from, &to, client_id).await?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let folder = rx
        .await
        .map_err(|_| "Folder picker closed unexpectedly".to_string())?
        .ok_or_else(|| "cancelled".to_string())?
        .into_path()
        .map_err(|e| format!("Invalid destination folder: {}", e))?;

    let firm_name = crate::branding::load(pool).await.firm_name;
    let stem = format!(
        "billable-time-{}-to-{}",
        report.start.format("%Y-%m-%d"),
        report.end.format("%Y-%m-%d")
    );
    let csv_path = folder.join(format!("{}.csv", stem));
    let markdown_path = folder.join(format!("{}.md", stem));

    std::fs::write(&csv_path, export::to_csv(&report))
        .map_err(|e| format!("Failed to write {}: {}", csv_path.display(), e))?;
    std::fs::write(
        &markdown_path,
        export::to_markdown(&report, Some(firm_name.as_str())),
    )
    .map_err(|e| format!("Failed to write {}: {}", markdown_path.display(), e))?;

    log::info!(
        "[Billing] exported {} row(s), {} billable, to {}",
        report.rows.len(),
        report.billable_meetings,
        folder.display()
    );

    Ok(BillingExportResult {
        folder: folder.to_string_lossy().to_string(),
        csv_path: csv_path.to_string_lossy().to_string(),
        markdown_path: markdown_path.to_string_lossy().to_string(),
        rows: report.rows.len(),
        billable_meetings: report.billable_meetings,
        excluded: report.excluded.total(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_rate_is_refused_rather_than_stored() {
        assert!(validate_rate(Some(0.0)).is_err());
        assert!(validate_rate(Some(-1.0)).is_err());
        assert!(validate_rate(Some(f64::NAN)).is_err());
        assert!(validate_rate(Some(MAX_HOURLY_RATE + 1.0)).is_err());
    }

    #[test]
    fn clearing_a_rate_is_a_valid_answer() {
        assert_eq!(validate_rate(None).unwrap(), None);
    }

    #[test]
    fn a_rate_is_stored_to_the_cent() {
        assert_eq!(validate_rate(Some(150.126)).unwrap(), Some(150.13));
        assert_eq!(validate_rate(Some(150.124)).unwrap(), Some(150.12));
        assert_eq!(validate_rate(Some(150.0)).unwrap(), Some(150.0));
        // Exact halves land wherever the binary representation of the input puts
        // them, which is a property of f64 rather than of this code. Both of these
        // are within a cent of the typed value, which is the guarantee that matters.
        assert!(matches!(validate_rate(Some(150.005)).unwrap(), Some(v) if (v - 150.0).abs() <= 0.01));
    }

    #[test]
    fn increments_are_bounded_and_non_negative() {
        assert!(validate_increment(-1, "x").is_err());
        assert!(validate_increment(MAX_ROUNDING_MINUTES + 1, "x").is_err());
        assert_eq!(validate_increment(15, "x").unwrap(), 15);
        assert_eq!(validate_increment(0, "x").unwrap(), 0);
    }

    #[test]
    fn currency_codes_are_three_letters_or_refused() {
        assert_eq!(validate_currency("usd").unwrap(), "USD");
        assert_eq!(validate_currency("  eur ").unwrap(), "EUR");
        assert_eq!(validate_currency("").unwrap(), "USD");
        assert!(validate_currency("dollars").is_err());
        assert!(validate_currency("US$").is_err());
        assert!(validate_currency("US").is_err());
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(bounded("  hi  ", 500), "hi");
        assert_eq!(bounded(&"x".repeat(600), 500).chars().count(), 500);
    }
}
