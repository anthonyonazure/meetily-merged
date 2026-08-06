'use client';

/**
 * Per-meeting consent level override.
 *
 * Sits next to the record button so the level for *this* recording can be
 * changed without going to Settings. It resets after each recording: an override
 * is for one meeting, and a permanent change belongs in Settings where it is
 * visible.
 */

import { useEffect, useState } from 'react';
import { ChevronDown, ShieldCheck } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { CONSENT_LEVELS, getConsentSettings, levelCopy } from '@/lib/consent';
import type { ConsentLevel } from '@/types/consent';

interface ConsentLevelPickerProps {
  /** Null means "use the saved default". */
  value: ConsentLevel | null;
  onChange: (level: ConsentLevel | null) => void;
  disabled?: boolean;
}

export function ConsentLevelPicker({ value, onChange, disabled }: ConsentLevelPickerProps) {
  const [defaultLevel, setDefaultLevel] = useState<ConsentLevel | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    getConsentSettings()
      .then(settings => setDefaultLevel(settings.consent_level))
      .catch(error => console.warn('Could not read the default consent level:', error));
  }, []);

  const effective = value ?? defaultLevel;
  if (!effective) return null;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          className="flex items-center gap-1.5 rounded border border-edge bg-surface px-2 py-1 text-xs text-muted-ink transition-colors hover:text-ink disabled:opacity-50"
        >
          <ShieldCheck className="h-3.5 w-3.5" />
          <span>Consent: {levelCopy(effective).label}</span>
          {value && <span className="status-chip">this meeting</span>}
          <ChevronDown className="h-3 w-3" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80 border-edge bg-surface p-2" align="start">
        <p className="px-1 pb-2 text-xs text-muted-ink">
          Consent level for the next recording only.
        </p>
        <div className="space-y-1">
          {CONSENT_LEVELS.map(level => {
            const active = effective === level;
            return (
              <button
                key={level}
                type="button"
                onClick={() => {
                  onChange(level === defaultLevel ? null : level);
                  setOpen(false);
                }}
                className={`w-full rounded border p-2 text-left transition-colors ${
                  active ? 'border-ink bg-active' : 'border-edge bg-surface hover:bg-wash'
                }`}
              >
                <span className="text-sm text-ink-bright">{levelCopy(level).label}</span>
                {level === defaultLevel && (
                  <span className="ml-2 text-[11px] text-faint">default</span>
                )}
                <p className="mt-0.5 text-xs text-muted-ink">{levelCopy(level).summary}</p>
              </button>
            );
          })}
        </div>
        {value && (
          <button
            type="button"
            onClick={() => {
              onChange(null);
              setOpen(false);
            }}
            className="mt-2 w-full rounded px-2 py-1 text-xs text-muted-ink transition-colors hover:text-ink"
          >
            Back to the saved default
          </button>
        )}
      </PopoverContent>
    </Popover>
  );
}
