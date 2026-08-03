'use client';

import { useCallback, useEffect, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
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

type CalendarStatus =
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

// Sidebar section listing calendar events for the next 24 hours (macOS).
export function UpcomingEvents() {
  const router = useRouter();
  const { setMeetingTitle } = useTranscripts();
  const [status, setStatus] = useState<CalendarStatus>('loading');
  const [events, setEvents] = useState<CalendarEvent[]>([]);

  const loadEvents = useCallback(async () => {
    try {
      const upcoming = await invoke<CalendarEvent[]>('calendar_upcoming_events');
      setEvents(upcoming);
    } catch (error) {
      console.error('Failed to load calendar events:', error);
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
          setStatus('granted');
          void loadEvents();
        } else if (permission === 'not_determined') {
          setStatus('not_determined');
        } else {
          // denied / restricted / write_only: stay quiet, no nagging.
          setStatus('denied');
        }
      } catch {
        // Non-macOS platforms report a clear error; hide the section.
        if (!cancelled) setStatus('unsupported');
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [loadEvents]);

  // Periodic refresh while granted.
  useEffect(() => {
    if (status !== 'granted') return;
    const interval = setInterval(() => void loadEvents(), REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [status, loadEvents]);

  const handleConnect = useCallback(async () => {
    try {
      const granted = await invoke<boolean>('calendar_request_access');
      if (granted) {
        setStatus('granted');
        void loadEvents();
      } else {
        setStatus('denied');
        toast.error('Calendar access was not granted', {
          description: 'You can enable it later in System Settings > Privacy & Security > Calendars.',
        });
      }
    } catch (error) {
      console.error('Calendar access request failed:', error);
      toast.error('Calendar access request failed', { description: String(error) });
    }
  }, [loadEvents]);

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

  if (status === 'loading' || status === 'unsupported' || status === 'denied') {
    return null;
  }

  return (
    <div className="mx-3 mt-3">
      <div className="flex items-center p-3 text-lg font-semibold h-10 rounded-lg">
        <CalendarClock className="w-4 h-4 mr-2 text-muted-ink" />
        <span className="text-muted-ink">Upcoming</span>
      </div>

      {status === 'not_determined' && (
        <button
          onClick={() => void handleConnect()}
          className="mx-3 mb-1 px-2 py-1.5 text-xs text-blue-600 hover:bg-blue-50 rounded-md text-left w-[calc(100%-1.5rem)]"
        >
          Show calendar meetings…
        </button>
      )}

      {status === 'granted' && events.length === 0 && (
        <div className="mx-3 mb-1 px-2 py-1 text-xs text-faint">
          No meetings in the next 24 hours
        </div>
      )}

      {status === 'granted' &&
        events.map(event => (
          <div
            key={`${event.id}-${event.start}`}
            onClick={() => handleEventClick(event)}
            title={`Start a meeting named "${event.title}"`}
            className="group flex items-center gap-2 mx-3 px-2 py-1.5 rounded-md hover:bg-wash cursor-pointer"
          >
            <span className="text-xs text-faint w-14 flex-shrink-0">
              {formatEventTime(event)}
            </span>
            <span className="text-sm text-muted-ink truncate flex-1">{event.title}</span>
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
