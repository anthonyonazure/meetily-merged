/** Local semantic search. Mirrors the Rust types in src-tauri/src/embeddings. */

export interface EmbeddingsSettings {
  enabled: boolean;
  model: string;
  top_k: number;
}

export interface IndexCounts {
  transcript_chunks: number;
  summaries: number;
  memory_facts: number;
  total: number;
  meetings_indexed: number;
  meetings_with_transcripts: number;
  rows_from_other_models: number;
  last_indexed_at: string | null;
}

export interface IndexStatus {
  settings: EmbeddingsSettings;
  counts: IndexCounts;
  model_downloaded: boolean;
  model_loaded: boolean;
  model_id: string;
  dimensions: number;
  download_size_mb: number;
  models_dir: string | null;
  reindex_running: boolean;
  summary: string;
}

export type SearchMatchKind = 'semantic' | 'keyword' | 'both';

export interface SearchHit {
  source_kind: 'transcript_chunk' | 'summary' | 'memory_fact';
  source_id: string;
  meeting_id: string;
  meeting_title: string;
  client_id: string | null;
  text: string;
  semantic_score: number | null;
  keyword_match: boolean;
  score: number;
  match_kind: SearchMatchKind;
}

export interface SearchResults {
  hits: SearchHit[];
  semantic_used: boolean;
  semantic_unavailable_reason: string | null;
  unindexed_meetings: number;
  stale_rows: number;
}

/** Payload of the `embeddings-model-download-progress` event. */
export interface ModelDownloadProgress {
  file: string;
  progress: number;
  downloaded_mb: number;
  total_mb: number;
  status: string;
}

/** Payload of the `embeddings-reindex-progress` event. */
export interface ReindexProgress {
  phase: 'preparing' | 'indexing' | 'complete' | 'failed';
  meetings_done: number;
  meetings_total: number;
  passages_written: number;
  current_meeting_title: string | null;
  error: string | null;
}
