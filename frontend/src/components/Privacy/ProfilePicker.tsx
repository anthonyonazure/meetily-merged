'use client';

/**
 * Per-client privacy profile picker. Used on the client record in
 * Settings → Clients and on the Clients page.
 *
 * "Workspace default" is a real choice, not a blank: it means whatever the
 * workspace default is set to, including nothing.
 */

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { ShieldCheck } from 'lucide-react';
import {
  listPrivacyProfiles,
  setClientPrivacyProfile,
} from '@/lib/privacy';
import type { PrivacyProfile } from '@/types/privacy';

const WORKSPACE_DEFAULT = '__workspace_default__';

interface ProfilePickerProps {
  clientId: string;
  clientName: string;
  profileId: string | null;
  /** Called with the new value once the change is saved. */
  onChange?: (profileId: string | null) => void;
  /** Renders the label above the control instead of inline. */
  layout?: 'inline' | 'stacked';
}

export function ProfilePicker({
  clientId,
  clientName,
  profileId,
  onChange,
  layout = 'stacked',
}: ProfilePickerProps) {
  const [profiles, setProfiles] = useState<PrivacyProfile[]>([]);
  const [value, setValue] = useState<string>(profileId ?? WORKSPACE_DEFAULT);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setValue(profileId ?? WORKSPACE_DEFAULT);
  }, [profileId]);

  useEffect(() => {
    void (async () => {
      try {
        setProfiles(await listPrivacyProfiles());
      } catch (error) {
        console.error('Failed to load privacy profiles:', error);
      }
    })();
  }, []);

  const apply = useCallback(
    async (next: string) => {
      const nextId = next === WORKSPACE_DEFAULT ? null : next;
      setSaving(true);
      const previous = value;
      setValue(next);
      try {
        await setClientPrivacyProfile(clientId, nextId);
        onChange?.(nextId);
      } catch (error) {
        setValue(previous);
        console.error('Failed to set the client privacy profile:', error);
        toast.error('Could not change the privacy profile', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setSaving(false);
      }
    },
    [clientId, onChange, value],
  );

  const selected = profiles.find(profile => profile.id === value) ?? null;

  const control = (
    <select
      value={value}
      disabled={saving}
      onChange={event => void apply(event.target.value)}
      aria-label={`Privacy profile for ${clientName}`}
      className="rounded-md border border-edge bg-surface px-2 py-1 text-xs text-ink focus:outline-none focus:ring-1 focus:ring-blue-300 disabled:opacity-50"
    >
      <option value={WORKSPACE_DEFAULT}>Workspace default</option>
      {profiles.map(profile => (
        <option key={profile.id} value={profile.id}>
          {profile.name}
        </option>
      ))}
    </select>
  );

  if (layout === 'inline') {
    return (
      <div className="flex items-center gap-2 min-w-0">
        <ShieldCheck className="w-3.5 h-3.5 text-muted-ink shrink-0" />
        {control}
      </div>
    );
  }

  return (
    <div className="space-y-1">
      <label className="block text-xs font-medium text-muted-ink">Privacy profile</label>
      {control}
      <p className="text-[11px] text-faint">
        {selected
          ? selected.description ||
            'Governs transcription, models, consent, sharing, and retention for this client\'s meetings.'
          : 'Uses whatever the workspace default is set to in Settings → Privacy profiles.'}
      </p>
    </div>
  );
}
