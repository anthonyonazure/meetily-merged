//! Pure billing arithmetic: rate resolution, rounding, minimum increments, and
//! the amount for one meeting.
//!
//! Everything here is deliberately I/O free so the money logic can be tested
//! exhaustively. The one rule that shapes every signature: a missing rate is a
//! distinct outcome, never a zero. `Option<f64>` goes all the way through, and
//! rows that cannot be priced come back labelled rather than silently valued at
//! nothing.

use serde::{Deserialize, Serialize};

/// Rounding increments above this are almost certainly a typo (8 hours).
pub const MAX_ROUNDING_MINUTES: i64 = 480;
/// Nobody bills a single meeting for more than a working month.
pub const MAX_MINUTES_OVERRIDE: i64 = 60 * 24 * 30;
/// A sanity ceiling on hourly rates, to catch a stray extra digit.
pub const MAX_HOURLY_RATE: f64 = 100_000.0;

/// Workspace-level billing configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingSettings {
    /// None means no workspace rate has been configured. Never 0.
    pub default_hourly_rate: Option<f64>,
    pub currency: String,
    /// 0 means no rounding; otherwise minutes round up to a multiple of this.
    pub rounding_minutes: i64,
    /// 0 means no floor; otherwise a billable meeting bills at least this many.
    pub min_billable_minutes: i64,
    /// Whether meetings with no client tag appear in the report at all.
    pub include_internal: bool,
}

impl Default for BillingSettings {
    fn default() -> Self {
        Self {
            default_hourly_rate: None,
            currency: "USD".to_string(),
            rounding_minutes: 0,
            min_billable_minutes: 0,
            include_internal: false,
        }
    }
}

/// Per-client billing configuration. Absent for clients nobody has configured.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClientBilling {
    /// None means "use the workspace rate".
    pub hourly_rate: Option<f64>,
    pub billable: bool,
}

impl Default for ClientBilling {
    fn default() -> Self {
        Self {
            hourly_rate: None,
            billable: true,
        }
    }
}

/// Per-meeting corrections. `None` on either field means "inherit".
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct MeetingBillingOverride {
    pub billable: Option<bool>,
    pub minutes_override: Option<i64>,
}

/// Where a row's rate came from, so the UI can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateSource {
    Client,
    Workspace,
    /// No rate anywhere. The row shows "no rate set" and stays out of totals.
    None,
}

/// How a meeting's length was established. Reported per row because the three
/// answers deserve different amounts of trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinutesSource {
    /// `minutes_override` on the meeting. The operator's word wins.
    Override,
    /// The recording's own length, from the last transcript segment's
    /// recording-relative end time. This is the real answer.
    Recorded,
    /// First-to-last wall-clock transcript timestamp. Covers speech only, so it
    /// can under-report a meeting that opened or closed in silence.
    TranscriptSpan,
    /// The sum of the segments' own durations: speech time, not meeting time.
    /// The weakest of the three and always an under-report.
    SpeechTime,
    /// No transcript at all, so no length can be derived.
    Unknown,
}

impl MinutesSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::Recorded => "recorded",
            Self::TranscriptSpan => "transcript_span",
            Self::SpeechTime => "speech_time",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a row is or is not part of the total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    /// Priced and counted.
    Billable,
    /// Explicitly marked non-billable, by the client or by this meeting.
    NotBillable,
    /// Billable, but no rate is configured. Counted as excluded, shown as
    /// "no rate set" — never as 0.00.
    NoRate,
    /// Billable with a rate, but the meeting has no derivable length.
    NoLength,
}

impl RowState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Billable => "billable",
            Self::NotBillable => "not_billable",
            Self::NoRate => "no_rate",
            Self::NoLength => "no_length",
        }
    }

    /// Only fully-priced rows contribute to the invoice total.
    pub fn counts_toward_total(self) -> bool {
        matches!(self, Self::Billable)
    }
}

