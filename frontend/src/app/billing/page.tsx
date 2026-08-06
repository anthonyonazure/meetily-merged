'use client';

/**
 * The Billing page: what a period of meetings is worth, per client.
 *
 * Two things this screen refuses to do, both deliberate. It never shows a money
 * figure it had to guess: a meeting with no rate reads "no rate set" and is listed
 * under what was excluded. And it never hides those rows to make the total look
 * tidier, because a short total nobody can explain is worse than a total with a
 * visible gap.
 */

import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import {
  AlertTriangle,
  Check,
  Download,
  Receipt,
  RotateCcw,
  Settings2,
  X,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { BillingReport, BillingRow, BillingSettings } from '@/types/billing';
import type { ClientWithCounts } from '@/types/clients';
import {
  describeExcluded,
  exportBillingReport,
  formatMinutes,
  formatMoney,
  getBillingReport,
  getBillingSettings,
  monthRange,
  MINUTES_SOURCE_COPY,
  NO_RATE_LABEL,
  RATE_SOURCE_COPY,
  ROW_STATE_COPY,
  setMeetingBillingOverride,
} from '@/lib/billing';

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** The per-row editor: billable on/off, and the minutes actually billed. */
function RowEditor({
  row,
  currency,
  onSaved,
  onClose,
}: {
  row: BillingRow;
  currency: string;
  onSaved: () => void;
  onClose: () => void;
}) {
  const [billable, setBillable] = useState<boolean>(row.state !== 'not_billable');
  const [minutes, setMinutes] = useState<string>(String(row.minutes));
  const [note, setNote] = useState<string>(row.note);
  const [busy, setBusy] = useState(false);

  const save = async (clear: boolean) => {
    setBusy(true);
    try {
      const parsed = Number.parseInt(minutes, 10);
      await setMeetingBillingOverride(
        row.meeting_id,
        clear ? null : billable,
        clear || Number.isNaN(parsed) ? null : parsed,
        clear ? '' : note,
      );
      onSaved();
      onClose();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="border-t border-edge bg-wash px-4 py-3 space-y-3">
      <div className="flex flex-wrap items-end gap-4">
        <label className="flex items-center gap-2 text-sm text-ink">
          <input
            type="checkbox"
            checked={billable}
            onChange={event => setBillable(event.target.checked)}
            className="accent-ink"
          />
          Billable
        </label>
        <label className="text-sm text-ink">
          <span className="block text-xs text-muted-ink mb-1">Billed minutes</span>
          <input
            type="number"
            min={0}
            value={minutes}
            onChange={event => setMinutes(event.target.value)}
            className="w-28 rounded-md border border-edge bg-surface px-2 py-1 text-sm text-ink"
          />
        </label>
        <label className="flex-1 min-w-[12rem] text-sm text-ink">
          <span className="block text-xs text-muted-ink mb-1">Note (kept with the meeting)</span>
          <input
            type="text"
            value={note}
            onChange={event => setNote(event.target.value)}
            placeholder="Why this differs from the recording"
            className="w-full rounded-md border border-edge bg-surface px-2 py-1 text-sm text-ink"
          />
        </label>
      </div>
      <p className="text-xs text-muted-ink">
        Recorded length: {formatMinutes(row.minutes)}. {MINUTES_SOURCE_COPY[row.minutes_source]} At{' '}
        {row.rate === null ? NO_RATE_LABEL : formatMoney(row.rate, currency)}
        {row.rate === null ? '' : ' per hour'}. Changing this does not touch the transcript.
      </p>
      <div className="flex items-center gap-2">
        <Button size="sm" onClick={() => void save(false)} disabled={busy}>
          <Check className="w-3.5 h-3.5 mr-1" />
          Save
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void save(true)} disabled={busy}>
          <RotateCcw className="w-3.5 h-3.5 mr-1" />
          Back to inherited
        </Button>
        <Button size="sm" variant="ghost" onClick={onClose} disabled={busy}>
          <X className="w-3.5 h-3.5 mr-1" />
          Cancel
        </Button>
      </div>
    </div>
  );
}

export default function BillingPage() {
  const initial = useMemo(() => monthRange(new Date()), []);
  const [from, setFrom] = useState(initial.from);
  const [to, setTo] = useState(initial.to);
  const [clientId, setClientId] = useState<string>('');
  const [clients, setClients] = useState<ClientWithCounts[]>([]);
  const [settings, setSettings] = useState<BillingSettings | null>(null);
  const [report, setReport] = useState<BillingReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        setClients(await invoke<ClientWithCounts[]>('clients_list'));
      } catch (error) {
        console.error('Failed to load clients:', error);
      }
      try {
        setSettings(await getBillingSettings());
      } catch (error) {
        console.error('Failed to load billing settings:', error);
      }
    })();
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setReport(await getBillingReport(from, to, clientId || null));
    } catch (error) {
      toast.error(String(error));
      setReport(null);
    } finally {
      setLoading(false);
    }
  }, [from, to, clientId]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleExport = async () => {
    try {
      const result = await exportBillingReport(from, to, clientId || null);
      toast.success(`Exported ${result.rows} row(s) to ${result.folder}`);
    } catch (error) {
      if (String(error).includes('cancelled')) return;
      toast.error(String(error));
    }
  };

  const currency = report?.currency ?? settings?.currency ?? 'USD';
  const excluded = report ? describeExcluded(report) : null;

  return (
    <div className="h-screen overflow-y-auto custom-scrollbar bg-app">
      <div className="max-w-5xl mx-auto p-8 space-y-6">
        <div className="flex items-start gap-3">
          <Receipt className="w-6 h-6 text-muted-ink mt-1" />
          <div className="flex-1">
            <h1 className="text-2xl font-display font-semibold text-ink">Billable time</h1>
            <p className="text-sm text-muted-ink mt-0.5">
              Every recorded meeting, priced from its actual length. Nothing here is
              invoiced automatically.
            </p>
          </div>
        </div>

        {/* Filters */}
        <div className="bg-surface border border-edge rounded-lg p-4">
          <div className="flex flex-wrap items-end gap-4">
            <label className="text-sm">
              <span className="block text-xs text-muted-ink mb-1">From</span>
              <input
                type="date"
                value={from}
                onChange={event => setFrom(event.target.value)}
                className="rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
              />
            </label>
            <label className="text-sm">
              <span className="block text-xs text-muted-ink mb-1">To</span>
              <input
                type="date"
                value={to}
                onChange={event => setTo(event.target.value)}
                className="rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
              />
            </label>
            <label className="text-sm flex-1 min-w-[10rem]">
              <span className="block text-xs text-muted-ink mb-1">Client</span>
              <select
                value={clientId}
                onChange={event => setClientId(event.target.value)}
                className="w-full rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
              >
                <option value="">All clients</option>
                {clients.map(client => (
                  <option key={client.id} value={client.id}>
                    {client.name}
                  </option>
                ))}
              </select>
            </label>
            <Button variant="outline" size="sm" onClick={() => void handleExport()}>
              <Download className="w-3.5 h-3.5 mr-1.5" />
              Export CSV and invoice summary
            </Button>
          </div>
          {settings && (
            <p className="text-xs text-faint mt-3">
              Workspace rate:{' '}
              {settings.default_hourly_rate === null
                ? NO_RATE_LABEL
                : `${formatMoney(settings.default_hourly_rate, currency)} per hour`}
              {settings.rounding_minutes > 0
                ? ` · rounded up to ${settings.rounding_minutes} minutes`
                : ' · no rounding'}
              {settings.min_billable_minutes > 0
                ? ` · minimum ${settings.min_billable_minutes} minutes`
                : ''}
              {settings.include_internal ? ' · untagged meetings included' : ''}
              {'. '}
              <span className="inline-flex items-center gap-1">
                <Settings2 className="w-3 h-3" />
                Change this in Settings → Billing.
              </span>
            </p>
          )}
        </div>

        {report?.warning && (
          <div className="flex items-start gap-2 rounded-lg border border-edge bg-wash px-4 py-3">
            <AlertTriangle className="w-4 h-4 text-muted-ink mt-0.5 flex-shrink-0" />
            <p className="text-sm text-ink">{report.warning}</p>
          </div>
        )}

        {/* Totals */}
        {report && (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
            <div className="bg-surface border border-edge rounded-lg p-4">
              <div className="text-xs text-muted-ink">Billable meetings</div>
              <div className="text-xl font-display text-ink">{report.billable_meetings}</div>
            </div>
            <div className="bg-surface border border-edge rounded-lg p-4">
              <div className="text-xs text-muted-ink">Recorded time</div>
              <div className="text-xl font-display text-ink">
                {formatMinutes(report.total_minutes)}
              </div>
            </div>
            <div className="bg-surface border border-edge rounded-lg p-4">
              <div className="text-xs text-muted-ink">Billed time</div>
              <div className="text-xl font-display text-ink">
                {formatMinutes(report.total_rounded_minutes)}
              </div>
            </div>
            <div className="bg-surface border border-edge rounded-lg p-4">
              <div className="text-xs text-muted-ink">Total</div>
              <div className="text-xl font-display text-ink">
                {formatMoney(report.total_amount, currency)}
              </div>
            </div>
          </div>
        )}

        {excluded && (
          <p className="text-xs text-muted-ink border border-dashed border-edge rounded-lg px-4 py-2">
            {excluded}
          </p>
        )}

        {/* Rows */}
        <div className="bg-surface border border-edge rounded-lg overflow-hidden">
          {loading && !report ? (
            <div className="p-6 text-sm text-faint">Loading…</div>
          ) : !report || report.rows.length === 0 ? (
            <div className="p-6 text-sm text-faint">
              No meetings in this range.
              {settings && !settings.include_internal
                ? ' Meetings with no client tag are hidden; turn on "include untagged meetings" in Settings → Billing to see them.'
                : ''}
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs text-muted-ink border-b border-edge">
                  <th className="px-4 py-2 font-medium">Date</th>
                  <th className="px-4 py-2 font-medium">Meeting</th>
                  <th className="px-4 py-2 font-medium">Client</th>
                  <th className="px-4 py-2 font-medium text-right">Recorded</th>
                  <th className="px-4 py-2 font-medium text-right">Billed</th>
                  <th className="px-4 py-2 font-medium text-right">Rate</th>
                  <th className="px-4 py-2 font-medium text-right">Amount</th>
                  <th className="px-4 py-2" />
                </tr>
              </thead>
              <tbody>
                {report.rows.map(row => (
                  <Fragment key={row.meeting_id}>
                    <tr
                      className={`border-b border-edge ${
                        row.state === 'billable' ? '' : 'text-muted-ink'
                      }`}
                    >
                      <td className="px-4 py-2 whitespace-nowrap">{formatDate(row.date)}</td>
                      <td className="px-4 py-2">
                        <span className="text-ink">{row.title}</span>
                        {row.note ? (
                          <span className="block text-xs text-faint">{row.note}</span>
                        ) : null}
                      </td>
                      <td className="px-4 py-2">{row.client_name ?? 'Internal'}</td>
                      <td
                        className="px-4 py-2 text-right whitespace-nowrap"
                        title={MINUTES_SOURCE_COPY[row.minutes_source]}
                      >
                        {formatMinutes(row.minutes)}
                        {row.minutes_source !== 'recorded' && row.minutes_source !== 'override' ? (
                          <span className="text-faint"> *</span>
                        ) : null}
                      </td>
                      <td className="px-4 py-2 text-right whitespace-nowrap">
                        {formatMinutes(row.rounded_minutes)}
                      </td>
                      <td
                        className="px-4 py-2 text-right whitespace-nowrap"
                        title={RATE_SOURCE_COPY[row.rate_source]}
                      >
                        {row.rate === null ? (
                          <span className="text-faint italic">{NO_RATE_LABEL}</span>
                        ) : (
                          formatMoney(row.rate, currency)
                        )}
                      </td>
                      <td className="px-4 py-2 text-right whitespace-nowrap">
                        {row.amount === null ? (
                          <span
                            className="text-faint italic"
                            title={ROW_STATE_COPY[row.state]}
                          >
                            {row.state === 'not_billable'
                              ? 'not billable'
                              : row.state === 'no_length'
                                ? 'no length'
                                : NO_RATE_LABEL}
                          </span>
                        ) : (
                          <span className="text-ink">{formatMoney(row.amount, currency)}</span>
                        )}
                      </td>
                      <td className="px-4 py-2 text-right">
                        <button
                          onClick={() =>
                            setEditing(editing === row.meeting_id ? null : row.meeting_id)
                          }
                          className="text-xs text-muted-ink hover:text-ink underline"
                        >
                          Adjust
                        </button>
                      </td>
                    </tr>
                    {editing === row.meeting_id && (
                      <tr>
                        <td colSpan={8} className="p-0">
                          <RowEditor
                            row={row}
                            currency={currency}
                            onSaved={() => void load()}
                            onClose={() => setEditing(null)}
                          />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <p className="text-xs text-faint">
          * This length was estimated from the transcript rather than measured from the
          recording. Hover the figure to see which.
        </p>
      </div>
    </div>
  );
}
