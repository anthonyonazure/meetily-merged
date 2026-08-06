'use client';

/**
 * The chip on meeting details showing which privacy profile governed this
 * meeting. Clicking it explains what that profile allowed and what it blocked,
 * in mechanics, so the answer to "why was cloud transcription refused here?" is
 * one click away.
 */

import { useCallback, useEffect, useState } from 'react';
import { ShieldCheck, ShieldOff } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { meetingPrivacyProfile, profileEffects, SOURCE_COPY } from '@/lib/privacy';
import type { MeetingProfileView } from '@/types/privacy';

interface MeetingProfileChipProps {
  meetingId: string;
  /** Bump to refetch, e.g. after the client tag changes. */
  refreshKey?: number;
}

export function MeetingProfileChip({ meetingId, refreshKey = 0 }: MeetingProfileChipProps) {
  const [view, setView] = useState<MeetingProfileView | null>(null);

  const load = useCallback(async () => {
    try {
      setView(await meetingPrivacyProfile(meetingId));
    } catch (error) {
      console.error('Failed to load the meeting privacy profile:', error);
      setView(null);
    }
  }, [meetingId]);

  useEffect(() => {
    void load();
  }, [load, refreshKey]);

  if (!view) return null;

  const profile = view.profile;
  const effects = profile ? profileEffects(profile) : { allows: [], blocks: [] };
  const Icon = profile ? ShieldCheck : ShieldOff;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md border border-edge bg-wash text-xs text-ink hover:bg-active transition-colors"
          title={view.summary}
        >
          <Icon className="w-3.5 h-3.5 text-muted-ink" />
          <span className="truncate max-w-[160px]">
            {profile ? profile.name : 'No profile'}
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-3 space-y-3">
        <div>
          <div className="text-sm font-medium text-ink">
            {profile ? profile.name : 'No privacy profile'}
          </div>
          <div className="text-[11px] text-muted-ink">
            {profile
              ? `Applied ${SOURCE_COPY[view.source] ?? ''}${
                  view.client_name ? ` (${view.client_name})` : ''
                }`
              : SOURCE_COPY.none}
          </div>
        </div>

        {profile && profile.description ? (
          <p className="text-xs text-muted-ink">{profile.description}</p>
        ) : null}

        {effects.allows.length > 0 && (
          <div>
            <div className="text-[11px] font-medium text-muted-ink mb-1">Allowed</div>
            <ul className="space-y-1">
              {effects.allows.map(line => (
                <li key={line} className="text-xs text-ink leading-snug">
                  {line}
                </li>
              ))}
            </ul>
          </div>
        )}

        {effects.blocks.length > 0 && (
          <div>
            <div className="text-[11px] font-medium text-muted-ink mb-1">Blocked</div>
            <ul className="space-y-1">
              {effects.blocks.map(line => (
                <li key={line} className="text-xs text-ink leading-snug">
                  {line}
                </li>
              ))}
            </ul>
          </div>
        )}

        <p className="text-[11px] text-faint border-t border-edge pt-2">
          Change this in Settings → Privacy profiles, or by tagging the meeting with a different
          client.
        </p>
      </PopoverContent>
    </Popover>
  );
}