/// A stored rate is only a rate if it is a positive, finite number. A zero or
/// negative value in the column is treated as "unset" rather than as free work,
/// so a bad write can never quietly price a meeting at nothing.
pub fn sanitize_rate(rate: Option<f64>) -> Option<f64> {
    rate.filter(|r| r.is_finite() && *r > 0.0 && *r <= MAX_HOURLY_RATE)
}

/// The rate for a meeting: the client's if it has one, else the workspace's,
/// else nothing.
pub fn resolve_rate(
    client_rate: Option<f64>,
    workspace_rate: Option<f64>,
) -> (Option<f64>, RateSource) {
    if let Some(rate) = sanitize_rate(client_rate) {
        return (Some(rate), RateSource::Client);
    }
    if let Some(rate) = sanitize_rate(workspace_rate) {
        return (Some(rate), RateSource::Workspace);
    }
    (None, RateSource::None)
}

/// Whether a meeting is billable: the meeting's own answer if it gave one, else
/// the client's, else yes.
pub fn resolve_billable(
    meeting_override: Option<bool>,
    client_billable: Option<bool>,
) -> bool {
    meeting_override.or(client_billable).unwrap_or(true)
}

/// Seconds of recorded audio to whole minutes, rounding up any part-minute.
///
/// Rounding up here is not the billing increment — it is just "a 90-second call
/// is 2 minutes of wall time, not 1". The billing increment is applied
/// afterwards by [`round_minutes`].
pub fn seconds_to_minutes(seconds: f64) -> i64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds / 60.0).ceil() as i64
}

/// Applies the billing increment and the minimum.
///
/// A meeting with no measurable length stays at zero: a 0-minute row is a data
/// problem, and quietly turning it into the 15-minute minimum would invent
/// billable time out of nothing.
pub fn round_minutes(actual_minutes: i64, rounding_minutes: i64, min_billable_minutes: i64) -> i64 {
    let actual = actual_minutes.max(0);
    if actual == 0 {
        return 0;
    }

    let mut minutes = actual;
    if rounding_minutes > 0 {
        let increments = (minutes + rounding_minutes - 1) / rounding_minutes;
        minutes = increments * rounding_minutes;
    }
    if min_billable_minutes > 0 && minutes < min_billable_minutes {
        minutes = min_billable_minutes;
    }
    minutes
}

/// Money to whole cents. Every amount in a report and an export goes through
/// here, so a total is the sum of the rounded lines rather than a rounded sum of
/// unrounded lines (which is how invoices end up off by a cent).
pub fn to_cents(amount: f64) -> f64 {
    if !amount.is_finite() {
        return 0.0;
    }
    (amount * 100.0).round() / 100.0
}

/// The billable amount for a number of minutes at an hourly rate.
pub fn amount_for(rounded_minutes: i64, rate: f64) -> f64 {
    if rounded_minutes <= 0 || !rate.is_finite() || rate <= 0.0 {
        return 0.0;
    }
    to_cents(rounded_minutes as f64 / 60.0 * rate)
}

/// Everything computed for one meeting, before it is joined with its title.
#[derive(Debug, Clone, PartialEq)]
pub struct BillingComputation {
    pub minutes: i64,
    pub minutes_source: MinutesSource,
    pub rounded_minutes: i64,
    pub rate: Option<f64>,
    pub rate_source: RateSource,
    /// None whenever the row cannot be priced. Never 0.0 as a stand-in.
    pub amount: Option<f64>,
    pub state: RowState,
}

