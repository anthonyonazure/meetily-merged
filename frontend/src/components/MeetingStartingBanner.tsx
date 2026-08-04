'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Video, X } from 'lucide-react';

interface MeetingStartingPayload {
  title: string;
  url: string;
  start: string;
}

const AUTO_DISMISS_MS = 3 * 60 * 1000;

// In-app companion to the "meeting starting" OS notification emitted by the
// Rust autojoin scheduler. Prompt-then-open only: the meeting link opens
// exclusively via the Join button.
export function MeetingStartingBanner() {
  const [prompt, setPrompt] = useState<MeetingStartingPayload | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void (async () => {
      unlisten = await listen<MeetingStartingPayload>('calendar-meeting-starting', event => {
        setPrompt(event.payload);
      });
    })();
    return () => unlisten?.();
  }, []);

  // A stale "meeting starting" banner is worse than none.
  useEffect(() => {
    if (!prompt) return;
    const timer = setTimeout(() => setPrompt(null), AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [prompt]);

  const handleJoin = useCallback(async () => {
    if (!prompt) return;
    try {
      await invoke('open_external_url', { url: prompt.url });
      setPrompt(null);
    } catch (error) {
      toast.error('Failed to open meeting link', { description: String(error) });
    }
  }, [prompt]);

  if (!prompt) return null;

  return (
    <div className="fixed top-3 left-1/2 -translate-x-1/2 z-50 flex items-center gap-3 bg-surface border border-edge rounded-lg shadow-lg px-4 py-2.5 max-w-[70vw]">
      <Video className="w-4 h-4 text-muted-ink flex-shrink-0" />
      <span className="text-sm text-ink truncate">
        Meeting starting: <span className="font-medium">{prompt.title}</span>
      </span>
      <button
        onClick={() => void handleJoin()}
        className="px-2.5 py-1 text-xs font-medium text-blue-600 border border-blue-200 rounded hover:bg-blue-50 flex-shrink-0"
      >
        Join
      </button>
      <button
        onClick={() => setPrompt(null)}
        aria-label="Dismiss"
        className="text-faint hover:text-ink flex-shrink-0"
      >
        <X className="w-4 h-4" />
      </button>
    </div>
  );
}
