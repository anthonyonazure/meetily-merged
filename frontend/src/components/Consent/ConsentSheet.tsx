'use client';

/**
 * The pre-record consent sheet.
 *
 * Only appears when the level in force needs something from the operator, or
 * when a blocking rule matched. At `self_only` it never appears at all — the
 * whole point of that level is that it costs nothing.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { AlertTriangle, Check, Copy, Plus, Volume2, X } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  grantClearance,
  levelCopy,
  prefillAttendees,
  speakAnnouncement,
} from '@/lib/consent';
import type {
  AttendeeDecision,
  ConsentDecisionState,
  ConsentMethod,
  ConsentPlan,
} from '@/types/consent';

interface ConsentSheetProps {
  plan: ConsentPlan | null;
  /** Called after clearance is granted; the caller then starts recording. */
  onCleared: () => void;
  onCancel: () => void;
}

const STATE_CYCLE: ConsentDecisionState[] = ['unknown', 'consented', 'declined'];

const STATE_COPY: Record<ConsentDecisionState, { label: string; className: string }> = {
  consented: { label: 'Told, no objection', className: 'bg-ink text-app' },
  declined: { label: 'Objected', className: 'bg-rec text-app' },
  unknown: { label: 'Not confirmed', className: 'bg-wash text-muted-ink border border-edge' },
};

