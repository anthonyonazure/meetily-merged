'use client';

/**
 * Settings → Billing: the workspace rate, currency, rounding, and minimum.
 *
 * The rate field is deliberately empty-able. Clearing it is a real choice ("we
 * have not agreed a rate") and is stored as no rate at all, which makes every
 * unpriced meeting say so instead of quietly billing at nothing.
 */

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Receipt } from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { BillingSettings } from '@/types/billing';
import { getBillingSettings, NO_RATE_LABEL, setBillingSettings } from '@/lib/billing';

const ROUNDING_OPTIONS = [
  { value: 0, label: 'No rounding' },
  { value: 6, label: '6 minutes (tenth of an hour)' },
  { value: 10, label: '10 minutes' },
  { value: 15, label: '15 minutes' },
  { value: 30, label: '30 minutes' },
  { value: 60, label: '1 hour' },
];

export function BillingSettingsPanel() {
  const [settings, setSettings] = useState<BillingSettings | null>(null);
  const [rate, setRate] = useState('');
  const [currency, setCurrency] = useState('USD');
  const [rounding, setRounding] = useState(0);
  const [minimum, setMinimum] = useState('0');
  const [includeInternal, setIncludeInternal] = useState(false);
  const [busy, setBusy] = useState(false);

  const apply = useCallback((loaded: BillingSettings) => {
    setSettings(loaded);
    setRate(loaded.default_hourly_rate === null ? '' : String(loaded.default_hourly_rate));
    setCurrency(loaded.currency);
    setRounding(loaded.rounding_minutes);
    setMinimum(String(loaded.min_billable_minutes));
    setIncludeInternal(loaded.include_internal);
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        apply(await getBillingSettings());
      } catch (error) {
        console.error('Failed to load billing settings:', error);
      }
    })();
  }, [apply]);

  const save = async () => {
    setBusy(true);
    try {
      const trimmed = rate.trim();
      const parsedRate = trimmed === '' ? null : Number.parseFloat(trimmed);
      if (parsedRate !== null && Number.isNaN(parsedRate)) {
        toast.error('That rate is not a number.');
        return;
      }
      const parsedMinimum = Number.parseInt(minimum, 10);
      apply(
        await setBillingSettings({
          default_hourly_rate: parsedRate,
          currency,
          rounding_minutes: rounding,
          min_billable_minutes: Number.isNaN(parsedMinimum) ? 0 : parsedMinimum,
          include_internal: includeInternal,
        }),
      );
      toast.success('Billing settings saved');
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-6 mt-6 max-w-2xl">
      <div className="flex items-start gap-3">
        <Receipt className="w-5 h-5 text-muted-ink mt-0.5" />
        <div>
          <h2 className="text-lg font-display font-semibold section-header inline-block">
            Billing
          </h2>
          <p className="text-sm text-muted-ink mt-2">
            The default rate and rounding used on the Billable time page. A client can
            have its own rate, which wins over this one.
          </p>
        </div>
      </div>

      <div className="bg-surface border border-edge rounded-lg p-4 space-y-4">
        <div className="flex flex-wrap gap-4">
          <label className="text-sm">
            <span className="block text-xs text-muted-ink mb-1">Workspace hourly rate</span>
            <input
              type="number"
              min={0}
              step="0.01"
              value={rate}
              onChange={event => setRate(event.target.value)}
              placeholder="Leave empty for no rate"
              className="w-44 rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
            />
          </label>
          <label className="text-sm">
            <span className="block text-xs text-muted-ink mb-1">Currency</span>
            <input
              type="text"
              maxLength={3}
              value={currency}
              onChange={event => setCurrency(event.target.value.toUpperCase())}
              className="w-24 rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink uppercase"
            />
          </label>
        </div>
        <p className="text-xs text-faint">
          Leaving the rate empty is a real answer. Meetings then read &quot;{NO_RATE_LABEL}
          &quot; on the Billable time page and are listed separately rather than counted as
          nothing.
        </p>

        <div className="flex flex-wrap gap-4">
          <label className="text-sm">
            <span className="block text-xs text-muted-ink mb-1">Round each meeting up to</span>
            <select
              value={rounding}
              onChange={event => setRounding(Number.parseInt(event.target.value, 10))}
              className="rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
            >
              {ROUNDING_OPTIONS.map(option => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
          <label className="text-sm">
            <span className="block text-xs text-muted-ink mb-1">Minimum billable minutes</span>
            <input
              type="number"
              min={0}
              value={minimum}
              onChange={event => setMinimum(event.target.value)}
              className="w-32 rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
            />
          </label>
        </div>
        <p className="text-xs text-faint">
          Rounding and the minimum apply per meeting, not to the total. A meeting with no
          recorded length stays at zero either way: the minimum lifts a short call, it does
          not create one.
        </p>

        <label className="flex items-start gap-2 text-sm text-ink">
          <input
            type="checkbox"
            checked={includeInternal}
            onChange={event => setIncludeInternal(event.target.checked)}
            className="accent-ink mt-0.5"
          />
          <span>
            Include meetings with no client tag
            <span className="block text-xs text-muted-ink">
              Your own internal time. Off by default, since it is not invoiced.
            </span>
          </span>
        </label>

        <div className="flex items-center gap-3 pt-1">
          <Button size="sm" onClick={() => void save()} disabled={busy}>
            Save
          </Button>
          {settings && (
            <span className="text-xs text-faint">
              Currently{' '}
              {settings.default_hourly_rate === null
                ? NO_RATE_LABEL
                : `${settings.default_hourly_rate} ${settings.currency} per hour`}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
