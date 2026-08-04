'use client';

import { useCallback, useEffect, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { CalendarClock, Video } from 'lucide-react';
import { useTranscripts } from '@/contexts/TranscriptContext';

const REFRESH_INTERVAL_MS = 5 * 60 * 1000;

interface CalendarEvent {
  id: string;
  title: string;
  start: string;
  end: string;
  organizer: string | null;
  meeting_url: string | null;
}

type EventSource = 'local' | 'm365';

interface MergedEvent extends CalendarEvent {
  source: EventSource;
}

type LocalCalendarStatus =
  | 'loading'
  | 'unsupported'
  | 'not_determined'
  | 'denied'
  | 'granted';

function formatEventTime(event: CalendarEvent): string {
  const start = new Date(event.start);
  const now = new Date();
  if (start.getTime() <= now.getTime() && new Date(event.end).getTime() > now.getTime()) {
    return 'now';
  }
  const sameDay = start.toDateString() === now.toDateString();
  const time = start.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return sameDay ? time : `${start.toLocaleDateString([], { weekday: 'short' })} ${time}`;
}

// Two sources can hold the same meeting (e.g. an M365 account also synced
// into the OS calendar). Treat "same title, same start minute" as one
// meeting: keep the local entry, but adopt the other side's meeting URL if
// the kept one has none.
function mergeEvents(local: CalendarEvent[], m365: CalendarEvent[]): MergedEvent[] {
  const merged = new Map<string, MergedEvent>();
  const keyFor = (event: CalendarEvent) =>
    `${event.title.trim().toLowerCase()}|${new Date(event.start).setSeconds(0, 0)}`;

  for (const event of local) {
    merged.set(keyFor(event), { ...event, source: 'local' });
  }
  for (const event of m365) {
    const key = keyFor(event);
    const existing = merged.get(key);
    if (!existing) {
      merged.set(key, { ...event, source: 'm365' });
    } else if (!existing.meeting_url && event.meeting_url) {
      merged.set(key, { ...existing, meeting_url: event.meeting_url });
    }
  }
  return [...merged.values()].sort((a, b) => a.start.localeCompare(b.start));
}

// Sidebar section listing calendar events for the next 24 hours. Two
// sources: the OS calendar via EventKit (macOS) and Microsoft 365 via Graph
// (any platform, when connected in Settings → Integrations).
export function UpcomingEvents() {
  const router = useRouter();
  const { setMeetingTitle } = useTranscripts();
  const [localStatus, setLocalStatus] = useState<LocalCalendarStatus>('loading');
  const [m365Connected, setM365Connected] = useState(false);
  const [localEvents, setLocalEvents] = useState<CalendarEvent[]>([]);
  const [m365Events, setM365Events] = useState<CalendarEvent[]>([]);

  const loadLocalEvents = useCallback(async () => {
    try {
      setLocalEvents(await invoke<CalendarEvent[]>('calendar_upcoming_events'));
    } catch (error) {
      console.error('Failed to load calendar events:', error);
    }
  }, []);

  const loadM365Events = useCallback(async () => {
    try {
      const status = await invoke<{ connected: boolean }>('m365_auth_status');
      setM365Connected(status.connected);
      if (!status.connected) {
        setM365Events([]);
        return;
      }
      setM365Events(await invoke<CalendarEvent[]>('m365_upcoming_events'));
    } catch (error) {
      // Token expiry etc. — log quietly; the sidebar is not the place to nag.
      console.error('Failed to load M365 calendar events:', error);
    }
  }, []);

  // Determine platform support and permission state once on mount.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const permission = await invoke<string>('calendar_permission_status');
        if (cancelled) return;
        if (permission === 'full_access') {
          setLocalStatus('granted');
          void loadLocalEvents();
        } else if (permission === 'not_determined') {
          setLocalStatus('not_determined');
        } else {
          // denied / restricted / write_only: stay quiet, no nagging.
          setLocalStatus('denied');
        }
      } catch {
        // Non-macOS platforms report a clear error; M365 may still work.
        if (!cancelled) setLocalStatus('unsupported');
      }
    })();
    void loadM365Events();
    return () => {
      cancelled = true;
    };
  }, [loadLocalEvents, loadM365Events]);

  // React immediately when the user connects M365 in settings.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void (async () => {
      unlisten = await listen('m365-connected', () => void loadM365Events());
    })();
    return () => unlisten?.();
  }, [loadM365Events]);

  // Periodic refresh of whichever sources are active.
  useEffect(() => {
    if (localStatus !== 'granted' && !m365Connected) return;
    const interval = setInterval(() => {
      if (localStatus === 'granted') void loadLocalEvents();
      void loadM365Events();
    }, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [localStatus, m365Connected, loadLocalEvents, loadM365Events]);

  const handleConnect = useCallback(async () => {
    try {
      const granted = await invoke<boolean>('calendar_request_access');
      if (granted) {
        setLocalStatus('granted');
        void loadLocalEvents();
      } else {
        setLocalStatus('denied');
        toast.error('Calendar access was not granted', {
          description: 'You can enable it later in System Settings > Privacy & Security > Calendars.',
        });
      }
    } catch (error) {
      console.error('Calendar access request failed:', error);
      toast.error('Calendar access request failed', { description: String(error) });
    }
  }, [loadLocalEvents]);

  const handleEventClick = useCallback(
    (event: CalendarEvent) => {
      // Prefill the new-meeting name and land on the recording page.
      setMeetingTitle(event.title);
      router.push('/');
    },
    [router, setMeetingTitle],
  );

  const handleJoin = useCallback(async (event: CalendarEvent, mouseEvent: ReactMouseEvent) => {
    mouseEvent.stopPropagation();
    if (!event.meeting_url) return;
    try {
      await invoke('open_external_url', { url: event.meeting_url });
    } catch (error) {
      console.error('Failed to open meeting link:', error);
      toast.error('Failed to open meeting link', { description: String(error) });
    }
  }, []);

  const localVisible = localStatus === 'granted' || localStatus === 'not_determined';
  if (!localVisible && !m365Connected) {
    return null;
  }

  const events = mergeEvents(localStatus === 'granted' ? localEvents : [], m365Events);
  const showEvents = localStatus === 'granted' || m365Connected;

  return (
    <div className="mx-3 mt-3">
      <div className="flex items-center p-3 text-lg font-semibold h-10 rounded-lg">
        <CalendarClock className="w-4 h-4 mr-2 text-muted-ink" />
        <span className="text-muted-ink">Upcoming</span>
      </div>

      {localStatus === 'not_determined' && (
        <button
          onClick={() => void handleConnect()}
          className="mx-3 mb-1 px-2 py-1.5 text-xs text-blue-600 hover:bg-blue-50 rounded-md text-left w-[calc(100%-1.5rem)]"
        >
          Show calendar meetings…
        </button>
      )}

      {showEvents && events.length === 0 && (
        <div className="mx-3 mb-1 px-2 py-1 text-xs text-faint">
          No meetings in the next 24 hours
        </div>
      )}

      {showEvents &&
        events.map(event => (
          <div
            key={`${event.id}-${event.start}`}
            onClick={() => handleEventClick(event)}
            title={`Start a meeting named "${event.title}"`}
            className="group flex items-center gap-2 mx-3 px-2 py-1.5 rounded-md hover:bg-active/60 cursor-pointer"
          >
            <span className="text-xs text-faint w-14 flex-shrink-0">
              {formatEventTime(event)}
            </span>
            <span className="text-sm text-muted-ink truncate flex-1">{event.title}</span>
            <span
              title={event.source === 'm365' ? 'From Microsoft 365' : 'From your OS calendar'}
              className="text-[9px] uppercase tracking-wide text-faint border border-edge rounded px-1 py-px flex-shrink-0"
            >
              {event.source === 'm365' ? 'M365' : 'Cal'}
            </span>
            {event.meeting_url && (
              <button
                onClick={mouseEvent => void handleJoin(event, mouseEvent)}
                title="Join meeting"
                aria-label={`Join ${event.title}`}
                className="opacity-0 group-hover:opacity-100 flex items-center gap-1 px-1.5 py-0.5 text-xs text-blue-600 border border-blue-200 rounded hover:bg-blue-50 transition-opacity flex-shrink-0"
              >
                <Video className="w-3 h-3" />
                Join
              </button>
            )}
          </div>
        ))}
    </div>
  );
}
