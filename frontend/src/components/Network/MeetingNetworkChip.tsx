'use client';

import { useCallback, useEffect, useState } from 'react';
import { ChevronDown, ChevronUp, Laptop, Send } from 'lucide-react';
import {
  PURPOSE_LABEL,
  formatBytes,
  formatTimestamp,
  getMeetingNetworkReport,
} from '@/lib/network';
import { MeetingNetworkReport } from '@/types/network';

/**
 * The per-meeting answer to "did any of this leave my machine?".
 *
 * Deliberately shown for the local case too, not only the cloud one: a chip that
 * only ever appears when something went wrong teaches nothing, while one that says
 * "nothing left" every time is what makes the one time it says otherwise legible.
 */
export function MeetingNetworkChip({ meetingId }: { meetingId: string }) {
  const [report, setReport] = useState<MeetingNetworkReport | null>(null);
  const [open, setOpen] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setReport(await getMeetingNetworkReport(meetingId));
    } catch (error) {
      console.error('Failed to read this meeting network log:', error);
    }
  }, [meetingId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!report) return null;

  const leftDevice = report.audio_left_device || report.transcript_left_device;

  return (
    <div className="rounded border border-edge bg-surface">
      <button
        type="button"
        onClick={() => setOpen(value => !value)}
        className="flex w-full items-start gap-3 p-3 text-left"
      >
        {leftDevice ? (
          <Send className="w-4 h-4 mt-0.5 text-rec shrink-0" />
        ) : (
          <Laptop className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
        )}
        <span className="flex-1 text-sm text-ink">{report.verdict}</span>
        {report.events.length > 0 &&
          (open ? (
            <ChevronUp className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
          ) : (
            <ChevronDown className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
          ))}
      </button>

      {open && report.events.length > 0 && (
        <ul className="border-t border-edge divide-y divide-edge">
          {report.events.map(event => (
            <li key={event.id} className="flex flex-wrap items-baseline gap-x-3 gap-y-1 p-3 text-xs">
              <span className="text-muted-ink">{formatTimestamp(event.created_at)}</span>
              <span className="text-ink">{event.host}</span>
              <span className="text-muted-ink">
                {PURPOSE_LABEL[event.purpose] ?? event.purpose}
              </span>
              {event.carried_audio && <span className="status-chip">Audio</span>}
              {event.carried_transcript && <span className="status-chip">Transcript</span>}
              <span className="text-muted-ink">
                {formatBytes(event.bytes_out)} out / {formatBytes(event.bytes_in)} in
              </span>
              {event.profile_name && (
                <span className="text-faint">profile {event.profile_name}</span>
              )}
              {event.outcome !== 'ok' && <span className="text-rec">failed</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
