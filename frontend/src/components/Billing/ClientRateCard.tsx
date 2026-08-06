'use client';

/**
 * The rate on a client record. Shows what is in force and where it came from, so
 * "why is this meeting priced at 150?" is answered without opening two screens.
 */

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { CircleDollarSign } from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { ClientBillingView } from '@/types/billing';
import { formatRate, getClientBilling, NO_RATE_LABEL, setClientBilling } from '@/lib/billing';

interface ClientRateCardProps {
  clientId: string;
  clientName: string;
}

export function ClientRateCard({ clientId, clientName }: ClientRateCardProps) {
  const [view, setView] = useState<ClientBillingView | null>(null);
  const [rate, setRate] = useState('');
  const [billable, setBillable] = useState(true);
  const [busy, setBusy] = useState(false);

  const apply = useCallback((loaded: ClientBillingView) => {
    setView(loaded);
    setRate(loaded.hourly_rate === null ? '' : String(loaded.hourly_rate));
    setBillable(loaded.billable);
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        apply(await getClientBilling(clientId));
      } catch (error) {
        console.error("Failed to load the client's billing:", error);
      }
    })();
  }, [clientId, apply]);

  const save = async () => {
    setBusy(true);
    try {
      const trimmed = rate.trim();
      const parsed = trimmed === '' ? null : Number.parseFloat(trimmed);
      if (parsed !== null && Number.isNaN(parsed)) {
        toast.error('That rate is not a number.');
        return;
      }
      apply(await setClientBilling(clientId, parsed, billable));
      toast.success(`Saved billing for ${clientName}`);
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const currency = view?.currency ?? 'USD';

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <CircleDollarSign className="w-4 h-4 text-muted-ink" />
        <h3 className="text-sm font-medium text-ink">Rate</h3>
      </div>

      <div className="flex flex-wrap items-end gap-3">
        <label className="text-sm">
          <span className="block text-xs text-muted-ink mb-1">Hourly rate for {clientName}</span>
          <input
            type="number"
            min={0}
            step="0.01"
            value={rate}
            onChange={event => setRate(event.target.value)}
            placeholder="Use the workspace rate"
            className="w-44 rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
          />
        </label>
        <label className="flex items-center gap-2 text-sm text-ink pb-1.5">
          <input
            type="checkbox"
            checked={billable}
            onChange={event => setBillable(event.target.checked)}
            className="accent-ink"
          />
          Billable
        </label>
        <Button size="sm" onClick={() => void save()} disabled={busy} className="mb-0.5">
          Save
        </Button>
      </div>

      {view && (
        <p className="text-xs text-muted-ink">
          {view.effective_rate === null ? (
            <>
              No rate applies yet, so this client&apos;s meetings show &quot;{NO_RATE_LABEL}&quot;
              on the Billable time page. Set one here, or set a workspace rate in Settings →
              Billing.
            </>
          ) : (
            <>
              In force: {formatRate(view.effective_rate, currency)}{' '}
              {view.effective_rate_source === 'client'
                ? "(this client's own rate)"
                : '(inherited from the workspace rate)'}
              {view.billable ? '' : '. Meetings for this client are marked non-billable.'}
            </>
          )}
        </p>
      )}
    </div>
  );
}
