'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AlertTriangle, Loader2, Search as SearchIcon, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { MATCH_KIND_LABEL, SOURCE_KIND_LABEL, searchSemantic } from '@/lib/embeddings';
import { SearchResults } from '@/types/embeddings';

const DEBOUNCE_MS = 350;

/**
 * Search across everything recorded with one client: their transcripts, summaries,
 * and memory facts, by words and by meaning.
 *
 * The score and how each result was found stay on screen. A search that silently
 * mixes two ranking methods is a search you cannot trust for a client conversation,
 * and this is the surface where a wrong answer gets repeated to the client.
 */
export function ClientSearchPanel({
  clientId,
  clientName,
}: {
  clientId: string;
  clientName: string;
}) {
  const router = useRouter();
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchResults | null>(null);
  const [searching, setSearching] = useState(false);

  // Reset when the selected client changes, so results never belong to the
  // previous client.
  useEffect(() => {
    setQuery('');
    setResults(null);
  }, [clientId]);

  const run = useCallback(
    async (value: string) => {
      const trimmed = value.trim();
      if (trimmed.length < 2) {
        setResults(null);
        return;
      }
      setSearching(true);
      try {
        setResults(await searchSemantic({ query: trimmed, clientId }));
      } catch (error) {
        console.error('Client search failed:', error);
        setResults(null);
      } finally {
        setSearching(false);
      }
    },
    [clientId],
  );

  useEffect(() => {
    const handle = setTimeout(() => void run(query), DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [query, run]);

  return (
    <div className="bg-surface border border-edge rounded-lg p-4 mb-6">
      <label className="flex items-center gap-2 rounded border border-edge bg-app px-3 py-2">
        <SearchIcon className="w-4 h-4 text-muted-ink shrink-0" />
        <input
          value={query}
          onChange={event => setQuery(event.target.value)}
          placeholder={`Search everything said with ${clientName}…`}
          className="flex-1 bg-transparent text-sm text-ink placeholder:text-faint outline-none"
        />
        {searching && <Loader2 className="w-4 h-4 animate-spin text-muted-ink shrink-0" />}
        {query && !searching && (
          <button
            type="button"
            onClick={() => setQuery('')}
            aria-label="Clear search"
            className="text-muted-ink hover:text-ink"
          >
            <X className="w-4 h-4" />
          </button>
        )}
      </label>

      {results && (
        <div className="mt-4 space-y-3">
          {!results.semantic_used && results.semantic_unavailable_reason && (
            <p className="flex items-start gap-2 text-xs text-muted-ink">
              <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
              <span>
                Word matches only. {results.semantic_unavailable_reason} Turn on Search by
                meaning in Settings → Search to also find different wording.
              </span>
            </p>
          )}
          {results.unindexed_meetings > 0 && (
            <p className="flex items-start gap-2 text-xs text-muted-ink">
              <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
              <span>
                {results.unindexed_meetings} meeting(s) with transcripts are not indexed yet, so
                these results are incomplete.
              </span>
            </p>
          )}

          {results.hits.length === 0 ? (
            <p className="text-sm text-faint">Nothing matched.</p>
          ) : (
            <ul className="space-y-2">
              {results.hits.map(hit => (
                <li
                  key={`${hit.source_kind}:${hit.source_id}`}
                  className="rounded border border-edge bg-app p-3"
                >
                  <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-xs">
                    <span className="status-chip">
                      {SOURCE_KIND_LABEL[hit.source_kind] ?? hit.source_kind}
                    </span>
                    <button
                      type="button"
                      onClick={() => router.push(`/meeting-details?id=${hit.meeting_id}`)}
                      className="text-ink hover:underline"
                    >
                      {hit.meeting_title}
                    </button>
                    <span className="text-muted-ink">
                      {MATCH_KIND_LABEL[hit.match_kind] ?? hit.match_kind}
                    </span>
                    <span className="text-faint" title="Higher is a closer match">
                      score {hit.score.toFixed(2)}
                      {hit.semantic_score !== null
                        ? ` (similarity ${hit.semantic_score.toFixed(2)})`
                        : ''}
                    </span>
                  </div>
                  <p className="mt-2 whitespace-pre-wrap text-sm text-ink">{hit.text}</p>
                </li>
              ))}
            </ul>
          )}

          {results.hits.length > 0 && (
            <Button variant="ghost" size="sm" onClick={() => setQuery('')}>
              Clear results
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
