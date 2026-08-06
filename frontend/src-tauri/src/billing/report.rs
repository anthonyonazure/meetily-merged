//! Turning meetings into a billing report: the row shape, the date-range filter,
//! and the totals.
//!
//! The totals rule that runs through everything here: a row only adds to the
//! total if it is fully priced. Anything that could not be priced is counted and
//! named in `excluded`, so the report can never quietly be short.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::rules::{
    BillingComputation, BillingSettings, ClientBilling, MeetingBillingOverride, MinutesSource,
    RateSource, RowState,
};

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingRow {
    pub meeting_id: String,
    pub title: String,
    pub date: DateTime<Utc>,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    /// The length before the billing increment.
    pub minutes: i64,
    pub minutes_source: MinutesSource,
    /// The length after rounding and the minimum. 0 for non-billable rows.
    pub rounded_minutes: i64,
    /// None means no rate is configured. The UI shows "no rate set".
    pub rate: Option<f64>,
    pub rate_source: RateSource,
    /// None whenever the row could not be priced. Never 0.0 as a stand-in.
    pub amount: Option<f64>,
    pub state: RowState,
    pub billable: bool,
    pub note: String,
}

/// Why rows were left out of the total, with a count for each reason.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ExcludedCounts {
    /// Marked non-billable by the client or the meeting.
    pub not_billable: i64,
    /// Billable but with no rate configured anywhere.
    pub no_rate: i64,
    /// Billable with a rate, but no derivable length.
    pub no_length: i64,
}

impl ExcludedCounts {
    pub fn total(&self) -> i64 {
        self.not_billable + self.no_rate + self.no_length
    }

    /// A one-line English account of what was left out, or None when nothing was.
    pub fn describe(&self) -> Option<String> {
        if self.total() == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if self.no_rate > 0 {
            parts.push(format!(
                "{} with no rate set",
                self.no_rate
            ));
        }
        if self.no_length > 0 {
            parts.push(format!("{} with no recorded length", self.no_length));
        }
        if self.not_billable > 0 {
            parts.push(format!("{} marked non-billable", self.not_billable));
        }
        Some(format!(
            "{} meeting{} excluded from the total: {}",
            self.total(),
            if self.total() == 1 { "" } else { "s" },
            parts.join(", ")
        ))
    }
}

/// The whole report: the rows, the totals, and what was left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingReport {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub client_id: Option<String>,
    pub currency: String,
    pub rounding_minutes: i64,
    pub min_billable_minutes: i64,
    pub rows: Vec<BillingRow>,
    /// Meetings that produced a priced row.
    pub billable_meetings: i64,
    pub total_minutes: i64,
    pub total_rounded_minutes: i64,
    pub total_amount: f64,
    pub excluded: ExcludedCounts,
    /// Set when the workspace has no rate at all, so the UI can say what to fix.
    pub warning: Option<String>,
}

/// Inclusive day boundaries from `YYYY-MM-DD`, or a full RFC3339 instant taken
/// as given. Mirrors `consent::commands::parse_boundary` so a date typed into
/// one screen means the same thing in the other.
pub fn parse_boundary(value: &str, end_of_day: bool) -> Result<DateTime<Utc>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("A date is required".to_string());
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map_err(|_| format!("Could not read the date \"{}\". Use YYYY-MM-DD.", trimmed))?;
    let time = if end_of_day {
        date.and_hms_opt(23, 59, 59)
    } else {
        date.and_hms_opt(0, 0, 0)
    }
    .ok_or_else(|| "Invalid date".to_string())?;
    Ok(Utc.from_utc_datetime(&time))
}

/// Whether a meeting falls inside the requested window.
pub fn in_range(created_at: DateTime<Utc>, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
    created_at >= start && created_at <= end
}

/// Everything a single meeting contributes, before the DB is consulted for the
/// next one. Kept as its own struct so the row builder is a pure function.
#[derive(Debug, Clone)]
pub struct MeetingInput {
    pub meeting_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    pub raw_minutes: i64,
    pub raw_minutes_source: MinutesSource,
    pub client_billing: Option<ClientBilling>,
    pub meeting_override: MeetingBillingOverride,
    pub note: String,
}

fn to_row(input: &MeetingInput, computation: BillingComputation) -> BillingRow {
    BillingRow {
        meeting_id: input.meeting_id.clone(),
        title: input.title.clone(),
        date: input.created_at,
        client_id: input.client_id.clone(),
        client_name: input.client_name.clone(),
        minutes: computation.minutes,
        minutes_source: computation.minutes_source,
        rounded_minutes: computation.rounded_minutes,
        rate: computation.rate,
        rate_source: computation.rate_source,
        amount: computation.amount,
        state: computation.state,
        billable: computation.state != RowState::NotBillable,
        note: input.note.clone(),
    }
}

