'use client';

/**
 * The cost chip on meeting details: what this meeting is worth, and separately what
 * it cost the firm internally.
 *
 * The two figures are kept visually and verbally apart on purpose. The billable
 * amount is a number you could put on an invoice. The cost estimate is the
 * workspace rate multiplied by however many people were in the room, which is a
 * back-of-envelope figure and is labelled as one. Merging them would produce a
 * number that is neither.
 */

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { CircleDollarSign, RotateCcw } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Button } from '@/components/ui/button';
import type { AttendeeSource, MeetingBillingView } from '@/types/billing';
import {
  formatMinutes,
  formatMoney,
  getMeetingBilling,
  MINUTES_SOURCE_COPY,
  NO_RATE_LABEL,
  RATE_SOURCE_COPY,
  setMeetingBillingOverride,
} from '@/lib/billing';

const ATTENDEE_SOURCE_COPY: Record<AttendeeSource, string> = {
  calendar_attendees: 'from the attendee list captured for this meeting',
  diarized_speakers: 'from the number of distinct voices in the recording',
  none: '',
};

interface MeetingCostChipProps {
  meetingId: string;
  /** Bump to refetch, e.g. after the client tag changes. */
  refreshKey?: number;
}

export function MeetingCostChip({ meetingId, refreshKey = 0 }: MeetingCostChipProps) {
  const [view, setView] = useState<MeetingBillingView | null>(null);
  const [billable, setBillable] = useState(true);
  const [minutes, setMinutes] = useState('');
  const [note, setNote] = useState('');
  const [busy, setBusy] = useState(false);

  const apply = useCallback((loaded: MeetingBillingView) => {
    setView(loaded);
    setBillable(loaded.state !== 'not_billable');
    setMinutes(String(loaded.minutes));
    setNote(loaded.note);
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await getMeetingBilling(meetingId));
    } catch (error) {
      console.error("Failed to load the meeting's billing:", error);
      setView(null);
    }
  }, [meetingId, apply]);

  useEffect(() => {
    void load();
  }, [load, refreshKey]);

  if (!view) return null;

  const currency = view.currency;
  const label =
    view.amount !== null
      ? formatMoney(view.amount, currency)
      : view.state === 'not_billable'
        ? 'Not billable'
        : view.state === 'no_length'
          ? 'No length'
          : NO_RATE_LABEL;

  const save = async (clear: boolean) => {
    setBusy(true);
    try {
      const parsed = Number.parseInt(minutes, 10);
      apply(
        await setMeetingBillingOverride(
          meetingId,
          clear ? null : billable,
          clear || Number.isNaN(parsed) ? null : parsed,
          clear ? '' : note,
        ),
      );
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md border border-edge bg-wash text-xs text-ink hover:bg-active transition-colors"
          title="What this meeting is worth. Click to adjust."
        >
          <CircleDollarSign className="w-3.5 h-3.5 text-muted-ink" />
          <span className={view.amount === null ? 'text-muted-ink' : ''}>{label}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-96 p-3 space-y-3">
        <div>
          <div className="text-sm font-medium text-ink">Billable value</div>
          <div className="text-[11px] text-muted-ink">
            {formatMinutes(view.minutes)} recorded
            {view.rounded_minutes !== view.minutes
              ? `, billed as ${formatMinutes(view.rounded_minutes)}`
              : ''}
            {'. '}
            {MINUTES_SOURCE_COPY[view.minutes_source]}
          </div>
          <div className="text-[11px] text-muted-ink mt-1">
            {view.rate === null
              ? RATE_SOURCE_COPY.none
              : `${formatMoney(view.rate, currency)} per hour — ${RATE_SOURCE_COPY[view.rate_source]}`}
          </div>
        </div>

        {/* The internal-cost estimate, clearly separated from the amount above. */}
        <div className="border-t border-edge pt-2">
          <div className="text-sm font-medium text-ink">Estimated internal cost</div>
          {view.cost_estimate ? (
            <div className="text-[11px] text-muted-ink">
              {formatMoney(view.cost_estimate.amount, currency)} — {view.cost_estimate.attendees}{' '}
              {view.cost_estimate.attendees === 1 ? 'person' : 'people'}{' '}
              {ATTENDEE_SOURCE_COPY[view.attendee_source]} for{' '}
              {formatMinutes(view.cost_estimate.minutes)} at the workspace rate. This is a rough
              figure for what the meeting cost the firm, not an amount to invoice.
            </div>
          ) : (
            <div className="text-[11px] text-muted-ink">
              Not shown: this needs both a workspace rate and a known number of attendees. A
              guessed headcount would make the figure meaningless.
            </div>
          )}
        </div>

        {/* Override */}
        <div className="border-t border-edge pt-2 space-y-2">
          <div className="text-[11px] font-medium text-muted-ink">Adjust this meeting</div>
          <label className="flex items-center gap-2 text-xs text-ink">
            <input
              type="checkbox"
              checked={billable}
              onChange={event => setBillable(event.target.checked)}
              className="accent-ink"
            />
            Billable
          </label>
          <div className="flex items-end gap-2">
            <label className="text-xs text-ink">
              <span className="block text-[11px] text-muted-ink mb-1">Billed minutes</span>
              <input
                type="number"
                min={0}
                value={minutes}
                onChange={event => setMinutes(event.target.value)}
                className="w-24 rounded-md border border-edge bg-surface px-2 py-1 text-xs text-ink"
              />
            </label>
            <label className="flex-1 text-xs text-ink">
              <span className="block text-[11px] text-muted-ink mb-1">Note</span>
              <input
                type="text"
                value={note}
                onChange={event => setNote(event.target.value)}
                className="w-full rounded-md border border-edge bg-surface px-2 py-1 text-xs text-ink"
              />
            </label>
          </div>
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={() => void save(false)} disabled={busy}>
              Save
            </Button>
            <Button size="sm" variant="ghost" onClick={() => void save(true)} disabled={busy}>
              <RotateCcw className="w-3 h-3 mr-1" />
              Inherit
            </Button>
          </div>
          <p className="text-[11px] text-faint">
            This changes the billing figure only. The transcript and the recording are not
            touched.
          </p>
        </div>
      </PopoverContent>
    </Popover>
  );
}
