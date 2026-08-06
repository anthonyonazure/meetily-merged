/**
 * Billing client: the Tauri command wrappers plus the formatting the UI shares.
 *
 * The formatting rule that runs through this file: a null rate or amount renders
 * as `NO_RATE_LABEL`, never as a currency-formatted zero. A zero next to a client's
 * name reads as "this work was free", which is a different claim from "nobody has
 * set a rate yet".
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  BillingExportResult,
  BillingReport,
  BillingSettings,
  BillingSettingsInput,
  ClientBillingView,
  MeetingBillingView,
  MinutesSource,
  RateSource,
  RowState,
} from '@/types/billing';

/** Shown wherever a rate or amount is unknown. Matches the Rust exports. */
export const NO_RATE_LABEL = 'no rate set';

const CURRENCY_SYMBOLS: Record<string, string> = {
  USD: '$',
  CAD: '$',
  AUD: '$',
  NZD: '$',
  EUR: '€',
  GBP: '£',
  JPY: '¥',
};

/** Money, or the honest label when there is no number to show. */
export function formatMoney(amount: number | null, currency: string): string {
  if (amount === null || amount === undefined) return NO_RATE_LABEL;
  const code = (currency || 'USD').toUpperCase();
  const symbol = CURRENCY_SYMBOLS[code];
  const value = amount.toFixed(2);
  return symbol ? `${symbol}${value}` : `${value} ${code}`;
}

/** An hourly rate, or the label. */
export function formatRate(rate: number | null, currency: string): string {
  if (rate === null || rate === undefined) return NO_RATE_LABEL;
  return `${formatMoney(rate, currency)}/h`;
}

/** Minutes as `1h 05m`. */
export function formatMinutes(minutes: number): string {
  if (!minutes || minutes <= 0) return '—';
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest}m`;
  if (rest === 0) return `${hours}h`;
  return `${hours}h ${String(rest).padStart(2, '0')}m`;
}

/** Plain English for how a length was established, for a tooltip. */
export const MINUTES_SOURCE_COPY: Record<MinutesSource, string> = {
  override: 'You set these minutes by hand.',
  recorded: 'The recorded length of the meeting.',
  transcript_span:
    'Estimated from the first and last thing said, so silence at either end is not counted.',
  speech_time:
    'Speech time only, added up from the transcript segments. This under-reports the meeting.',
  unknown: 'No length could be worked out from this meeting.',
};

export const RATE_SOURCE_COPY: Record<RateSource, string> = {
  client: "This client's own rate.",
  workspace: 'The workspace rate.',
  none: 'No rate is configured, so this meeting cannot be priced.',
};

export const ROW_STATE_COPY: Record<RowState, string> = {
  billable: 'Counted in the total.',
  not_billable: 'Marked non-billable, so it is not in the total.',
  no_rate: 'No rate is set, so it is not in the total.',
  no_length: 'No recorded length, so it is not in the total.',
};

/** The one-line account of what was left out, or null when nothing was. */
export function describeExcluded(report: BillingReport): string | null {
  const { not_billable, no_rate, no_length } = report.excluded;
  const total = not_billable + no_rate + no_length;
  if (total === 0) return null;
  const parts: string[] = [];
  if (no_rate > 0) parts.push(`${no_rate} with no rate set`);
  if (no_length > 0) parts.push(`${no_length} with no recorded length`);
  if (not_billable > 0) parts.push(`${not_billable} marked non-billable`);
  return `${total} meeting${total === 1 ? '' : 's'} excluded from the total: ${parts.join(', ')}`;
}

/** `YYYY-MM-DD` for a date input. */
export function toDateInput(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

/** The first and last day of the month a date falls in. */
export function monthRange(date: Date): { from: string; to: string } {
  return {
    from: toDateInput(new Date(date.getFullYear(), date.getMonth(), 1)),
    to: toDateInput(new Date(date.getFullYear(), date.getMonth() + 1, 0)),
  };
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export function getBillingSettings(): Promise<BillingSettings> {
  return invoke<BillingSettings>('billing_settings_get');
}

export function setBillingSettings(input: BillingSettingsInput): Promise<BillingSettings> {
  return invoke<BillingSettings>('billing_settings_set', { input });
}

export function getClientBilling(clientId: string): Promise<ClientBillingView> {
  return invoke<ClientBillingView>('client_billing_get', { clientId });
}

export function setClientBilling(
  clientId: string,
  hourlyRate: number | null,
  billable: boolean,
): Promise<ClientBillingView> {
  return invoke<ClientBillingView>('client_billing_set', { clientId, hourlyRate, billable });
}

export function getMeetingBilling(meetingId: string): Promise<MeetingBillingView> {
  return invoke<MeetingBillingView>('meeting_billing_get', { meetingId });
}

export function setMeetingBillingOverride(
  meetingId: string,
  billable: boolean | null,
  minutesOverride: number | null,
  note: string,
): Promise<MeetingBillingView> {
  return invoke<MeetingBillingView>('meeting_billing_override_set', {
    meetingId,
    billable,
    minutesOverride,
    note,
  });
}

export function getBillingReport(
  from: string,
  to: string,
  clientId: string | null,
): Promise<BillingReport> {
  return invoke<BillingReport>('billing_report', { from, to, clientId });
}

export function exportBillingReport(
  from: string,
  to: string,
  clientId: string | null,
): Promise<BillingExportResult> {
  return invoke<BillingExportResult>('billing_export', { from, to, clientId });
}
