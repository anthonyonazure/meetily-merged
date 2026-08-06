'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { AlertTriangle, Check, Loader2, RefreshCw, Search } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  getIndexStatus,
  reindexEverything,
  setEmbeddingsSettings,
} from '@/lib/embeddings';
import { IndexStatus, ModelDownloadProgress, ReindexProgress } from '@/types/embeddings';

const TOP_K_CHOICES = [6, 12, 20, 30];

export function SemanticSearchSettings() {
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [download, setDownload] = useState<ModelDownloadProgress | null>(null);
  const [reindex, setReindex] = useState<ReindexProgress | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await getIndexStatus());
    } catch (error) {
      console.error('Failed to read the search index status:', error);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The model download and the reindex both run in the background and report
  // progress through events; the poll is only a fallback for a missed event.
  useEffect(() => {
    const unlistenDownload = listen<ModelDownloadProgress>(
      'embeddings-model-download-progress',
      event => {
        setDownload(event.payload);
        if (event.payload.status === 'complete') {
          setDownload(null);
          void refresh();
        }
      },
    );
    const unlistenReindex = listen<ReindexProgress>('embeddings-reindex-progress', event => {
      setReindex(event.payload);
      if (event.payload.phase === 'complete' || event.payload.phase === 'failed') {
        void refresh();
      }
    });
    return () => {
      void unlistenDownload.then(un => un());
      void unlistenReindex.then(un => un());
    };
  }, [refresh]);

  useEffect(() => {
    const downloading = download !== null;
    const indexing = reindex !== null && reindex.phase === 'indexing';
    if (!downloading && !indexing) {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      return;
    }
    if (!pollRef.current) {
      pollRef.current = setInterval(() => void refresh(), 4000);
    }
    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, [download, reindex, refresh]);

  const save = useCallback(
    async (next: { enabled?: boolean; topK?: number }) => {
      if (!status || busy) return;
      setBusy(true);
      try {
        const saved = await setEmbeddingsSettings({
          enabled: next.enabled ?? status.settings.enabled,
          topK: next.topK ?? status.settings.top_k,
        });
        if (saved.enabled && !status.model_downloaded) {
          toast.success('Downloading the search model', {
            description: `About ${status.download_size_mb} MB, once. Search keeps working on word matching until it lands.`,
          });
        }
        await refresh();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        toast.error('Could not save the search setting', { description: message });
      } finally {
        setBusy(false);
      }
    },
    [busy, refresh, status],
  );

  const runReindex = useCallback(async () => {
    try {
      await reindexEverything();
      setReindex({
        phase: 'preparing',
        meetings_done: 0,
        meetings_total: 0,
        passages_written: 0,
        current_meeting_title: null,
        error: null,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error('Could not start the reindex', { description: message });
    }
  }, []);

  if (!status) {
    return (
      <div className="flex items-center gap-2 py-8 text-muted-ink">
        <Loader2 className="w-4 h-4 animate-spin" />
        Reading the search index…
      </div>
    );
  }

  const { counts, settings } = status;
  const incomplete = counts.meetings_with_transcripts - counts.meetings_indexed;
  const indexIsQuestionable =
    settings.enabled && (incomplete > 0 || counts.rows_from_other_models > 0);

  return (
    <div className="space-y-8 py-6">
      <section>
        <h2 className="section-header text-xl mb-4">Search by meaning</h2>
        <p className="text-sm text-muted-ink max-w-3xl">
          Search normally matches the words you type. With this on, it also finds meetings that
          mean the same thing in different words, so &ldquo;when did we agree that deadline&rdquo;
          finds the moment the client said &ldquo;let&rsquo;s lock the go-live for the
          14th&rdquo;. Everything runs on this machine: a {status.download_size_mb} MB model is
          downloaded once, and no text is ever sent anywhere to search it.
        </p>

        <div className="mt-5 flex items-start justify-between gap-6 rounded border border-edge bg-surface p-4">
          <div>
            <div className="font-medium text-ink">Local semantic search</div>
            <div className="text-sm text-muted-ink mt-1">
              {settings.enabled ? 'On' : 'Off'} · model {status.model_id} ·{' '}
              {status.dimensions} dimensions
            </div>
          </div>
          <Button
            variant={settings.enabled ? 'outline' : 'default'}
            disabled={busy}
            onClick={() => void save({ enabled: !settings.enabled })}
          >
            {busy && <Loader2 className="w-4 h-4 mr-2 animate-spin" />}
            {settings.enabled ? 'Turn off' : 'Turn on'}
          </Button>
        </div>
      </section>

      {download && (
        <section className="rounded border border-edge bg-wash p-4">
          <div className="flex items-center gap-2 text-sm text-ink">
            <Loader2 className="w-4 h-4 animate-spin" />
            Downloading the {download.file} — {download.progress}% (
            {download.downloaded_mb.toFixed(1)} of {download.total_mb.toFixed(1)} MB)
          </div>
          <div className="mt-3 h-1.5 w-full rounded bg-edge">
            <div
              className="h-1.5 rounded bg-ink transition-all"
              style={{ width: `${Math.min(100, Math.max(0, download.progress))}%` }}
            />
          </div>
        </section>
      )}

      <section>
        <h3 className="section-header text-lg mb-4">What is indexed</h3>
        <div
          className={`flex items-start gap-3 rounded border p-4 ${
            indexIsQuestionable ? 'border-rec bg-wash' : 'border-edge bg-surface'
          }`}
        >
          {indexIsQuestionable ? (
            <AlertTriangle className="w-4 h-4 mt-0.5 text-rec shrink-0" />
          ) : (
            <Check className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
          )}
          <p className="text-sm text-ink">{status.summary}</p>
        </div>

        <dl className="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-4">
          {[
            ['Transcript passages', counts.transcript_chunks],
            ['Summary passages', counts.summaries],
            ['Client memory facts', counts.memory_facts],
            ['Meetings covered', `${counts.meetings_indexed} of ${counts.meetings_with_transcripts}`],
          ].map(([label, value]) => (
            <div key={String(label)} className="rounded border border-edge bg-surface p-3">
              <dt className="text-xs uppercase tracking-wide text-muted-ink">{label}</dt>
              <dd className="mt-1 text-lg text-ink-bright">{value}</dd>
            </div>
          ))}
        </dl>

        {counts.last_indexed_at && (
          <p className="mt-3 text-xs text-faint">
            Last indexed {new Date(counts.last_indexed_at).toLocaleString()}.
          </p>
        )}

        <div className="mt-5 flex flex-wrap items-center gap-3">
          <Button
            variant="outline"
            disabled={
              !settings.enabled ||
              !status.model_downloaded ||
              status.reindex_running ||
              reindex?.phase === 'indexing'
            }
            onClick={() => void runReindex()}
          >
            {status.reindex_running || reindex?.phase === 'indexing' ? (
              <Loader2 className="w-4 h-4 mr-2 animate-spin" />
            ) : (
              <RefreshCw className="w-4 h-4 mr-2" />
            )}
            Index everything recorded so far
          </Button>
          {reindex && reindex.phase === 'indexing' && (
            <span className="text-sm text-muted-ink">
              {reindex.meetings_done} of {reindex.meetings_total} meetings ·{' '}
              {reindex.passages_written} passages
              {reindex.current_meeting_title ? ` · ${reindex.current_meeting_title}` : ''}
            </span>
          )}
          {reindex?.phase === 'failed' && (
            <span className="text-sm text-rec">{reindex.error ?? 'The reindex failed.'}</span>
          )}
        </div>
        <p className="mt-3 text-xs text-muted-ink max-w-3xl">
          New meetings are indexed automatically once their summary finishes. Use this for
          meetings recorded before you turned the feature on, or if the index looks wrong.
          Speech from a speaker who has not confirmed consent under strict per-speaker rules is
          left out of the index entirely, and comes back only after they confirm and you index
          again.
        </p>
      </section>

      <section>
        <h3 className="section-header text-lg mb-4">How many results to use</h3>
        <p className="text-sm text-muted-ink max-w-3xl">
          How many passages a search returns, and how many the assistant is given when it
          answers a question about a large amount of recorded material.
        </p>
        <div className="mt-4 flex gap-2">
          {TOP_K_CHOICES.map(choice => (
            <Button
              key={choice}
              variant={settings.top_k === choice ? 'default' : 'outline'}
              size="sm"
              disabled={busy}
              onClick={() => void save({ topK: choice })}
            >
              {choice}
            </Button>
          ))}
        </div>
      </section>

      <section className="flex items-start gap-3 rounded border border-edge bg-wash p-4">
        <Search className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
        <p className="text-sm text-muted-ink">
          Word matching always runs, whether this is on or off. Turning it off leaves your
          existing index in place, so turning it back on does not need a fresh download or a
          fresh index.
          {status.models_dir ? ` Model files live in ${status.models_dir}.` : ''}
        </p>
      </section>
    </div>
  );
}
