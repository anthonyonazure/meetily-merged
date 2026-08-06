'use client';

import { motion } from 'framer-motion';
import { listen } from '@tauri-apps/api/event';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useEffect, useState } from 'react';
import { activeConsentSession, levelCopy } from '@/lib/consent';
import type { ConsentSession } from '@/types/consent';

interface RecordingStatusBarProps {
  isPaused?: boolean;
}

export const RecordingStatusBar: React.FC<RecordingStatusBarProps> = ({ isPaused = false }) => {
  // Get recording duration from backend-synced context (in seconds)
  // Backend polls every 500ms, providing smooth updates
  const { activeDuration, isRecording } = useRecordingState();

  // Display state synced from backend
  const [displaySeconds, setDisplaySeconds] = useState(0);

  // The consent level this recording was cleared at. Shown plainly next to the
  // timer: unmistakable while recording, and absent the rest of the time.
  const [consent, setConsent] = useState<ConsentSession | null>(null);

  useEffect(() => {
    if (!isRecording) {
      setConsent(null);
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    activeConsentSession()
      .then(session => { if (!cancelled) setConsent(session); })
      .catch(error => console.warn('Could not read the active consent level:', error));

    listen<ConsentSession>('consent-session-started', event => {
      if (!cancelled) setConsent(event.payload);
    }).then(fn => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [isRecording]);

  // Sync with backend duration when it changes (handles refresh/navigation)
  useEffect(() => {
    if (activeDuration !== null) {
      // Round to nearest second to avoid decimal issues
      setDisplaySeconds(Math.floor(activeDuration));
    }
  }, [activeDuration]);

  const formatDuration = (seconds: number): string => {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: -10 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -10 }}
      transition={{ duration: 0.2 }}
      className="flex items-center gap-2 px-3 py-2 bg-wash border border-edge rounded-lg mb-2"
    >
      <div className={`w-2 h-2 rounded-full ${isPaused ? 'bg-faint' : 'bg-rec animate-pulse'}`} />
      <span className={`text-sm ${isPaused ? 'text-muted-ink' : 'text-muted-ink'}`}>
        {isPaused ? 'Paused' : 'Recording'} • <span className="font-mono">{formatDuration(displaySeconds)}</span>
      </span>
      {consent && (
        <span className="status-chip ml-auto" title={levelCopy(consent.level).summary}>
          Consent: {levelCopy(consent.level).label}
          {consent.override_confirmed ? ' (overridden)' : ''}
        </span>
      )}
    </motion.div>
  );
};