/// The single place a meeting turns into a billing line.
///
/// `raw_minutes` is the length derived from the recording; `minutes_source`
/// records how. The override, if any, replaces the length entirely (before the
/// increment is applied), which is how "we only bill 30 of those 50 minutes" is
/// expressed without touching the transcript.
pub fn compute(
    raw_minutes: i64,
    raw_source: MinutesSource,
    settings: &BillingSettings,
    client: Option<ClientBilling>,
    meeting: MeetingBillingOverride,
) -> BillingComputation {
    let (minutes, minutes_source) = match meeting.minutes_override {
        Some(override_minutes) if override_minutes >= 0 => {
            (override_minutes.min(MAX_MINUTES_OVERRIDE), MinutesSource::Override)
        }
        _ => (raw_minutes.max(0), raw_source),
    };

    let (rate, rate_source) = resolve_rate(
        client.and_then(|c| c.hourly_rate),
        settings.default_hourly_rate,
    );
    let billable = resolve_billable(meeting.billable, client.map(|c| c.billable));

    if !billable {
        return BillingComputation {
            minutes,
            minutes_source,
            rounded_minutes: 0,
            rate,
            rate_source,
            amount: None,
            state: RowState::NotBillable,
        };
    }

    let rounded_minutes = round_minutes(
        minutes,
        settings.rounding_minutes,
        settings.min_billable_minutes,
    );

    // Order matters: "no rate" is the more useful complaint to surface, since a
    // missing rate blocks every meeting while a missing length blocks one.
    let state = if rate.is_none() {
        RowState::NoRate
    } else if rounded_minutes == 0 {
        RowState::NoLength
    } else {
        RowState::Billable
    };

    let amount = match (state, rate) {
        (RowState::Billable, Some(rate)) => Some(amount_for(rounded_minutes, rate)),
        _ => None,
    };

    BillingComputation {
        minutes,
        minutes_source,
        rounded_minutes,
        rate,
        rate_source,
        amount,
        state,
    }
}

/// The internal-cost estimate: what the meeting cost the firm in salaried time,
/// as distinct from what it can be invoiced for.
///
/// Deliberately a separate function with a separate return type. It uses the
/// workspace rate only (a client's billing rate is a price, not a cost) and
/// returns None the moment either input is missing, because a cost figure with a
/// guessed attendee count is worse than no figure.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeetingCostEstimate {
    pub attendees: i64,
    pub minutes: i64,
    pub rate: f64,
    pub amount: f64,
}

pub fn estimate_meeting_cost(
    minutes: i64,
    attendees: Option<i64>,
    workspace_rate: Option<f64>,
) -> Option<MeetingCostEstimate> {
    let rate = sanitize_rate(workspace_rate)?;
    let attendees = attendees.filter(|n| *n > 0)?;
    if minutes <= 0 {
        return None;
    }
    Some(MeetingCostEstimate {
        attendees,
        minutes,
        rate,
        amount: to_cents(minutes as f64 / 60.0 * rate * attendees as f64),
    })
}

/// Minutes as `1h 05m`, for report rows and invoice lines.
pub fn format_minutes(minutes: i64) -> String {
    if minutes <= 0 {
        return "—".to_string();
    }
    let hours = minutes / 60;
    let mins = minutes % 60;
    if hours == 0 {
        format!("{}m", mins)
    } else if mins == 0 {
        format!("{}h", hours)
    } else {
        format!("{}h {:02}m", hours, mins)
    }
}