/// Assembles the report from already-gathered per-meeting inputs.
///
/// Pure on purpose: the command layer does the reads, this does the arithmetic,
/// and the tests exercise every combination without a database.
pub fn build(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    client_id: Option<String>,
    settings: &BillingSettings,
    inputs: &[MeetingInput],
) -> BillingReport {
    let mut rows = Vec::with_capacity(inputs.len());
    let mut excluded = ExcludedCounts::default();
    let mut billable_meetings = 0i64;
    let mut total_minutes = 0i64;
    let mut total_rounded_minutes = 0i64;
    let mut total_amount = 0.0f64;

    for input in inputs {
        // Untagged meetings are the firm's own time. They are only reported when
        // the operator has asked for them.
        if input.client_id.is_none() && !settings.include_internal {
            continue;
        }
        let computation = super::rules::compute(
            input.raw_minutes,
            input.raw_minutes_source,
            settings,
            input.client_billing,
            input.meeting_override,
        );
        let row = to_row(input, computation);

        match row.state {
            RowState::Billable => {
                billable_meetings += 1;
                total_minutes += row.minutes;
                total_rounded_minutes += row.rounded_minutes;
                total_amount += row.amount.unwrap_or(0.0);
            }
            RowState::NotBillable => excluded.not_billable += 1,
            RowState::NoRate => excluded.no_rate += 1,
            RowState::NoLength => excluded.no_length += 1,
        }
        rows.push(row);
    }

    let warning = if settings.default_hourly_rate.is_none() && excluded.no_rate > 0 {
        Some(
            "No workspace rate is set, so these meetings cannot be priced. Set one in Billing settings, or give the client its own rate."
                .to_string(),
        )
    } else {
        None
    };

    BillingReport {
        start,
        end,
        client_id,
        currency: settings.currency.clone(),
        rounding_minutes: settings.rounding_minutes,
        min_billable_minutes: settings.min_billable_minutes,
        rows,
        billable_meetings,
        total_minutes,
        total_rounded_minutes,
        // Sum of already-rounded lines, so the total matches the printed rows.
        total_amount: super::rules::to_cents(total_amount),
        excluded,
        warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(rate: Option<f64>, rounding: i64, include_internal: bool) -> BillingSettings {
        BillingSettings {
            default_hourly_rate: rate,
            currency: "USD".to_string(),
            rounding_minutes: rounding,
            min_billable_minutes: 0,
            include_internal,
        }
    }

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, day, 10, 0, 0).unwrap()
    }

    fn input(id: &str, client: Option<&str>, minutes: i64) -> MeetingInput {
        MeetingInput {
            meeting_id: id.to_string(),
            title: format!("Meeting {}", id),
            created_at: at(3),
            client_id: client.map(str::to_string),
            client_name: client.map(|c| format!("Client {}", c)),
            raw_minutes: minutes,
            raw_minutes_source: MinutesSource::Recorded,
            client_billing: None,
            meeting_override: MeetingBillingOverride::default(),
            note: String::new(),
        }
    }

    #[test]
    fn boundaries_are_inclusive_days_or_exact_instants() {
        let start = parse_boundary("2026-08-01", false).unwrap();
        let end = parse_boundary("2026-08-01", true).unwrap();
        assert_eq!(start.format("%H:%M:%S").to_string(), "00:00:00");
        assert_eq!(end.format("%H:%M:%S").to_string(), "23:59:59");
        assert!(end > start);

        let exact = parse_boundary("2026-08-01T14:30:00Z", true).unwrap();
        assert_eq!(exact.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn unreadable_boundaries_are_refused() {
        assert!(parse_boundary("08/01/2026", false).is_err());
        assert!(parse_boundary("", false).is_err());
        assert!(parse_boundary("   ", false).is_err());
    }

    #[test]
    fn range_membership_includes_both_ends() {
        let start = at(1);
        let end = at(5);
        assert!(in_range(at(1), start, end));
        assert!(in_range(at(3), start, end));
        assert!(in_range(at(5), start, end));
        assert!(!in_range(at(6), start, end));
    }

    #[test]
    fn a_simple_report_totals_its_billable_rows() {
        let report = build(
            at(1),
            at(9),
            None,
            &settings(Some(150.0), 0, false),
            &[input("a", Some("c1"), 60), input("b", Some("c1"), 30)],
        );
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.billable_meetings, 2);
        assert_eq!(report.total_minutes, 90);
        assert_eq!(report.total_rounded_minutes, 90);
        assert_eq!(report.total_amount, 225.0);
        assert_eq!(report.excluded.total(), 0);
        assert!(report.warning.is_none());
    }

    #[test]
    fn unpriceable_rows_are_shown_counted_and_kept_out_of_the_total() {
        let report = build(
            at(1),
            at(9),
            None,
            &settings(None, 0, false),
            &[input("a", Some("c1"), 60)],
        );
        assert_eq!(report.rows.len(), 1, "the row is still visible");
        assert_eq!(report.rows[0].amount, None);
        assert_eq!(report.rows[0].state, RowState::NoRate);
        assert_eq!(report.total_amount, 0.0);
        assert_eq!(report.billable_meetings, 0);
        assert_eq!(report.excluded.no_rate, 1);
        assert!(report.warning.is_some(), "the operator is told why");
    }

    #[test]
    fn a_client_rate_prices_a_row_even_with_no_workspace_rate() {
        let mut with_rate = input("a", Some("c1"), 60);
        with_rate.client_billing = Some(ClientBilling {
            hourly_rate: Some(200.0),
            billable: true,
        });
        let report = build(
            at(1),
            at(9),
            None,
            &settings(None, 0, false),
            &[with_rate, input("b", Some("c2"), 60)],
        );
        assert_eq!(report.total_amount, 200.0);
        assert_eq!(report.billable_meetings, 1);
        assert_eq!(report.excluded.no_rate, 1);
    }

    #[test]
    fn untagged_meetings_are_hidden_unless_asked_for() {
        let hidden = build(
            at(1),
            at(9),
            None,
            &settings(Some(150.0), 0, false),
            &[input("a", None, 60)],
        );
        assert!(hidden.rows.is_empty());

        let shown = build(
            at(1),
            at(9),
            None,
            &settings(Some(150.0), 0, true),
            &[input("a", None, 60)],
        );
        assert_eq!(shown.rows.len(), 1);
        assert_eq!(shown.total_amount, 150.0);
    }

    #[test]
    fn the_total_is_the_sum_of_rounded_lines_not_a_rounded_sum() {
        // Three 7-minute meetings at 150/h: each line is 17.50, total 52.50.
        // Summing unrounded (17.4999...) and rounding once would give 52.50 too,
        // but at 10 lines the two diverge, so the invariant is worth pinning.
        let rows: Vec<MeetingInput> = (0..10)
            .map(|i| input(&format!("m{}", i), Some("c1"), 7))
            .collect();
        let report = build(at(1), at(9), None, &settings(Some(150.0), 0, false), &rows);
        assert_eq!(report.total_amount, 175.0);
        for row in &report.rows {
            assert_eq!(row.amount, Some(17.5));
        }
    }

    #[test]
    fn rounding_applies_per_meeting_not_to_the_total() {
        // Four 20-minute meetings with 15-minute rounding bill 30 each, not 80
        // rounded up once. This is the behaviour an MSP actually invoices.
        let rows: Vec<MeetingInput> = (0..4)
            .map(|i| input(&format!("m{}", i), Some("c1"), 20))
            .collect();
        let report = build(at(1), at(9), None, &settings(Some(120.0), 15, false), &rows);
        assert_eq!(report.total_minutes, 80);
        assert_eq!(report.total_rounded_minutes, 120);
        assert_eq!(report.total_amount, 240.0);
    }

    #[test]
    fn non_billable_and_no_length_rows_are_counted_separately() {
        let mut non_billable = input("a", Some("c1"), 60);
        non_billable.client_billing = Some(ClientBilling {
            hourly_rate: None,
            billable: false,
        });
        let mut no_length = input("b", Some("c1"), 0);
        no_length.raw_minutes_source = MinutesSource::Unknown;

        let report = build(
            at(1),
            at(9),
            None,
            &settings(Some(150.0), 0, false),
            &[non_billable, no_length],
        );
        assert_eq!(report.excluded.not_billable, 1);
        assert_eq!(report.excluded.no_length, 1);
        assert_eq!(report.excluded.no_rate, 0);
        assert_eq!(report.total_amount, 0.0);
        assert!(report.warning.is_none(), "a rate exists, so no rate warning");
    }

    #[test]
    fn the_excluded_description_names_every_reason() {
        let counts = ExcludedCounts {
            not_billable: 2,
            no_rate: 1,
            no_length: 3,
        };
        let text = counts.describe().unwrap();
        assert!(text.starts_with("6 meetings excluded from the total:"));
        assert!(text.contains("1 with no rate set"));
        assert!(text.contains("3 with no recorded length"));
        assert!(text.contains("2 marked non-billable"));
        assert_eq!(ExcludedCounts::default().describe(), None);
    }

    #[test]
    fn the_excluded_description_is_singular_for_one() {
        let counts = ExcludedCounts {
            not_billable: 0,
            no_rate: 1,
            no_length: 0,
        };
        assert!(counts.describe().unwrap().starts_with("1 meeting excluded"));
    }
}
