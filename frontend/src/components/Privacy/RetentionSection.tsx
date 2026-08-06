'use client';

/**
 * Settings → Privacy profiles → Retention.
 *
 * The destructive corner of the feature, so the UI is built around not
 * surprising anyone: dry run ships on, the preview shows exactly which meetings
 * are in range, turning dry run off takes a confirmation, and running the sweep
 * by hand takes another one.
 */

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { AlertTriangle, Clock, Play, RotateCcw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import {
  getRetentionSettings,
  retentionPreview,
  retentionRunNow,
  setRetentionDryRun,
} from '@/lib/privacy';
import type { PurgeCandidate, RetentionRunResult, RetentionSettings } from '@/types/privacy';

function formatDate(value: string | null): string {
  if (!value) return 'never';
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

function whenLine(candidate: PurgeCandidate): string {
  if (candidate.days_until_purge < 0) {
    return `past its window by ${Math.abs(candidate.days_until_purge)} day${
      Math.abs(candidate.days_until_purge) === 1 ? '' : 's'
    }`;
  }
  if (candidate.days_until_purge === 0) return 'in range today';
  return `in ${candidate.days_until_purge} day${candidate.days_until_purge === 1 ? '' : 's'}`;
}

export function RetentionSection() {
  const [settings, setSettings] = useState<RetentionSettings | null>(null);
  const [candidates, setCandidates] = useState<PurgeCandidate[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [confirmRun, setConfirmRun] = useState(false);
  const [lastResult, setLastResult] = useState<RetentionRunResult | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [loaded, preview] = await Promise.all([getRetentionSettings(), retentionPreview()]);
      setSettings(loaded);
      setCandidates(preview);
    } catch (error) {
      console.error('Failed to load retention settings:', error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const toggleDryRun = useCallback(
    async (nextDryRun: boolean) => {
      setBusy(true);
      try {
        // Turning dry run off is the moment purges become real, so the command
        // requires an explicit confirmation flag alongside the value.
        setSettings(await setRetentionDryRun(nextDryRun, true));
      } catch (error) {
        console.error('Failed to change the dry-run setting:', error);
        toast.error('Could not change the dry-run setting', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const runNow = useCallback(
    async (confirm: boolean) => {
      setBusy(true);
      setConfirmRun(false);
      try {
        const result = await retentionRunNow(confirm);
        setLastResult(result);
        await refresh();
      } catch (error) {
        console.error('Retention run failed:', error);
        toast.error('The retention run failed', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  const dryRun = settings?.dry_run ?? true;
  const overdue = candidates.filter(candidate => candidate.days_until_purge < 0);

  return (
    <section className="mt-8 pt-6 border-t border-edge">
      <div className="flex items-center gap-3 mb-1">
        <Clock className="w-5 h-5 text-muted-ink" />
        <h3 className="text-lg font-display font-semibold text-ink">Retention</h3>
      </div>
      <p className="text-sm text-muted-ink mb-4">
        Once an hour, meetings older than their profile&apos;s window are purged: recording files,
        transcript, summary, extracted facts, and action items. The meeting keeps its title and
        date, and the consent log is never touched.
      </p>

      <div className="bg-surface border border-edge rounded-lg p-4 space-y-3">
        <div className="flex items-start gap-3">
          <Switch
            checked={dryRun}
            disabled={busy || loading}
            onCheckedChange={checked => void toggleDryRun(checked)}
            aria-label="Dry run"
          />
          <div>
            <Label className="text-ink">Dry run</Label>
            <p className="text-xs text-muted-ink leading-snug">
              {dryRun
                ? 'On. The hourly sweep writes what it would remove into the log and deletes nothing.'
                : 'Off. The hourly sweep deletes the meetings listed below once they pass their window.'}
            </p>
          </div>
        </div>

        {!dryRun && (
          <div className="flex items-start gap-2 bg-wash border border-edge rounded-md px-3 py-2">
            <AlertTriangle className="w-4 h-4 text-muted-ink mt-0.5 shrink-0" />
            <p className="text-xs text-ink leading-snug">
              Deletions are permanent. Recording files are removed from disk, and transcripts and
              summaries are removed from the database.
            </p>
          </div>
        )}

        <div className="text-[11px] text-faint">
          Last sweep: {formatDate(settings?.last_run_at ?? null)}
          {settings?.armed_at ? ` · dry run turned off ${formatDate(settings.armed_at)}` : ''}
        </div>
      </div>

      {/* Preview */}
      <div className="mt-4">
        <div className="flex items-center justify-between mb-2">
          <Label className="text-ink">Next 30 days</Label>
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => void refresh()}
            title="Refresh the preview"
            aria-label="Refresh the preview"
          >
            <RotateCcw className="w-3.5 h-3.5 text-muted-ink" />
          </Button>
        </div>
        {loading ? (
          <div className="text-sm text-faint py-4">Loading…</div>
        ) : candidates.length === 0 ? (
          <div className="text-sm text-faint py-4 text-center border border-dashed border-edge rounded-lg bg-surface">
            Nothing is due in the next 30 days. Only clients whose profile sets a window are ever in
            range.
          </div>
        ) : (
          <div className="space-y-1.5">
            {candidates.map(candidate => (
              <div
                key={candidate.meeting_id}
                className="flex items-center gap-3 bg-surface border border-edge rounded-lg px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <div className="text-sm text-ink truncate">{candidate.title}</div>
                  <div className="text-[11px] text-muted-ink truncate">
                    {candidate.client_name ? `${candidate.client_name} · ` : ''}
                    {candidate.profile_name} · {candidate.retention_days} day window ·{' '}
                    {candidate.age_days} days old
                  </div>
                </div>
                <span className="status-chip whitespace-nowrap">{whenLine(candidate)}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Run now */}
      <div className="mt-4 flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => void runNow(false)}
          title="Run the sweep as a preview"
        >
          <Play className="w-3.5 h-3.5" />
          <span>Run as preview</span>
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy || dryRun || overdue.length === 0}
          onClick={() => setConfirmRun(true)}
          title={
            dryRun
              ? 'Turn dry run off first'
              : overdue.length === 0
                ? 'Nothing is past its window'
                : 'Purge the meetings past their window now'
          }
        >
          <span>Purge {overdue.length} now</span>
        </Button>
      </div>

      {confirmRun && (
        <div className="mt-3 bg-surface border border-edge rounded-lg p-4 space-y-3">
          <p className="text-sm text-ink">
            Purge {overdue.length} meeting{overdue.length === 1 ? '' : 's'} now? Recording files are
            deleted from disk and transcripts, summaries, facts, and action items are removed. This
            cannot be undone.
          </p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => setConfirmRun(false)}>
              Cancel
            </Button>
            <Button variant="outline" size="sm" onClick={() => void runNow(true)}>
              Purge now
            </Button>
          </div>
        </div>
      )}

      {lastResult && (
        <div className="mt-3 bg-surface border border-edge rounded-lg p-3">
          <div className="text-sm text-ink">
            {lastResult.dry_run ? 'Preview' : 'Purge'} finished: {lastResult.purged.length} meeting
            {lastResult.purged.length === 1 ? '' : 's'}{' '}
            {lastResult.dry_run ? 'would be purged' : 'purged'}.
          </div>
          {lastResult.refused_reason && (
            <div className="text-[11px] text-muted-ink mt-1">{lastResult.refused_reason}</div>
          )}
          {lastResult.purged.length > 0 && (
            <ul className="mt-2 space-y-0.5">
              {lastResult.purged.map(outcome => (
                <li key={outcome.meeting_id} className="text-[11px] text-muted-ink">
                  {outcome.title}: {outcome.files_removed} recording file
                  {outcome.files_removed === 1 ? '' : 's'}, {outcome.transcripts_removed} transcript
                  row{outcome.transcripts_removed === 1 ? '' : 's'}, {outcome.facts_removed} fact
                  {outcome.facts_removed === 1 ? '' : 's'}, {outcome.action_items_removed} action
                  item{outcome.action_items_removed === 1 ? '' : 's'}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
