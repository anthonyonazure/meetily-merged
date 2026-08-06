'use client';

/**
 * Settings → Privacy profiles.
 *
 * A profile is a named bundle of processing rules attached to a client, so the
 * right policy applies to their meetings without anyone remembering it. Each
 * field is described by what it does, never by what it means legally.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { Copy, Pencil, Plus, ShieldCheck, Trash2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { LEVEL_COPY, ENFORCEMENT_COPY, CONSENT_LEVELS } from '@/lib/consent';
import {
  blankProfileInput,
  createPrivacyProfile,
  deletePrivacyProfile,
  getPrivacySettings,
  listPrivacyProfiles,
  MODE_COPY,
  privacyProfileUsage,
  profileEffects,
  REDACT_COPY,
  RETENTION_COPY,
  setDefaultPrivacyProfile,
  SHARING_COPY,
  toProfileInput,
  updatePrivacyProfile,
} from '@/lib/privacy';
import type { ConsentLevel, EnforcementMode } from '@/types/consent';
import type {
  PrivacyProfile,
  PrivacyProfileInput,
  ProcessingMode,
} from '@/types/privacy';

const WORKSPACE_NONE = '__none__';
const MODES: ProcessingMode[] = ['local_only', 'cloud_allowed'];

interface EditorState {
  /** Null while creating (including a duplicate of an existing profile). */
  id: string | null;
  input: PrivacyProfileInput;
}