/// A money string for the given currency code. Known symbols get one, anything
/// else keeps its code, which is better than pretending every currency is
/// dollars.
pub fn format_money(amount: f64, currency: &str) -> String {
    let code = currency.trim().to_uppercase();
    let symbol = match code.as_str() {
        "USD" | "CAD" | "AUD" | "NZD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        _ => "",
    };
    let amount = to_cents(amount);
    if symbol.is_empty() {
        format!("{:.2} {}", amount, if code.is_empty() { "USD" } else { &code })
    } else {
        format!("{}{:.2}", symbol, amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(rate: Option<f64>, rounding: i64, minimum: i64) -> BillingSettings {
        BillingSettings {
            default_hourly_rate: rate,
            currency: "USD".to_string(),
            rounding_minutes: rounding,
            min_billable_minutes: minimum,
            include_internal: false,
        }
    }

    // ---- rate resolution -------------------------------------------------

    #[test]
    fn a_zero_or_negative_stored_rate_is_treated_as_unset() {
        assert_eq!(sanitize_rate(Some(0.0)), None);
        assert_eq!(sanitize_rate(Some(-150.0)), None);
        assert_eq!(sanitize_rate(Some(f64::NAN)), None);
        assert_eq!(sanitize_rate(Some(f64::INFINITY)), None);
        assert_eq!(sanitize_rate(Some(150.0)), Some(150.0));
        assert_eq!(sanitize_rate(None), None);
    }

    #[test]
    fn an_absurd_rate_is_refused_rather_than_used() {
        assert_eq!(sanitize_rate(Some(MAX_HOURLY_RATE + 1.0)), None);
        assert_eq!(sanitize_rate(Some(MAX_HOURLY_RATE)), Some(MAX_HOURLY_RATE));
    }

    #[test]
    fn the_client_rate_beats_the_workspace_rate() {
        assert_eq!(resolve_rate(Some(200.0), Some(150.0)), (Some(200.0), RateSource::Client));
        assert_eq!(resolve_rate(None, Some(150.0)), (Some(150.0), RateSource::Workspace));
        assert_eq!(resolve_rate(None, None), (None, RateSource::None));
        // A junk client rate falls through to the workspace rather than winning.
        assert_eq!(resolve_rate(Some(0.0), Some(150.0)), (Some(150.0), RateSource::Workspace));
    }

    #[test]
    fn billable_defaults_to_yes_and_the_meeting_has_the_last_word() {
        assert!(resolve_billable(None, None));
        assert!(resolve_billable(None, Some(true)));
        assert!(!resolve_billable(None, Some(false)));
        assert!(!resolve_billable(Some(false), Some(true)));
        // A meeting can be billed even for a client marked non-billable.
        assert!(resolve_billable(Some(true), Some(false)));
    }

    // ---- minutes ---------------------------------------------------------

    #[test]
    fn seconds_round_up_to_whole_minutes() {
        assert_eq!(seconds_to_minutes(0.0), 0);
        assert_eq!(seconds_to_minutes(1.0), 1);
        assert_eq!(seconds_to_minutes(60.0), 1);
        assert_eq!(seconds_to_minutes(61.0), 2);
        assert_eq!(seconds_to_minutes(3000.0), 50);
        assert_eq!(seconds_to_minutes(-5.0), 0);
        assert_eq!(seconds_to_minutes(f64::NAN), 0);
    }

    #[test]
    fn no_rounding_leaves_minutes_alone() {
        assert_eq!(round_minutes(37, 0, 0), 37);
        assert_eq!(round_minutes(1, 0, 0), 1);
    }

    #[test]
    fn rounding_goes_up_to_the_next_increment() {
        assert_eq!(round_minutes(1, 15, 0), 15);
        assert_eq!(round_minutes(15, 15, 0), 15);
        assert_eq!(round_minutes(16, 15, 0), 30);
        assert_eq!(round_minutes(50, 15, 0), 60);
        assert_eq!(round_minutes(60, 15, 0), 60);
        assert_eq!(round_minutes(7, 6, 0), 12);
    }

    #[test]
    fn the_minimum_lifts_a_short_call_but_never_creates_one() {
        assert_eq!(round_minutes(4, 0, 30), 30);
        assert_eq!(round_minutes(45, 0, 30), 45);
        // The crucial case: nothing recorded stays nothing.
        assert_eq!(round_minutes(0, 15, 30), 0);
        assert_eq!(round_minutes(0, 0, 30), 0);
    }

    #[test]
    fn rounding_and_minimum_compose_with_the_larger_winning() {
        // 5 minutes rounds to 15, then the 30-minute floor lifts it.
        assert_eq!(round_minutes(5, 15, 30), 30);
        // 40 minutes rounds to 45, which already clears the floor.
        assert_eq!(round_minutes(40, 15, 30), 45);
    }

    // ---- amounts ---------------------------------------------------------

    #[test]
    fn amounts_are_whole_cents() {
        assert_eq!(amount_for(60, 150.0), 150.0);
        assert_eq!(amount_for(30, 150.0), 75.0);
        assert_eq!(amount_for(15, 150.0), 37.5);
        // 7 minutes at 150/h is 17.499999..., which must land on 17.50.
        assert_eq!(amount_for(7, 150.0), 17.5);
        assert_eq!(amount_for(0, 150.0), 0.0);
        assert_eq!(amount_for(60, 0.0), 0.0);
    }

    #[test]
    fn cents_rounding_is_half_up_and_survives_junk() {
        assert_eq!(to_cents(1.005), 1.0);
        assert_eq!(to_cents(1.006), 1.01);
        assert_eq!(to_cents(f64::NAN), 0.0);
    }

    // ---- compute ---------------------------------------------------------

    #[test]
    fn a_normal_billable_meeting_prices_as_expected() {
        let result = compute(
            50,
            MinutesSource::Recorded,
            &settings(Some(150.0), 15, 0),
            None,
            MeetingBillingOverride::default(),
        );
        assert_eq!(result.state, RowState::Billable);
        assert_eq!(result.minutes, 50);
        assert_eq!(result.rounded_minutes, 60);
        assert_eq!(result.rate, Some(150.0));
        assert_eq!(result.rate_source, RateSource::Workspace);
        assert_eq!(result.amount, Some(150.0));
    }

    #[test]
    fn with_no_rate_anywhere_the_row_is_no_rate_and_has_no_amount() {
        let result = compute(
            50,
            MinutesSource::Recorded,
            &settings(None, 15, 0),
            None,
            MeetingBillingOverride::default(),
        );
        assert_eq!(result.state, RowState::NoRate);
        // The load-bearing assertion for the whole feature: not Some(0.0).
        assert_eq!(result.amount, None);
        assert_eq!(result.rate, None);
        assert!(!result.state.counts_toward_total());
    }

    #[test]
    fn a_meeting_with_no_length_is_no_length_not_a_minimum_charge() {
        let result = compute(
            0,
            MinutesSource::Unknown,
            &settings(Some(150.0), 15, 30),
            None,
            MeetingBillingOverride::default(),
        );
        assert_eq!(result.state, RowState::NoLength);
        assert_eq!(result.rounded_minutes, 0);
        assert_eq!(result.amount, None);
    }

    #[test]
    fn a_non_billable_meeting_keeps_its_minutes_but_earns_nothing() {
        let result = compute(
            50,
            MinutesSource::Recorded,
            &settings(Some(150.0), 15, 0),
            Some(ClientBilling { hourly_rate: None, billable: false }),
            MeetingBillingOverride::default(),
        );
        assert_eq!(result.state, RowState::NotBillable);
        assert_eq!(result.minutes, 50, "the length is still worth showing");
        assert_eq!(result.rounded_minutes, 0);
        assert_eq!(result.amount, None);
    }

    #[test]
    fn a_meeting_override_beats_the_client_flag_in_both_directions() {
        let non_billable_client = Some(ClientBilling { hourly_rate: Some(200.0), billable: false });
        let forced_on = compute(
            30,
            MinutesSource::Recorded,
            &settings(Some(150.0), 0, 0),
            non_billable_client,
            MeetingBillingOverride { billable: Some(true), minutes_override: None },
        );
        assert_eq!(forced_on.state, RowState::Billable);
        assert_eq!(forced_on.amount, Some(100.0));

        let forced_off = compute(
            30,
            MinutesSource::Recorded,
            &settings(Some(150.0), 0, 0),
            Some(ClientBilling::default()),
            MeetingBillingOverride { billable: Some(false), minutes_override: None },
        );
        assert_eq!(forced_off.state, RowState::NotBillable);
    }

    #[test]
    fn a_minutes_override_replaces_the_recorded_length_before_rounding() {
        let result = compute(
            50,
            MinutesSource::Recorded,
            &settings(Some(120.0), 15, 0),
            None,
            MeetingBillingOverride { billable: None, minutes_override: Some(20) },
        );
        assert_eq!(result.minutes, 20);
        assert_eq!(result.minutes_source, MinutesSource::Override);
        assert_eq!(result.rounded_minutes, 30);
        assert_eq!(result.amount, Some(60.0));
    }

    #[test]
    fn a_zero_minutes_override_is_honoured_as_do_not_bill_this_time() {
        let result = compute(
            50,
            MinutesSource::Recorded,
            &settings(Some(120.0), 15, 30),
            None,
            MeetingBillingOverride { billable: None, minutes_override: Some(0) },
        );
        assert_eq!(result.minutes, 0);
        assert_eq!(result.minutes_source, MinutesSource::Override);
        assert_eq!(result.state, RowState::NoLength);
        assert_eq!(result.amount, None);
    }

    #[test]
    fn an_absurd_minutes_override_is_clamped_not_trusted() {
        let result = compute(
            10,
            MinutesSource::Recorded,
            &settings(Some(100.0), 0, 0),
            None,
            MeetingBillingOverride { billable: None, minutes_override: Some(i64::MAX) },
        );
        assert_eq!(result.minutes, MAX_MINUTES_OVERRIDE);
    }

    #[test]
    fn a_negative_minutes_override_falls_back_to_the_recorded_length() {
        let result = compute(
            42,
            MinutesSource::Recorded,
            &settings(Some(100.0), 0, 0),
            None,
            MeetingBillingOverride { billable: None, minutes_override: Some(-5) },
        );
        assert_eq!(result.minutes, 42);
        assert_eq!(result.minutes_source, MinutesSource::Recorded);
    }

    #[test]
    fn no_rate_is_reported_ahead_of_no_length_when_both_are_missing() {
        let result = compute(
            0,
            MinutesSource::Unknown,
            &settings(None, 0, 0),
            None,
            MeetingBillingOverride::default(),
        );
        assert_eq!(result.state, RowState::NoRate);
    }

    // ---- cost estimate ---------------------------------------------------

    #[test]
    fn the_cost_estimate_multiplies_by_attendees() {
        let estimate = estimate_meeting_cost(60, Some(4), Some(150.0)).unwrap();
        assert_eq!(estimate.amount, 600.0);
        assert_eq!(estimate.attendees, 4);
        assert_eq!(estimate.rate, 150.0);
    }

    #[test]
    fn the_cost_estimate_refuses_to_guess() {
        // No attendee data: no figure, rather than a one-person assumption.
        assert!(estimate_meeting_cost(60, None, Some(150.0)).is_none());
        assert!(estimate_meeting_cost(60, Some(0), Some(150.0)).is_none());
        // No workspace rate: no figure.
        assert!(estimate_meeting_cost(60, Some(4), None).is_none());
        assert!(estimate_meeting_cost(60, Some(4), Some(0.0)).is_none());
        // No length: no figure.
        assert!(estimate_meeting_cost(0, Some(4), Some(150.0)).is_none());
    }

    #[test]
    fn the_cost_estimate_ignores_the_client_rate_by_construction() {
        // Only a workspace rate is accepted, so a client's price cannot be
        // mistaken for the firm's cost. This test documents the signature.
        let estimate = estimate_meeting_cost(30, Some(2), Some(100.0)).unwrap();
        assert_eq!(estimate.amount, 100.0);
    }

    // ---- formatting ------------------------------------------------------

    #[test]
    fn minutes_format_as_hours_and_minutes() {
        assert_eq!(format_minutes(0), "—");
        assert_eq!(format_minutes(-3), "—");
        assert_eq!(format_minutes(5), "5m");
        assert_eq!(format_minutes(60), "1h");
        assert_eq!(format_minutes(65), "1h 05m");
        assert_eq!(format_minutes(605), "10h 05m");
    }

    #[test]
    fn money_formats_with_a_symbol_when_one_is_known() {
        assert_eq!(format_money(1234.5, "USD"), "$1234.50");
        assert_eq!(format_money(10.0, "eur"), "€10.00");
        assert_eq!(format_money(10.0, "GBP"), "£10.00");
        assert_eq!(format_money(10.0, "CHF"), "10.00 CHF");
        assert_eq!(format_money(10.0, ""), "10.00 USD");
    }

    #[test]
    fn minutes_source_labels_are_stable() {
        assert_eq!(MinutesSource::Recorded.as_str(), "recorded");
        assert_eq!(MinutesSource::TranscriptSpan.as_str(), "transcript_span");
        assert_eq!(MinutesSource::SpeechTime.as_str(), "speech_time");
        assert_eq!(RowState::NoRate.as_str(), "no_rate");
    }
}
