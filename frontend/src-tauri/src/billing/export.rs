//! Billing exports: a CSV for whatever the firm's accounting system eats, and a
//! Markdown summary that can be pasted straight into an invoice.
//!
//! Both formats say "no rate set" where a rate is missing. Neither ever prints
//! 0.00 for a row that could not be priced, because a zero in a spreadsheet
//! column is indistinguishable from free work.

use super::report::BillingReport;
use super::rules::{format_minutes, format_money, RowState};

/// The literal shown wherever a row has no rate configured.
pub const NO_RATE_LABEL: &str = "no rate set";

/// RFC 4180 field escaping. A field is quoted when it contains a comma, a quote,
/// or a newline, and inner quotes are doubled.
pub fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_row(fields: &[String]) -> String {
    fields
        .iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",")
}

/// A machine-readable rate or amount cell: the number, or the honest label.
fn optional_number(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{:.2}", v),
        None => NO_RATE_LABEL.to_string(),
    }
}

/// The CSV export. One header row, one row per meeting, then a totals row.
pub fn to_csv(report: &BillingReport) -> String {
    let mut out = String::new();
    out.push_str(&csv_row(&[
        "Date".into(),
        "Meeting".into(),
        "Client".into(),
        "Minutes".into(),
        "Billed minutes".into(),
        "Length source".into(),
        "Rate".into(),
        "Currency".into(),
        "Amount".into(),
        "Status".into(),
        "Note".into(),
    ]));
    out.push('\n');

    for row in &report.rows {
        let amount = match row.state {
            RowState::Billable => optional_number(row.amount),
            RowState::NoRate => NO_RATE_LABEL.to_string(),
            RowState::NotBillable => "not billable".to_string(),
            RowState::NoLength => "no recorded length".to_string(),
        };
        out.push_str(&csv_row(&[
            row.date.format("%Y-%m-%d").to_string(),
            row.title.clone(),
            row.client_name.clone().unwrap_or_else(|| "Internal".into()),
            row.minutes.to_string(),
            row.rounded_minutes.to_string(),
            row.minutes_source.as_str().to_string(),
            optional_number(row.rate),
            report.currency.clone(),
            amount,
            row.state.as_str().to_string(),
            row.note.clone(),
        ]));
        out.push('\n');
    }

    out.push_str(&csv_row(&[
        "TOTAL".into(),
        format!("{} billable meeting(s)", report.billable_meetings),
        String::new(),
        report.total_minutes.to_string(),
        report.total_rounded_minutes.to_string(),
        String::new(),
        String::new(),
        report.currency.clone(),
        format!("{:.2}", report.total_amount),
        String::new(),
        report.excluded.describe().unwrap_or_default(),
    ]));
    out.push('\n');

    out
}

/// Escapes the pipes that would otherwise break a Markdown table cell.
fn md_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

