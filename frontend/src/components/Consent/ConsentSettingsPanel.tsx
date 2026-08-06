'use client';

/**
 * Settings → Consent.
 *
 * The operator picks a default level, edits the two pieces of text that reach
 * other people (the pasteable disclaimer and the spoken announcement), maintains
 * the blocking lists, and exports the log.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { Download, Plus, Volume2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';
import {
  CONSENT_LEVELS,
  ENFORCEMENT_COPY,
  exportConsentLog,
  getConsentSettings,
  LEVEL_COPY,
  saveConsentSettings,
  speakAnnouncement,
} from '@/lib/consent';
import type { ConsentLevel, ConsentSettings, EnforcementMode } from '@/types/consent';
import { ManagedBanner } from '@/components/Fleet/ManagedBanner';

function isoDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}

interface TagEditorProps {
  label: string;
  hint: string;
  placeholder: string;
  values: string[];
  onChange: (values: string[]) => void;
}

function TagEditor({ label, hint, placeholder, values, onChange }: TagEditorProps) {
  const [draft, setDraft] = useState('');

  const add = useCallback(() => {
    const entry = draft.trim();
    if (!entry) return;
    if (!values.some(v => v.toLowerCase() === entry.toLowerCase())) {
      onChange([...values, entry]);
    }
    setDraft('');
  }, [draft, values, onChange]);

  return (
    <div className="space-y-2">
      <div>
        <Label className="text-ink">{label}</Label>
        <p className="text-xs text-muted-ink">{hint}</p>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {values.map(value => (
          <span
            key={value}
            className="inline-flex items-center gap-1 rounded border border-edge bg-wash px-2 py-0.5 text-xs text-ink"
          >
            {value}
            <button
              type="button"
              onClick={() => onChange(values.filter(v => v !== value))}
              className="text-faint transition-colors hover:text-ink"
              aria-label={`Remove ${value}`}
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        {values.length === 0 && (
          <span className="text-xs text-faint">Nothing on this list.</span>
        )}
      </div>
      <div className="flex gap-2">
        <Input
          value={draft}
          onChange={e => setDraft(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          placeholder={placeholder}
          className="h-8 max-w-xs bg-surface text-sm"
        />
        <Button type="button" variant="outline" size="sm" onClick={add}>
          <Plus className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

export function ConsentSettingsPanel() {
  const [settings, setSettings] = useState<ConsentSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const [exporting, setExporting] = useState(false);
  const today = useMemo(() => isoDate(new Date()), []);
  const [from, setFrom] = useState(() => {
    const start = new Date();
    start.setMonth(start.getMonth() - 3);
    return isoDate(start);
  });
  const [to, setTo] = useState(today);

  useEffect(() => {
    getConsentSettings()
      .then(setSettings)
      .catch(error => {
        console.error('Failed to load consent settings:', error);
        toast.error('Could not load consent settings');
      });
  }, []);

  const persist = useCallback(
    async (next: ConsentSettings) => {
      const previous = settings;
      setSettings(next);
      setSaving(true);
      try {
        const saved = await saveConsentSettings({
          consent_level: next.consent_level,
          per_speaker_enforcement: next.per_speaker_enforcement,
          spoken_announcement_enabled: next.spoken_announcement_enabled,
          announcement_text: next.announcement_text,
          disclaimer_text: next.disclaimer_text,
          blocked_title_keywords: next.blocked_title_keywords,
          blocked_domains: next.blocked_domains,
        });
        setSettings(saved);
      } catch (error) {
        console.error('Failed to save consent settings:', error);
        toast.error('Could not save consent settings');
        if (previous) setSettings(previous);
      } finally {
        setSaving(false);
      }
    },
    [settings],
  );

  const update = useCallback(
    (patch: Partial<ConsentSettings>) => {
      if (!settings) return;
      void persist({ ...settings, ...patch });
    },
    [settings, persist],
  );

  const testAnnouncement = useCallback(async () => {
    if (!settings || speaking) return;
    setSpeaking(true);
    try {
      await speakAnnouncement(settings.announcement_text);
    } catch (error) {
      console.error('Announcement test failed:', error);
      toast.error('Could not play the announcement', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSpeaking(false);
    }
  }, [settings, speaking]);

  const runExport = useCallback(async () => {
    if (exporting) return;
    setExporting(true);
    try {
      const result = await exportConsentLog({ from, to });
      toast.success(`Exported ${result.events} consent event(s)`, {
        description: result.folder,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message !== 'cancelled') {
        console.error('Consent log export failed:', error);
        toast.error('Could not export the consent log', { description: message });
      }
    } finally {
      setExporting(false);
    }
  }, [exporting, from, to]);

  if (!settings) {
    return <p className="mt-6 text-sm text-muted-ink">Loading consent settings...</p>;
  }

  return (
    <div className="mt-6 space-y-8">
      {/* Consent is the setting an organisation is most likely to manage, so say so
          before the operator wonders why a control will not move. */}
      <ManagedBanner
        keys={[
          'consent_level_floor',
          'consent_enforcement',
          'blocked_title_keywords',
          'blocked_domains',
        ]}
      />

      <section className="space-y-3">
        <div>
          <h2 className="section-header text-base">Default consent level</h2>
          <p className="mt-2 text-xs text-muted-ink">
            Applies to every recording unless you change it for one meeting before
            starting. Nobody in the meeting sees anything the app does not explicitly
            send or play.
          </p>
        </div>
        <div className="grid gap-2">
          {CONSENT_LEVELS.map(level => {
            const active = settings.consent_level === level;
            return (
              <button
                key={level}
                type="button"
                onClick={() => update({ consent_level: level })}
                disabled={saving}
                className={`rounded border p-3 text-left transition-colors ${
                  active
                    ? 'border-ink bg-active'
                    : 'border-edge bg-surface hover:bg-wash'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span
                    className={`h-2.5 w-2.5 flex-shrink-0 rounded-full border ${
                      active ? 'border-ink bg-ink' : 'border-edge'
                    }`}
                  />
                  <span className="text-sm font-medium text-ink-bright">
                    {LEVEL_COPY[level].label}
                  </span>
                  {active && <span className="status-chip ml-auto">Default</span>}
                </div>
                <p className="mt-1.5 pl-[18px] text-xs text-muted-ink">
                  {LEVEL_COPY[level].summary}
                </p>
                <p className="mt-1 pl-[18px] text-xs text-faint">
                  {LEVEL_COPY[level].logs}
                </p>
              </button>
            );
          })}
        </div>

        {settings.consent_level === 'per_speaker' && (
          <div className="rounded border border-edge bg-wash p-3">
            <Label className="text-ink">Unconfirmed speakers</Label>
            <div className="mt-2 grid gap-2">
              {(['flag_only', 'strict'] as EnforcementMode[]).map(mode => {
                const active = settings.per_speaker_enforcement === mode;
                return (
                  <button
                    key={mode}
                    type="button"
                    onClick={() => update({ per_speaker_enforcement: mode })}
                    disabled={saving}
                    className={`rounded border p-2 text-left transition-colors ${
                      active ? 'border-ink bg-surface' : 'border-edge bg-surface hover:bg-active'
                    }`}
                  >
                    <span className="text-sm text-ink-bright">
                      {ENFORCEMENT_COPY[mode].label}
                    </span>
                    <p className="mt-0.5 text-xs text-muted-ink">
                      {ENFORCEMENT_COPY[mode].summary}
                    </p>
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </section>

      <section className="space-y-3 border-t border-edge pt-6">
        <div>
          <h2 className="section-header text-base">What other people hear and read</h2>
          <p className="mt-2 text-xs text-muted-ink">
            The disclaimer is the one that works everywhere: you paste it into the
            meeting chat. The announcement plays out loud through your current output
            device, so people on the call hear it.
          </p>
        </div>

        <div className="space-y-1.5">
          <Label className="text-ink">Chat disclaimer</Label>
          <Textarea
            value={settings.disclaimer_text}
            onChange={e => setSettings({ ...settings, disclaimer_text: e.target.value })}
            onBlur={() => update({ disclaimer_text: settings.disclaimer_text })}
            rows={2}
            className="bg-surface text-sm"
          />
        </div>

        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <Label className="text-ink">Spoken announcement</Label>
            {settings.spoken_announcement_supported ? (
              <Switch
                checked={settings.spoken_announcement_enabled}
                onCheckedChange={value => update({ spoken_announcement_enabled: value })}
              />
            ) : (
              <span className="text-xs text-faint">Not available on this platform</span>
            )}
          </div>
          <Textarea
            value={settings.announcement_text}
            onChange={e => setSettings({ ...settings, announcement_text: e.target.value })}
            onBlur={() => update({ announcement_text: settings.announcement_text })}
            rows={2}
            className="bg-surface text-sm"
          />
          {settings.spoken_announcement_supported && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={testAnnouncement}
              disabled={speaking}
            >
              <Volume2 className="mr-2 h-3.5 w-3.5" />
              {speaking ? 'Playing...' : 'Test'}
            </Button>
          )}
        </div>
      </section>

      <section className="space-y-4 border-t border-edge pt-6">
        <div>
          <h2 className="section-header text-base">Meetings that will not record</h2>
          <p className="mt-2 text-xs text-muted-ink">
            Before recording starts, the meeting title is checked against these words
            and the attendee list against these email domains. A match refuses the
            recording and logs why. You can still override it for one meeting, which is
            also logged.
          </p>
        </div>
        <TagEditor
          label="Blocked words in the meeting title"
          hint="Matched as whole words, so 'HR' will not trip on 'Thursday'."
          placeholder="e.g. disciplinary"
          values={settings.blocked_title_keywords}
          onChange={values => update({ blocked_title_keywords: values })}
        />
        <TagEditor
          label="Blocked attendee email domains"
          hint="Subdomains count, so 'clientlegal.com' also covers 'mail.clientlegal.com'."
          placeholder="e.g. clientlegal.com"
          values={settings.blocked_domains}
          onChange={values => update({ blocked_domains: values })}
        />
      </section>

      <section className="space-y-3 border-t border-edge pt-6">
        <div>
          <h2 className="section-header text-base">Export the consent log</h2>
          <p className="mt-2 text-xs text-muted-ink">
            Writes every consent event in the range to a folder you pick, as a
            spreadsheet-ready CSV and a readable Markdown file. The log itself is
            append-only: corrections are new entries, nothing is ever edited away.
          </p>
        </div>
        <div className="flex flex-wrap items-end gap-3">
          <div className="space-y-1">
            <Label className="text-xs text-muted-ink">From</Label>
            <Input
              type="date"
              value={from}
              max={to}
              onChange={e => setFrom(e.target.value)}
              className="h-8 w-40 bg-surface text-sm"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-xs text-muted-ink">To</Label>
            <Input
              type="date"
              value={to}
              min={from}
              onChange={e => setTo(e.target.value)}
              className="h-8 w-40 bg-surface text-sm"
            />
          </div>
          <Button type="button" variant="outline" size="sm" onClick={runExport} disabled={exporting}>
            <Download className="mr-2 h-3.5 w-3.5" />
            {exporting ? 'Exporting...' : 'Export CSV and Markdown'}
          </Button>
        </div>
      </section>
    </div>
  );
}
