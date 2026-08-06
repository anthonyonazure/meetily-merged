/**
 * Billing types, mirroring the Rust payloads in `src-tauri/src/billing/`.
 *
 * Note the nullable money fields. `rate: null` means "no rate configured" and
 * `amount: null` means "this row could not be priced". Neither is ever 0, and the
 * UI must render them as words, not as a number.
 */

/** Where a row's rate came from. */
export type RateSource = 'client' | 'workspace' | 'none';

/** How a meeting's length was established. */
export type MinutesSource =
  | 'override'
  | 'recorded'
  | 'transcript_span'
  | 'speech_time'
  | 'unknown';

/** Whether a row counts toward the total, and why not when it does not. */
export type RowState = 'billable' | 'not_billable' | 'no_rate' | 'no_length';

export interface BillingSettings {
  /** Null means no workspace rate is configured. */
  default_hourly_rate: number | null;
  currency: string;
  /** 0 means no rounding. */
  rounding_minutes: number;
  /** 0 means no minimum. */
  min_billable_minutes: number;
  /** Whether meetings with no client tag appear in the report. */
  include_internal: boolean;
}

export interface BillingSettingsInput {
  default_hourly_rate: number | null;
  currency: string;
  rounding_minutes: number;
  min_billable_minutes: number;
  include_internal: boolean;
}

export interface ClientBillingView {
  client_id: string;
  /** Null means this client has no rate of its own. */
  hourly_rate: number | null;
  billable: boolean;
  /** The rate actually in force once the workspace fallback is applied. */
  effective_rate: number | null;
  effective_rate_source: RateSource;
  currency: string;
}

export interface BillingRow {
  meeting_id: string;
  title: string;
  date: string;
  client_id: string | null;
  client_name: string | null;
  minutes: number;
  minutes_source: MinutesSource;
  rounded_minutes: number;
  /** Null means no rate is set. Render as "no rate set". */
  rate: number | null;
  rate_source: RateSource;
  /** Null whenever the row could not be priced. Never render as 0.00. */
  amount: number | null;
  state: RowState;
  billable: boolean;
  note: string;
}

export interface ExcludedCounts {
  not_billable: number;
  no_rate: number;
  no_length: number;
}

export interface BillingReport {
  start: string;
  end: string;
  client_id: string | null;
  currency: string;
  rounding_minutes: number;
  min_billable_minutes: number;
  rows: BillingRow[];
  billable_meetings: number;
  total_minutes: number;
  total_rounded_minutes: number;
  total_amount: number;
  excluded: ExcludedCounts;
  /** Set when nothing can be priced because no rate exists anywhere. */
  warning: string | null;
}

export interface BillingExportResult {
  folder: string;
  csv_path: string;
  markdown_path: string;
  rows: number;
  billable_meetings: number;
  excluded: number;
}

/** Where an attendee count came from, for the cost estimate. */
export type AttendeeSource = 'calendar_attendees' | 'diarized_speakers' | 'none';

export interface MeetingCostEstimate {
  attendees: number;
  minutes: number;
  rate: number;
  amount: number;
}

export interface MeetingBillingView {
  meeting_id: string;
  client_id: string | null;
  client_name: string | null;
  minutes: number;
  minutes_source: MinutesSource;
  rounded_minutes: number;
  rate: number | null;
  rate_source: RateSource;
  amount: number | null;
  state: RowState;
  currency: string;
  billable_override: boolean | null;
  minutes_override: number | null;
  note: string;
  /** Null when attendees or the workspace rate are unknown. Never a guess. */
  cost_estimate: MeetingCostEstimate | null;
  attendee_source: AttendeeSource;
}