/// The invoice-ready Markdown summary: a table of billable lines, the total, and
/// an explicit account of anything left out.
pub fn to_markdown(report: &BillingReport, firm_name: Option<&str>) -> String {
    let mut out = String::new();

    match firm_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(firm) => out.push_str(&format!("# {} — billable time\n\n", md_cell(firm))),
        None => out.push_str("# Billable time\n\n"),
    }

    out.push_str(&format!(
        "- Period: {} to {}\n",
        report.start.format("%Y-%m-%d"),
        report.end.format("%Y-%m-%d")
    ));
    if let Some(name) = report
        .rows
        .iter()
        .find_map(|r| r.client_name.as_deref())
        .filter(|_| report.client_id.is_some())
    {
        out.push_str(&format!("- Client: {}\n", md_cell(name)));
    }
    if report.rounding_minutes > 0 {
        out.push_str(&format!(
            "- Rounded up to the nearest {} minutes\n",
            report.rounding_minutes
        ));
    }
    if report.min_billable_minutes > 0 {
        out.push_str(&format!(
            "- Minimum billable increment: {} minutes\n",
            report.min_billable_minutes
        ));
    }
    out.push('\n');

    let billable: Vec<_> = report
        .rows
        .iter()
        .filter(|r| r.state == RowState::Billable)
        .collect();

    if billable.is_empty() {
        out.push_str("_No billable meetings in this period._\n\n");
    } else {
        out.push_str("| Date | Meeting | Client | Time | Rate | Amount |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for row in &billable {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                row.date.format("%Y-%m-%d"),
                md_cell(&row.title),
                md_cell(row.client_name.as_deref().unwrap_or("Internal")),
                format_minutes(row.rounded_minutes),
                row.rate
                    .map(|r| format_money(r, &report.currency))
                    .unwrap_or_else(|| NO_RATE_LABEL.to_string()),
                row.amount
                    .map(|a| format_money(a, &report.currency))
                    .unwrap_or_else(|| NO_RATE_LABEL.to_string()),
            ));
        }
        out.push_str(&format!(
            "| **Total** | **{} meeting{}** |  | **{}** |  | **{}** |\n\n",
            report.billable_meetings,
            if report.billable_meetings == 1 { "" } else { "s" },
            format_minutes(report.total_rounded_minutes),
            format_money(report.total_amount, &report.currency),
        ));
    }

    // Anything excluded is stated, and the meetings are named, so the total can
    // be checked rather than trusted.
    if let Some(summary) = report.excluded.describe() {
        out.push_str("## Not included in the total\n\n");
        out.push_str(&format!("{}\n\n", summary));
        for row in report.rows.iter().filter(|r| r.state != RowState::Billable) {
            let reason = match row.state {
                RowState::NoRate => NO_RATE_LABEL,
                RowState::NoLength => "no recorded length",
                RowState::NotBillable => "marked non-billable",
                RowState::Billable => continue,
            };
            out.push_str(&format!(
                "- {} — {} ({}){}\n",
                row.date.format("%Y-%m-%d"),
                md_cell(&row.title),
                reason,
                if row.note.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", md_cell(row.note.trim()))
                }
            ));
        }
        out.push('\n');
    }

    if let Some(warning) = &report.warning {
        out.push_str(&format!("> {}\n", warning));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::report::{build, MeetingInput};
    use crate::billing::rules::{
        BillingSettings, ClientBilling, MeetingBillingOverride, MinutesSource,
    };
    use chrono::{DateTime, TimeZone, Utc};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, day, 10, 0, 0).unwrap()
    }

    fn settings(rate: Option<f64>) -> BillingSettings {
        BillingSettings {
            default_hourly_rate: rate,
            currency: "USD".to_string(),
            rounding_minutes: 15,
            min_billable_minutes: 0,
            include_internal: false,
        }
    }

    fn input(id: &str, title: &str, minutes: i64) -> MeetingInput {
        MeetingInput {
            meeting_id: id.to_string(),
            title: title.to_string(),
            created_at: at(3),
            client_id: Some("c1".to_string()),
            client_name: Some("Acme".to_string()),
            raw_minutes: minutes,
            raw_minutes_source: MinutesSource::Recorded,
            client_billing: None,
            meeting_override: MeetingBillingOverride::default(),
            note: String::new(),
        }
    }

    #[test]
    fn csv_quotes_only_what_needs_quoting() {
        assert_eq!(csv_field("Acme"), "Acme");
        assert_eq!(csv_field("Acme, Inc"), "\"Acme, Inc\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn csv_has_a_header_a_row_per_meeting_and_a_total() {
        let report = build(
            at(1),
            at(9),
            None,
            &settings(Some(150.0)),
            &[input("a", "Q3 review", 50)],
        );
        let csv = to_csv(&report);
        let lines: Vec<&str> = csv.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Date,Meeting,Client,Minutes,Billed minutes"));
        assert!(lines[1].contains("Q3 review"));
        assert!(lines[1].contains("Acme"));
        assert!(lines[1].contains("150.00"));
        assert!(lines[1].contains("recorded"));
        assert!(lines[2].starts_with("TOTAL"));
        assert!(lines[2].contains("150.00"));
    }

    #[test]
    fn csv_says_no_rate_set_rather_than_zero() {
        let report = build(at(1), at(9), None, &settings(None), &[input("a", "Call", 50)]);
        let csv = to_csv(&report);
        assert!(csv.contains(NO_RATE_LABEL));
        // The critical assertion: no 0.00 on the unpriced line.
        let row_line = csv.lines().nth(1).unwrap();
        assert!(!row_line.contains("0.00"), "row was: {}", row_line);
    }

    #[test]
    fn csv_escapes_a_comma_in_a_meeting_title() {
        let report = build(
            at(1),
            at(9),
            None,
            &settings(Some(100.0)),
            &[input("a", "Review, then plan", 60)],
        );
        assert!(to_csv(&report).contains("\"Review, then plan\""));
    }

    #[test]
    fn markdown_leads_with_the_firm_name_when_branded() {
        let report = build(at(1), at(9), None, &settings(Some(150.0)), &[input("a", "Call", 60)]);
        assert!(to_markdown(&report, Some("Vortex MSP")).starts_with("# Vortex MSP — billable time"));
        assert!(to_markdown(&report, None).starts_with("# Billable time"));
        assert!(to_markdown(&report, Some("   ")).starts_with("# Billable time"));
    }

    #[test]
    fn markdown_states_the_rounding_rules_it_applied() {
        let mut s = settings(Some(150.0));
        s.min_billable_minutes = 30;
        let report = build(at(1), at(9), None, &s, &[input("a", "Call", 5)]);
        let md = to_markdown(&report, None);
        assert!(md.contains("Rounded up to the nearest 15 minutes"));
        assert!(md.contains("Minimum billable increment: 30 minutes"));
        // 5 minutes -> 15 by rounding -> 30 by the floor.
        assert!(md.contains("30m"));
    }

    #[test]
    fn markdown_lists_and_explains_every_excluded_meeting() {
        let mut non_billable = input("b", "Internal chat", 30);
        non_billable.client_billing = Some(ClientBilling {
            hourly_rate: None,
            billable: false,
        });
        non_billable.note = "goodwill".to_string();
        let report = build(
            at(1),
            at(9),
            None,
            &settings(Some(150.0)),
            &[input("a", "Billable call", 60), non_billable],
        );
        let md = to_markdown(&report, None);
        assert!(md.contains("## Not included in the total"));
        assert!(md.contains("1 meeting excluded from the total"));
        assert!(md.contains("Internal chat"));
        assert!(md.contains("marked non-billable"));
        assert!(md.contains(": goodwill"));
    }

    #[test]
    fn markdown_with_nothing_billable_says_so_instead_of_an_empty_table() {
        let report = build(at(1), at(9), None, &settings(None), &[input("a", "Call", 60)]);
        let md = to_markdown(&report, None);
        assert!(md.contains("_No billable meetings in this period._"));
        assert!(md.contains(NO_RATE_LABEL));
        assert!(md.contains("> No workspace rate is set"));
    }

    #[test]
    fn markdown_escapes_pipes_that_would_break_the_table() {
        let report = build(
            at(1),
            at(9),
            None,
            &settings(Some(100.0)),
            &[input("a", "Plan | Review", 60)],
        );
        assert!(to_markdown(&report, None).contains("Plan \\| Review"));
    }
}
