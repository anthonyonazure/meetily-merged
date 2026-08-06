'use client';

/**
 * The `per_speaker` prompt.
 *
 * Speaker identity is only known once the diarization pass has run: while
 * recording, every segment is labelled "You" or "Others", which is a source, not
 * a person. So this prompt fires on `transcript-diarized`, when the app first
 * knows there were N distinct voices — and it asks about all of them at once
 * rather than firing a toast per speaker.
 *
 * Mounted app-wide, because diarization finishes whether or not the operator is
 * looking at that meeting.
 */

import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Users } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import {
  consentSpeakersForMeeting,
  getConsentSettings,
  recordConsentEvent,
} from '@/lib/consent';
import type { ConsentDecisionState } from '@/types/consent';

interface DiarizedPayload {
  meeting_id?: string;
  num_speakers?: number;
}

interface PromptState {
  meetingId: string;
  speakers: string[];
}

export function SpeakerConsentPrompt() {
  const [prompt, setPrompt] = useState<PromptState | null>(null);
  const [decisions, setDecisions] = useState<Record<string, ConsentDecisionState>>({});
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    listen<DiarizedPayload>('transcript-diarized', async event => {
      const meetingId = event.payload?.meeting_id;
      if (!meetingId) return;
      try {
        const settings = await getConsentSettings();
        if (settings.consent_level !== 'per_speaker') return;
        const speakers = await consentSpeakersForMeeting(meetingId);
        const undecided = speakers
          .filter(s => !s.is_operator && s.state === 'unknown')
          .map(s => s.speaker);
        if (undecided.length === 0 || cancelled) return;
        setDecisions(Object.fromEntries(undecided.map(s => [s, 'unknown' as const])));
        setPrompt({ meetingId, speakers: undecided });
      } catch (error) {
        console.error('[Consent] could not build the speaker prompt:', error);
      }
    }).then(fn => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const close = useCallback(() => {
    setPrompt(null);
    setDecisions({});
    setSubmitting(false);
  }, []);

  const save = useCallback(async () => {
    if (!prompt || submitting) return;
    setSubmitting(true);
    const decided = prompt.speakers.filter(s => decisions[s] !== 'unknown');
    try {
      for (const speaker of decided) {
        const confirmed = decisions[speaker] === 'consented';
        await recordConsentEvent({
          eventType: confirmed ? 'speaker_confirmed' : 'speaker_declined',
          meetingId: prompt.meetingId,
          subject: speaker,
          method: 'verbal',
          detail: confirmed
            ? 'Operator confirmed this speaker consented'
            : 'Operator recorded that this speaker did not consent',
        });
      }
      close();
    } catch (error) {
      console.error('[Consent] could not save the speaker decisions:', error);
      toast.error('Could not save those decisions');
      setSubmitting(false);
    }
  }, [prompt, submitting, decisions, close]);

  if (!prompt) return null;

  const decidedCount = prompt.speakers.filter(s => decisions[s] !== 'unknown').length;

  return (
    <Dialog open onOpenChange={open => { if (!open) close(); }}>
      <DialogContent className="max-w-md border-edge bg-surface text-ink">
        <DialogHeader>
          <DialogTitle className="font-display text-ink-bright">
            Who consented?
          </DialogTitle>
          <DialogDescription className="text-muted-ink">
            This recording has {prompt.speakers.length} voice
            {prompt.speakers.length === 1 ? '' : 's'} besides yours. Speakers you do not
            confirm stay marked, and every choice here is logged.
          </DialogDescription>
        </DialogHeader>

        <ul className="space-y-1.5">
          {prompt.speakers.map(speaker => (
            <li
              key={speaker}
              className="flex items-center gap-2 rounded border border-edge bg-surface px-2 py-1.5"
            >
              <Users className="h-3.5 w-3.5 flex-shrink-0 text-faint" />
              <span className="flex-1 truncate text-sm text-ink">{speaker}</span>
              <div className="flex flex-shrink-0 gap-1">
                {(['consented', 'declined'] as ConsentDecisionState[]).map(state => (
                  <button
                    key={state}
                    type="button"
                    onClick={() =>
                      setDecisions(current => ({
                        ...current,
                        [speaker]: current[speaker] === state ? 'unknown' : state,
                      }))
                    }
                    className={`rounded border px-2 py-0.5 text-xs transition-colors ${
                      decisions[speaker] === state
                        ? 'border-ink bg-ink text-app'
                        : 'border-edge bg-surface text-muted-ink hover:text-ink'
                    }`}
                  >
                    {state === 'consented' ? 'Consented' : 'Declined'}
                  </button>
                ))}
              </div>
            </li>
          ))}
        </ul>

        <DialogFooter>
          <Button type="button" variant="outline" size="sm" onClick={close}>
            Decide later
          </Button>
          <Button type="button" size="sm" onClick={save} disabled={decidedCount === 0 || submitting}>
            {submitting ? 'Saving...' : `Log ${decidedCount} decision${decidedCount === 1 ? '' : 's'}`}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
