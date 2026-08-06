'use client';

/**
 * Meeting details → Consent.
 *
 * Two things: the consent record for this meeting (append-only, so it reads as a
 * timeline) and the per-speaker status list, which is where `per_speaker`
 * decisions actually get made after the diarization pass has named the speakers.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { ChevronDown, ChevronUp, ShieldCheck } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  consentLogForMeeting,
  consentRedactionState,
  consentSpeakersForMeeting,
  eventLabel,
  METHOD_COPY,
  recordConsentEvent,
} from '@/lib/consent';
import type {
  ConsentEvent,
  ConsentRedactionState,
  SpeakerConsentStatus,
} from '@/types/consent';

interface ConsentPanelProps {
  meetingId: string;
}

const STATE_CHIP: Record<string, string> = {
  consented: 'bg-ink text-app',
  declined: 'bg-rec text-app',
  unknown: 'bg-wash text-muted-ink border border-edge',
};

const STATE_LABEL: Record<string, string> = {
  consented: 'Confirmed',
  declined: 'Declined',
  unknown: 'Not asked yet',
};

function formatTime(iso: string): string {
  const parsed = new Date(iso);
  return Number.isNaN(parsed.getTime()) ? iso : parsed.toLocaleString();
}

export function ConsentPanel({ meetingId }: ConsentPanelProps) {
  const [expanded, setExpanded] = useState(false);
  const [events, setEvents] = useState<ConsentEvent[]>([]);
  const [speakers, setSpeakers] = useState<SpeakerConsentStatus[]>([]);
  const [redaction, setRedaction] = useState<ConsentRedactionState | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const meetingIdRef = useRef(meetingId);
  meetingIdRef.current = meetingId;

  const refresh = useCallback(async () => {
    const requested = meetingIdRef.current;
    try {
      const [log, speakerList, state] = await Promise.all([
        consentLogForMeeting(requested),
        consentSpeakersForMeeting(requested),
        consentRedactionState(requested),
      ]);
      if (meetingIdRef.current !== requested) return;
      setEvents(log);
      setSpeakers(speakerList);
      setRedaction(state);
    } catch (error) {
      console.error('Failed to load the consent record:', error);
    }
  }, []);

  useEffect(() => {
    setEvents([]);
    setSpeakers([]);
    void refresh();
  }, [meetingId, refresh]);

  // The diarization pass is what turns "Others" into named speakers, so a
  // finished pass is exactly when the per-speaker list becomes actionable.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen('transcript-diarized', () => {
      void refresh();
    }).then(fn => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

  const decide = useCallback(
    async (speaker: string, confirmed: boolean) => {
      setPending(speaker);
      try {
        await recordConsentEvent({
          eventType: confirmed ? 'speaker_confirmed' : 'speaker_declined',
          meetingId,
          subject: speaker,
          method: 'verbal',
          detail: confirmed
            ? 'Operator confirmed this speaker consented'
            : 'Operator recorded that this speaker did not consent',
        });
        await refresh();
      } catch (error) {
        console.error('Failed to record the speaker decision:', error);
        toast.error('Could not record that decision');
      } finally {
        setPending(null);
      }
    },
    [meetingId, refresh],
  );

  const unconfirmed = speakers.filter(s => !s.is_operator && s.state !== 'consented').length;
  const hasRecord = events.length > 0 || speakers.length > 0;

  return (
    <div className="border-t border-edge pt-4">
      <button
        type="button"
        onClick={() => setExpanded(current => !current)}
        className="flex w-full items-center gap-2 text-left"
      >
        <ShieldCheck className="h-4 w-4 text-muted-ink" />
        <span className="section-header flex-1 text-sm">Consent</span>
        {unconfirmed > 0 && (
          <span className="status-chip">{unconfirmed} unconfirmed</span>
        )}
        {redaction?.strict && <span className="status-chip">Withholding</span>}
        {expanded ? (
          <ChevronUp className="h-4 w-4 text-faint" />
        ) : (
          <ChevronDown className="h-4 w-4 text-faint" />
        )}
      </button>

      {expanded && (
        <div className="mt-3 space-y-5">
          {!hasRecord && (
            <p className="text-xs text-muted-ink">
              Nothing recorded for this meeting.
            </p>
          )}

          {speakers.length > 0 && (
            <section className="space-y-2">
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-ink">
                Speakers
              </h4>
              {redaction?.strict && (
                <p className="rounded border border-edge bg-wash p-2 text-xs text-ink">
                  Unconfirmed speakers&apos; words are being held back from summaries,
                  agents, chat, and exports. Confirming a speaker restores their text
                  everywhere.
                </p>
              )}
              <ul className="space-y-1.5">
                {speakers.map(speaker => (
                  <li
                    key={speaker.speaker}
                    className="flex items-center gap-2 rounded border border-edge bg-surface px-2 py-1.5"
                  >
                    <span className="flex-1 truncate text-sm text-ink">
                      {speaker.speaker}
                      {speaker.is_operator && (
                        <span className="ml-2 text-xs text-faint">you</span>
                      )}
                    </span>
                    <span
                      className={`flex-shrink-0 rounded px-1.5 py-0.5 text-[11px] ${STATE_CHIP[speaker.state]}`}
                    >
                      {STATE_LABEL[speaker.state]}
                    </span>
                    {!speaker.is_operator && (
                      <div className="flex flex-shrink-0 gap-1">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-6 px-2 text-xs"
                          disabled={pending === speaker.speaker}
                          onClick={() => decide(speaker.speaker, true)}
                        >
                          Confirm
                        </Button>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-6 px-2 text-xs"
                          disabled={pending === speaker.speaker}
                          onClick={() => decide(speaker.speaker, false)}
                        >
                          Declined
                        </Button>
                      </div>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          )}

          {events.length > 0 && (
            <section className="space-y-2">
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-ink">
                Record
              </h4>
              <ul className="space-y-1.5">
                {events.map(event => (
                  <li
                    key={event.id}
                    className="rounded border border-edge bg-surface px-2 py-1.5"
                  >
                    <div className="flex items-baseline justify-between gap-2">
                      <span className="text-sm text-ink">{eventLabel(event.event_type)}</span>
                      <span className="flex-shrink-0 font-mono text-[11px] text-faint">
                        {formatTime(event.created_at)}
                      </span>
                    </div>
                    {event.subject && (
                      <p className="text-xs text-ink">{event.subject}</p>
                    )}
                    {event.method && (
                      <p className="text-xs text-muted-ink">
                        {METHOD_COPY[event.method] ?? event.method}
                      </p>
                    )}
                    {event.detail && (
                      <p className="text-xs text-muted-ink">{event.detail}</p>
                    )}
                  </li>
                ))}
              </ul>
              <p className="text-xs text-faint">
                Entries are never edited or removed. A change of mind is a new entry.
              </p>
            </section>
          )}
        </div>
      )}
    </div>
  );
}