export function ConsentSheet({ plan, onCleared, onCancel }: ConsentSheetProps) {
  const [attendees, setAttendees] = useState<AttendeeDecision[]>([]);
  const [newAttendee, setNewAttendee] = useState('');
  const [noticeMethod, setNoticeMethod] = useState<ConsentMethod | null>(null);
  const [speaking, setSpeaking] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [overrideAcknowledged, setOverrideAcknowledged] = useState(false);

  const level = plan?.level ?? 'self_only';
  const blocked = plan?.blocked_reason ?? null;

  // Reset per-plan so a cancelled sheet cannot leak state into the next one.
  useEffect(() => {
    if (!plan) return;
    setAttendees(plan.attendees.map(name => ({ name, state: 'unknown' as const })));
    setNewAttendee('');
    setNoticeMethod(null);
    setOverrideAcknowledged(false);
    setSubmitting(false);
  }, [plan]);

  // At the affirmative level, try the calendar once if nothing was prefilled.
  useEffect(() => {
    if (!plan || plan.level !== 'affirmative' || plan.attendees.length > 0) return;
    let cancelled = false;
    prefillAttendees()
      .then(names => {
        if (cancelled || names.length === 0) return;
        setAttendees(names.map(name => ({ name, state: 'unknown' as const })));
      })
      .catch(error => console.warn('[Consent] attendee prefill failed:', error));
    return () => {
      cancelled = true;
    };
  }, [plan]);

  const confirmedCount = useMemo(
    () => attendees.filter(a => a.state === 'consented').length,
    [attendees],
  );

  const canProceed = useMemo(() => {
    if (!plan) return false;
    if (blocked && !overrideAcknowledged) return false;
    if (level === 'notify') return noticeMethod !== null;
    if (level === 'affirmative') return confirmedCount > 0;
    return true;
  }, [plan, blocked, overrideAcknowledged, level, noticeMethod, confirmedCount]);

  const copyDisclaimer = useCallback(async () => {
    if (!plan) return;
    try {
      await navigator.clipboard.writeText(plan.disclaimer_text);
      setNoticeMethod('chat_paste');
      toast.success('Disclaimer copied. Paste it into the meeting chat.');
    } catch (error) {
      console.error('[Consent] clipboard write failed:', error);
      toast.error('Could not copy the disclaimer');
    }
  }, [plan]);

  const playAnnouncement = useCallback(async () => {
    if (!plan || speaking) return;
    setSpeaking(true);
    try {
      await speakAnnouncement(plan.announcement_text);
      setNoticeMethod('spoken_announcement');
    } catch (error) {
      console.error('[Consent] announcement failed:', error);
      toast.error('Could not play the announcement', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSpeaking(false);
    }
  }, [plan, speaking]);

  const cycleAttendee = useCallback((index: number) => {
    setAttendees(current =>
      current.map((attendee, i) => {
        if (i !== index) return attendee;
        const next = STATE_CYCLE[(STATE_CYCLE.indexOf(attendee.state) + 1) % STATE_CYCLE.length];
        return { ...attendee, state: next };
      }),
    );
  }, []);

  const addAttendee = useCallback(() => {
    const name = newAttendee.trim();
    if (!name) return;
    setAttendees(current =>
      current.some(a => a.name.toLowerCase() === name.toLowerCase())
        ? current
        : [...current, { name, state: 'consented' }],
    );
    setNewAttendee('');
  }, [newAttendee]);

  const handleStart = useCallback(async () => {
    if (!plan || !canProceed || submitting) return;
    setSubmitting(true);
    try {
      await grantClearance({
        sessionId: plan.session_id,
        meetingTitle: plan.meeting_title,
        level: plan.level,
        attendees: level === 'affirmative' ? attendees : undefined,
        noticeMethod: level === 'notify' ? noticeMethod ?? 'other' : undefined,
        overrideBlock: Boolean(blocked),
        overrideReason: blocked
          ? `Operator started anyway. Reason on record: ${blocked}`
          : null,
      });
      onCleared();
    } catch (error) {
      console.error('[Consent] failed to record consent:', error);
      toast.error('Could not record consent', {
        description: error instanceof Error ? error.message : String(error),
      });
      setSubmitting(false);
    }
  }, [plan, canProceed, submitting, level, attendees, noticeMethod, blocked, onCleared]);

  if (!plan) return null;

  return (
    <Dialog open onOpenChange={open => { if (!open) onCancel(); }}>
      <DialogContent className="bg-surface border-edge text-ink max-w-lg">
        <DialogHeader>
          <DialogTitle className="font-display text-ink-bright">
            Before recording starts
          </DialogTitle>
          <DialogDescription className="text-muted-ink">
            {levelCopy(level).summary}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {plan.meeting_title && (
            <p className="text-xs text-muted-ink">
              Meeting: <span className="text-ink">{plan.meeting_title}</span>
            </p>
          )}

          {blocked && (
            <div className="rounded border border-rec/40 bg-wash p-3">
              <div className="flex items-start gap-2">
                <AlertTriangle className="mt-0.5 h-4 w-4 flex-shrink-0 text-rec" />
                <div className="space-y-2">
                  <p className="text-sm font-medium text-ink">Recording was refused</p>
                  <p className="text-xs text-muted-ink">{blocked}</p>
                  <label className="flex items-start gap-2 text-xs text-ink">
                    <input
                      type="checkbox"
                      checked={overrideAcknowledged}
                      onChange={e => setOverrideAcknowledged(e.target.checked)}
                      className="mt-0.5 accent-current"
                    />
                    <span>
                      Record anyway. This is logged as an override on this meeting&apos;s
                      consent record.
                    </span>
                  </label>
                </div>
              </div>
            </div>
          )}

          {level === 'notify' && (
            <div className="space-y-3">
              <div>
                <h3 className="section-header mb-2 text-sm">Disclaimer to paste</h3>
                <p className="rounded border border-edge bg-wash p-2 text-xs text-ink">
                  {plan.disclaimer_text}
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button type="button" variant="outline" size="sm" onClick={copyDisclaimer}>
                  <Copy className="mr-2 h-3.5 w-3.5" />
                  Copy disclaimer
                </Button>
                {plan.spoken_announcement_enabled && plan.spoken_announcement_supported && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={playAnnouncement}
                    disabled={speaking}
                  >
                    <Volume2 className="mr-2 h-3.5 w-3.5" />
                    {speaking ? 'Playing...' : 'Play announcement'}
                  </Button>
                )}
              </div>
              <div className="space-y-1">
                <Label className="text-xs text-muted-ink">How did you give notice?</Label>
                <div className="flex flex-wrap gap-1.5">
                  {(['chat_paste', 'spoken_announcement', 'verbal', 'in_person', 'other'] as ConsentMethod[]).map(
                    method => (
                      <button
                        key={method}
                        type="button"
                        onClick={() => setNoticeMethod(method)}
                        className={`rounded border px-2 py-1 text-xs transition-colors ${
                          noticeMethod === method
                            ? 'border-ink bg-ink text-app'
                            : 'border-edge bg-surface text-muted-ink hover:text-ink'
                        }`}
                      >
                        {method === 'chat_paste' && 'Pasted in chat'}
                        {method === 'spoken_announcement' && 'Played announcement'}
                        {method === 'verbal' && 'Said out loud'}
                        {method === 'in_person' && 'In person'}
                        {method === 'other' && 'Other'}
                      </button>
                    ),
                  )}
                </div>
              </div>
            </div>
          )}

          {level === 'affirmative' && (
            <div className="space-y-3">
              <div className="flex items-baseline justify-between">
                <h3 className="section-header text-sm">Attendees</h3>
                <span className="text-xs text-muted-ink">
                  {confirmedCount} of {attendees.length} confirmed
                </span>
              </div>
              <p className="text-xs text-muted-ink">
                Tap a name to cycle its state. Recording needs at least one confirmed
                attendee.
              </p>
              <ul className="max-h-48 space-y-1 overflow-y-auto">
                {attendees.map((attendee, index) => (
                  <li key={`${attendee.name}-${index}`}>
                    <button
                      type="button"
                      onClick={() => cycleAttendee(index)}
                      className="flex w-full items-center justify-between gap-2 rounded border border-edge bg-surface px-2 py-1.5 text-left hover:bg-wash"
                    >
                      <span className="truncate text-sm text-ink">{attendee.name}</span>
                      <span
                        className={`flex-shrink-0 rounded px-1.5 py-0.5 text-[11px] ${STATE_COPY[attendee.state].className}`}
                      >
                        {STATE_COPY[attendee.state].label}
                      </span>
                    </button>
                  </li>
                ))}
                {attendees.length === 0 && (
                  <li className="text-xs text-muted-ink">
                    No attendees yet. Add them below.
                  </li>
                )}
              </ul>
              <div className="flex gap-2">
                <Input
                  value={newAttendee}
                  onChange={e => setNewAttendee(e.target.value)}
                  onKeyDown={e => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      addAttendee();
                    }
                  }}
                  placeholder="Name or email"
                  className="h-8 bg-surface text-sm"
                />
                <Button type="button" variant="outline" size="sm" onClick={addAttendee}>
                  <Plus className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
          )}

          <p className="text-xs text-muted-ink">{levelCopy(level).logs}</p>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" size="sm" onClick={onCancel}>
            <X className="mr-2 h-3.5 w-3.5" />
            Do not record
          </Button>
          <Button type="button" size="sm" onClick={handleStart} disabled={!canProceed || submitting}>
            <Check className="mr-2 h-3.5 w-3.5" />
            {submitting ? 'Recording...' : 'Start recording'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
