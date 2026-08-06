/** Thin typed wrappers over the semantic search commands. */

import { invoke } from '@tauri-apps/api/core';
import {
  EmbeddingsSettings,
  IndexStatus,
  SearchResults,
} from '@/types/embeddings';

export function searchSemantic(args: {
  query: string;
  meetingId?: string | null;
  clientId?: string | null;
  topK?: number | null;
}): Promise<SearchResults> {
  return invoke<SearchResults>('search_semantic', {
    query: args.query,
    meetingId: args.meetingId ?? null,
    clientId: args.clientId ?? null,
    topK: args.topK ?? null,
  });
}

export function getIndexStatus(): Promise<IndexStatus> {
  return invoke<IndexStatus>('embeddings_index_status');
}

export function reindexEverything(): Promise<boolean> {
  return invoke<boolean>('embeddings_reindex');
}

export function getEmbeddingsSettings(): Promise<EmbeddingsSettings> {
  return invoke<EmbeddingsSettings>('embeddings_settings_get');
}

export function setEmbeddingsSettings(args: {
  enabled: boolean;
  modelName?: string | null;
  topK?: number | null;
}): Promise<EmbeddingsSettings> {
  return invoke<EmbeddingsSettings>('embeddings_settings_set', {
    enabled: args.enabled,
    modelName: args.modelName ?? null,
    topK: args.topK ?? null,
  });
}

/** What each kind of indexed passage is called in the UI. */
export const SOURCE_KIND_LABEL: Record<string, string> = {
  transcript_chunk: 'Transcript',
  summary: 'Summary',
  memory_fact: 'Client memory',
};

/** How a result was found, in words rather than a score. */
export const MATCH_KIND_LABEL: Record<string, string> = {
  semantic: 'Similar meaning',
  keyword: 'Exact words',
  both: 'Exact words and similar meaning',
};
