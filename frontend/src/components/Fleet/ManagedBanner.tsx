'use client';

import { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, Building2, Lock, RefreshCw } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { MANAGED_KEY_LABEL, getManagedConfig, reloadManagedConfig } from '@/lib/fleet';
import { ManagedKey, ManagedState } from '@/types/fleet';

/**
 * Shown above any settings panel a managed configuration touches, so a technician
 * who cannot change a control knows why rather than assuming the app is broken.
 *
 * `keys` narrows the banner to the settings this panel actually shows: a Consent
 * panel should not announce a locked retention window.
 */
export function ManagedBanner({ keys }: { keys: ManagedKey[] }) {
  const [state, setState] = useState<ManagedState | null>(null);
  const [reloading, setReloading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getManagedConfig()
      .then(next => {
        if (!cancelled) setState(next);
      })
      .catch(error => console.error('Failed to read the managed configuration:', error));
    return () => {
      cancelled = true;
    };
  }, []);

  // So an administrator who has just pushed a change does not have to make the
  // technician restart the app.
  const reload = useCallback(async () => {
    if (reloading) return;
    setReloading(true);
    try {
      const next = await reloadManagedConfig();
      setState(next);
      toast.success(
        next.found
          ? 'Re-read the settings your organisation pushed'
          : 'No managed settings file found; your own settings are in force',
        { description: next.path },
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error('Could not re-read the managed settings', { description: message });
    } finally {
      setReloading(false);
    }
  }, [reloading]);

  const reloadButton = (
    <Button variant="ghost" size="sm" disabled={reloading} onClick={() => void reload()}>
      <RefreshCw className={`w-3.5 h-3.5 mr-1.5 ${reloading ? 'animate-spin' : ''}`} />
      Check again
    </Button>
  );

  if (!state) return null;

  if (state.error) {
    return (
      <div className="mb-6 flex items-start gap-3 rounded border border-rec bg-wash p-4">
        <AlertTriangle className="w-4 h-4 mt-0.5 text-rec shrink-0" />
        <div className="text-sm">
          <p className="text-ink">
            Your organisation left a settings file this app could not read, so your own settings
            are in force.
          </p>
          <p className="mt-1 text-muted-ink">
            {state.error} File location: {state.path}
          </p>
          <div className="mt-2">{reloadButton}</div>
        </div>
      </div>
    );
  }

  const relevant = keys.filter(key => describes(state, key) !== null);
  if (relevant.length === 0) return null;

  const locked = relevant.filter(key => state.config.locked.includes(key));

  return (
    <div className="mb-6 rounded border border-edge bg-wash p-4">
      <div className="flex items-start gap-3">
        <Building2 className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
        <div className="text-sm">
          <p className="text-ink">Some of these settings are managed by your organisation.</p>
          <ul className="mt-2 space-y-1 text-muted-ink">
            {relevant.map(key => (
              <li key={key} className="flex items-start gap-2">
                {state.config.locked.includes(key) ? (
                  <Lock className="w-3 h-3 mt-1 shrink-0" />
                ) : (
                  <span className="mt-1 w-3 shrink-0" />
                )}
                <span>
                  <span className="text-ink">{MANAGED_KEY_LABEL[key]}:</span>{' '}
                  {describes(state, key)}
                  {state.config.locked.includes(key)
                    ? ' — you cannot change this here.'
                    : ' — you can make it stricter, but not looser.'}
                </span>
              </li>
            ))}
          </ul>
          {locked.length === 0 && (
            <p className="mt-2 text-xs text-faint">
              None of these are locked, so a stricter choice of your own still wins.
            </p>
          )}
          {state.config.warnings.length > 0 && (
            <div className="mt-3 border-t border-edge pt-3">
              <p className="text-ink">Parts of that file could not be used:</p>
              <ul className="mt-1 list-disc pl-5 text-muted-ink">
                {state.config.warnings.map(warning => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            </div>
          )}
          <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-edge pt-3">
            <span className="text-xs text-faint break-all">From {state.path}</span>
            {reloadButton}
          </div>
        </div>
      </div>
    </div>
  );
}

/** What the organisation set this key to, in words, or null when it set nothing. */
function describes(state: ManagedState, key: ManagedKey): string | null {
  const config = state.config;
  switch (key) {
    case 'default_privacy_profile':
      return config.default_privacy_profile;
    case 'consent_level_floor':
      return config.consent_level_floor ? CONSENT_LEVEL_COPY[config.consent_level_floor] : null;
    case 'consent_enforcement':
      return config.consent_enforcement === 'strict'
        ? 'unconfirmed speakers are withheld'
        : config.consent_enforcement === 'flag_only'
          ? 'unconfirmed speakers are only flagged'
          : null;
    case 'blocked_title_keywords':
      return config.blocked_title_keywords
        ? `${config.blocked_title_keywords.length} word(s) that block a recording`
        : null;
    case 'blocked_domains':
      return config.blocked_domains
        ? `${config.blocked_domains.length} attendee domain(s) that block a recording`
        : null;
    case 'retention_days':
      return config.retention_days !== null
        ? `at most ${config.retention_days} day(s)`
        : null;
    case 'allowed_transcription_providers':
      return config.allowed_transcription_providers
        ? providerList(config.allowed_transcription_providers)
        : null;
    case 'allowed_llm_providers':
      return config.allowed_llm_providers ? providerList(config.allowed_llm_providers) : null;
    case 'updates_enabled':
      return config.updates_enabled === null
        ? null
        : config.updates_enabled
          ? 'allowed'
          : 'turned off';
    default:
      return null;
  }
}

function providerList(list: string[]): string {
  return list.length === 0 ? 'none permitted' : list.join(', ');
}

const CONSENT_LEVEL_COPY: Record<string, string> = {
  self_only: 'you consent for yourself',
  notify: 'the room has to be told',
  affirmative: 'every named attendee has to be ticked off',
  per_speaker: 'every speaker has to be confirmed individually',
};