function ModeField({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  value: ProcessingMode;
  onChange: (mode: ProcessingMode) => void;
}) {
  return (
    <div className="space-y-1.5">
      <div>
        <Label className="text-ink">{label}</Label>
        <p className="text-xs text-muted-ink">{hint}</p>
      </div>
      <div className="grid gap-1.5 sm:grid-cols-2">
        {MODES.map(mode => {
          const active = value === mode;
          return (
            <button
              key={mode}
              type="button"
              onClick={() => onChange(mode)}
              className={`text-left rounded-lg border px-3 py-2 transition-colors ${
                active
                  ? 'border-ink bg-wash'
                  : 'border-edge bg-surface hover:bg-wash'
              }`}
            >
              <div className="text-sm font-medium text-ink">{MODE_COPY[mode].label}</div>
              <div className="text-[11px] text-muted-ink leading-snug">
                {MODE_COPY[mode].summary}
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ProfileEditor({
  editor,
  saving,
  onChange,
  onSave,
  onCancel,
}: {
  editor: EditorState;
  saving: boolean;
  onChange: (input: PrivacyProfileInput) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const { input } = editor;
  const set = <K extends keyof PrivacyProfileInput>(key: K, value: PrivacyProfileInput[K]) =>
    onChange({ ...input, [key]: value });

  return (
    <div className="mt-4 bg-surface border border-edge rounded-lg p-4 space-y-5">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium text-ink">
          {editor.id ? 'Edit profile' : 'New profile'}
        </span>
        <button
          onClick={onCancel}
          className="text-faint hover:text-muted-ink"
          title="Close"
          aria-label="Close editor"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <div>
          <Label className="text-ink">Name</Label>
          <input
            value={input.name}
            onChange={event => set('name', event.target.value)}
            placeholder="Law firms"
            autoFocus
            className="mt-1 w-full rounded-md border border-edge bg-surface px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
          />
        </div>
        <div>
          <Label className="text-ink">Description</Label>
          <input
            value={input.description}
            onChange={event => set('description', event.target.value)}
            placeholder="What this profile is for, in your words"
            className="mt-1 w-full rounded-md border border-edge bg-surface px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
          />
        </div>
      </div>

      <ModeField
        label="Transcription"
        hint="Where audio may be turned into text. A cloud provider is refused at recording start when this is set to on-device only."
        value={input.transcription_mode}
        onChange={mode => set('transcription_mode', mode)}
      />

      <ModeField
        label="Models"
        hint="Where summaries, chat answers, and agent runs may be generated."
        value={input.llm_mode}
        onChange={mode => set('llm_mode', mode)}
      />

      <div className="space-y-1.5">
        <div>
          <Label className="text-ink">Consent</Label>
          <p className="text-xs text-muted-ink">
            What happens before a recording starts for this client. This replaces the global
            consent level; an operator can pick something stricter for one meeting but not looser.
          </p>
        </div>
        <div className="space-y-1.5">
          {CONSENT_LEVELS.map(level => {
            const active = input.consent_level === level;
            return (
              <button
                key={level}
                type="button"
                onClick={() => set('consent_level', level as ConsentLevel)}
                className={`w-full text-left rounded-lg border px-3 py-2 transition-colors ${
                  active ? 'border-ink bg-wash' : 'border-edge bg-surface hover:bg-wash'
                }`}
              >
                <div className="text-sm font-medium text-ink">{LEVEL_COPY[level].label}</div>
                <div className="text-[11px] text-muted-ink leading-snug">
                  {LEVEL_COPY[level].summary}
                </div>
              </button>
            );
          })}
        </div>
      </div>

      {input.consent_level === 'per_speaker' && (
        <div className="space-y-1.5">
          <Label className="text-ink">Speakers you have not confirmed</Label>
          <div className="grid gap-1.5 sm:grid-cols-2">
            {(['flag_only', 'strict'] as EnforcementMode[]).map(mode => {
              const active = input.consent_enforcement === mode;
              return (
                <button
                  key={mode}
                  type="button"
                  onClick={() => set('consent_enforcement', mode)}
                  className={`text-left rounded-lg border px-3 py-2 transition-colors ${
                    active ? 'border-ink bg-wash' : 'border-edge bg-surface hover:bg-wash'
                  }`}
                >
                  <div className="text-sm font-medium text-ink">{ENFORCEMENT_COPY[mode].label}</div>
                  <div className="text-[11px] text-muted-ink leading-snug">
                    {ENFORCEMENT_COPY[mode].summary}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      <div className="space-y-1.5">
        <div>
          <Label className="text-ink">Retention</Label>
          <p className="text-xs text-muted-ink">
            {input.retention_days === null
              ? RETENTION_COPY.keep
              : RETENTION_COPY.window(input.retention_days)}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Switch
            checked={input.retention_days !== null}
            onCheckedChange={checked => set('retention_days', checked ? 90 : null)}
            aria-label="Remove meetings after a number of days"
          />
          <span className="text-sm text-ink">Remove old meetings</span>
          {input.retention_days !== null && (
            <>
              <input
                type="number"
                min={1}
                max={36500}
                value={input.retention_days}
                onChange={event => {
                  const parsed = Number.parseInt(event.target.value, 10);
                  set('retention_days', Number.isNaN(parsed) ? 1 : Math.max(1, parsed));
                }}
                className="w-20 rounded-md border border-edge bg-surface px-2 py-1 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
              />
              <span className="text-sm text-muted-ink">days old</span>
            </>
          )}
        </div>
      </div>

      <div className="space-y-2">
        <div className="flex items-start gap-3">
          <Switch
            checked={input.redact_pii}
            onCheckedChange={checked => set('redact_pii', checked)}
            aria-label="Mask obvious secrets"
          />
          <div>
            <div className="text-sm text-ink">Mask obvious secrets</div>
            <p className="text-[11px] text-muted-ink leading-snug">
              {input.redact_pii ? REDACT_COPY.on : REDACT_COPY.off}
            </p>
            <p className="text-[11px] text-faint leading-snug mt-0.5">{REDACT_COPY.scope}</p>
          </div>
        </div>

        <div className="flex items-start gap-3">
          <Switch
            checked={input.allow_sharing}
            onCheckedChange={checked => set('allow_sharing', checked)}
            aria-label="Allow share actions"
          />
          <div>
            <div className="text-sm text-ink">Allow Slack, Teams, and Outlook drafts</div>
            <p className="text-[11px] text-muted-ink leading-snug">
              {input.allow_sharing ? SHARING_COPY.on : SHARING_COPY.off}
            </p>
          </div>
        </div>
      </div>

      <div className="flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button variant="outline" size="sm" disabled={saving} onClick={onSave}>
          {editor.id ? 'Save changes' : 'Create profile'}
        </Button>
      </div>
    </div>
  );
}

export function PrivacyProfilesPanel() {
  const [profiles, setProfiles] = useState<PrivacyProfile[]>([]);
  const [defaultProfileId, setDefaultProfileId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<PrivacyProfile | null>(null);
  const [deleteUsage, setDeleteUsage] = useState(0);

  const refresh = useCallback(async () => {
    try {
      const [list, settings] = await Promise.all([listPrivacyProfiles(), getPrivacySettings()]);
      setProfiles(list);
      setDefaultProfileId(settings.default_profile_id);
    } catch (error) {
      console.error('Failed to load privacy profiles:', error);
      toast.error('Failed to load privacy profiles');
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleSave = useCallback(async () => {
    if (!editor) return;
    if (!editor.input.name.trim()) {
      toast.error('Give the profile a name');
      return;
    }
    setSaving(true);
    try {
      if (editor.id) {
        await updatePrivacyProfile(editor.id, editor.input);
      } else {
        await createPrivacyProfile(editor.input);
      }
      setEditor(null);
      await refresh();
    } catch (error) {
      console.error('Failed to save the privacy profile:', error);
      toast.error('Could not save the profile', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSaving(false);
    }
  }, [editor, refresh]);

  const startDelete = useCallback(async (profile: PrivacyProfile) => {
    setDeleteTarget(profile);
    try {
      setDeleteUsage(await privacyProfileUsage(profile.id));
    } catch {
      setDeleteUsage(0);
    }
  }, []);

  const handleDelete = useCallback(async () => {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleteTarget(null);
    try {
      await deletePrivacyProfile(target.id);
      await refresh();
    } catch (error) {
      console.error('Failed to delete the privacy profile:', error);
      toast.error('Could not delete the profile', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  }, [deleteTarget, refresh]);

  const handleDefaultChange = useCallback(
    async (value: string) => {
      const next = value === WORKSPACE_NONE ? null : value;
      const previous = defaultProfileId;
      setDefaultProfileId(next);
      try {
        await setDefaultPrivacyProfile(next);
      } catch (error) {
        setDefaultProfileId(previous);
        console.error('Failed to set the default privacy profile:', error);
        toast.error('Could not change the workspace default', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [defaultProfileId],
  );

  const defaultProfile = useMemo(
    () => profiles.find(profile => profile.id === defaultProfileId) ?? null,
    [profiles, defaultProfileId],
  );

  return (
    <div className="max-w-2xl py-6">
      <div className="flex items-center gap-3 mb-1">
        <ShieldCheck className="w-5 h-5 text-muted-ink" />
        <h2 className="text-xl font-display font-semibold text-ink">Privacy profiles</h2>
      </div>
      <p className="text-sm text-muted-ink mb-6">
        A profile bundles where transcription and models may run, what happens before a recording
        starts, whether summaries can be shared, and how long meetings are kept. Attach one to a
        client in Settings → Clients and it applies to their meetings by itself.
      </p>

      {/* Workspace default */}
      <div className="bg-surface border border-edge rounded-lg p-4 mb-6">
        <Label className="text-ink">Meetings with no client tag</Label>
        <p className="text-xs text-muted-ink mt-0.5 mb-2">
          Which profile governs a meeting that is not tagged with a client. Leave this on
          &ldquo;No profile&rdquo; and untagged meetings follow the app&apos;s own transcription,
          model, and consent settings, exactly as before.
        </p>
        <select
          value={defaultProfileId ?? WORKSPACE_NONE}
          onChange={event => void handleDefaultChange(event.target.value)}
          aria-label="Workspace default privacy profile"
          className="rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink focus:outline-none focus:ring-1 focus:ring-blue-300"
        >
          <option value={WORKSPACE_NONE}>No profile (use the app&apos;s settings)</option>
          {profiles.map(profile => (
            <option key={profile.id} value={profile.id}>
              {profile.name}
            </option>
          ))}
        </select>
        {defaultProfile && (
          <p className="text-[11px] text-faint mt-2">{defaultProfile.description}</p>
        )}
      </div>

      {/* Profiles */}
      {isLoading ? (
        <div className="text-sm text-faint py-6">Loading profiles…</div>
      ) : (
        <div className="space-y-2">
          {profiles.map(profile => {
            const effects = profileEffects(profile);
            return (
              <div
                key={profile.id}
                className="bg-surface border border-edge rounded-lg px-4 py-3"
              >
                <div className="flex items-center gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-ink truncate">{profile.name}</span>
                      {profile.is_builtin && <span className="status-chip">Built in</span>}
                      {profile.id === defaultProfileId && (
                        <span className="status-chip">Workspace default</span>
                      )}
                    </div>
                    <div className="text-xs text-muted-ink">
                      {MODE_COPY[profile.transcription_mode].label} transcription ·{' '}
                      {MODE_COPY[profile.llm_mode].label} models ·{' '}
                      {LEVEL_COPY[profile.consent_level].label} ·{' '}
                      {profile.retention_days === null
                        ? 'kept'
                        : `${profile.retention_days} days`}
                    </div>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    title="Edit profile"
                    aria-label={`Edit ${profile.name}`}
                    onClick={() => setEditor({ id: profile.id, input: toProfileInput(profile) })}
                  >
                    <Pencil className="w-4 h-4 text-muted-ink" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    title="Duplicate profile"
                    aria-label={`Duplicate ${profile.name}`}
                    onClick={() =>
                      setEditor({
                        id: null,
                        input: {
                          ...toProfileInput(profile),
                          name: `${profile.name} copy`,
                        },
                      })
                    }
                  >
                    <Copy className="w-4 h-4 text-muted-ink" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={profile.is_builtin}
                    title={
                      profile.is_builtin
                        ? 'Built-in profiles cannot be deleted. Duplicate it and edit the copy.'
                        : 'Delete profile'
                    }
                    aria-label={`Delete ${profile.name}`}
                    onClick={() => void startDelete(profile)}
                  >
                    <Trash2
                      className={`w-4 h-4 ${
                        profile.is_builtin ? 'text-faint' : 'text-muted-ink hover:text-red-600'
                      }`}
                    />
                  </Button>
                </div>
                {effects.blocks.length > 0 && (
                  <ul className="mt-2 space-y-0.5 border-t border-edge pt-2">
                    {effects.blocks.map(line => (
                      <li key={line} className="text-[11px] text-muted-ink leading-snug">
                        {line}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            );
          })}
        </div>
      )}

      {editor ? (
        <ProfileEditor
          editor={editor}
          saving={saving}
          onChange={input => setEditor({ ...editor, input })}
          onSave={() => void handleSave()}
          onCancel={() => setEditor(null)}
        />
      ) : (
        <Button
          variant="outline"
          size="sm"
          className="mt-4"
          onClick={() => setEditor({ id: null, input: blankProfileInput() })}
        >
          <Plus className="w-4 h-4" />
          <span>Add profile</span>
        </Button>
      )}

      <ConfirmationModal
        isOpen={deleteTarget !== null}
        text={
          deleteUsage > 0
            ? `Delete ${deleteTarget?.name ?? 'this profile'}? ${deleteUsage} client${
                deleteUsage === 1 ? '' : 's'
              } will fall back to the workspace default. No meetings are deleted.`
            : `Delete ${deleteTarget?.name ?? 'this profile'}? No meetings are deleted.`
        }
        onConfirm={() => void handleDelete()}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}
