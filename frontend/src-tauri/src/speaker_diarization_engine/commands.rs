// speaker_diarization_engine/commands.rs
//
// Tauri commands and background workflow for speaker diarization.
//
// Adapted from mimi202605/meeting-minutes (commits cf66169 + cbd21c2). Key
// differences from the fork:
// - Models are downloaded at runtime on first use (mirroring the whisper /
//   parakeet model flows in this tree) from OFFICIAL github.com/k2-fsa release
//   URLs only, with pinned SHA256 verification. No bundled model blobs.
// - The diarization pass runs after the meeting is saved to the database
//   (this tree saves from the frontend after `recording-stopped`), spawned in
//   the background from `api_save_transcript`. Failures only log and emit a
//   non-blocking event; they never affect the saved recording or transcripts.
// - Refines only lines NOT attributed to the local microphone: "You" lines
//   stay untouched; "Others"/unlabeled lines become "Speaker 1", "Speaker 2"...

use log::{info, warn};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use super::engine::{
    align_transcripts_with_speakers, SpeakerDiarizationEngine, TranscriptChunkForAlignment,
    PYANNOTE_MODEL_DIR, PYANNOTE_MODEL_FILE,
};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

/// Global engine instance.
static DIARIZATION_ENGINE: Mutex<Option<Arc<SpeakerDiarizationEngine>>> = Mutex::new(None);

/// Global models directory path (set during app initialization, whisper pattern).
static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Guard against concurrent model downloads.
static DOWNLOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Guard against concurrent diarization runs (model load + inference are heavy).
static DIARIZATION_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

// Official model sources — github.com/k2-fsa releases ONLY (no proxy mirrors).
// Note: "speaker-recongition-models" is the upstream release tag's own typo.
const CAMPLUS_MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx";
/// SHA256 of the CAM++ embedding model file (28,281,138 bytes), pinned from the
/// official URL above on 2026-08-03.
const CAMPLUS_MODEL_SHA256: &str =
    "f682b514c05d947ee3fa91cd6ec6c5c7543479a128373fa29b1faedccd21fd11";

const PYANNOTE_ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2";
/// SHA256 of the pyannote segmentation archive (6,958,444 bytes), pinned from
/// the official URL above on 2026-08-03. Verified BEFORE extraction.
const PYANNOTE_ARCHIVE_SHA256: &str =
    "24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488";

/// Set the models directory using app_data_dir (same location as whisper models).
/// Called during app setup.
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let app_data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("[Diarization] Failed to get app data dir: {}", e);
            return;
        }
    };

    let models_dir = app_data_dir.join("models");
    if !models_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            log::error!("[Diarization] Failed to create models directory: {}", e);
            return;
        }
    }

    info!(
        "[Diarization] Models directory set to: {}",
        models_dir.display()
    );

    let mut guard = MODELS_DIR.lock().unwrap();
    *guard = Some(models_dir);
}

fn get_models_directory() -> Result<PathBuf, String> {
    MODELS_DIR
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Diarization models directory not initialized".to_string())
}

/// Get or create the engine instance.
fn get_engine() -> Result<Arc<SpeakerDiarizationEngine>, String> {
    let mut guard = DIARIZATION_ENGINE.lock().unwrap();
    if guard.is_none() {
        let models_dir = get_models_directory()?;
        *guard = Some(Arc::new(SpeakerDiarizationEngine::new(models_dir)));
    }
    Ok(guard.as_ref().unwrap().clone())
}

/// Check whether both diarization models are present on disk.
#[tauri::command]
pub async fn diarization_is_ready() -> Result<bool, String> {
    Ok(get_engine()?.is_ready())
}

#[derive(Debug, Serialize)]
pub struct DiarizationModelInfo {
    pub ready: bool,
    pub segmentation_model_present: bool,
    pub embedding_model_present: bool,
    pub models_dir: String,
    /// Approximate download size in MB when models are missing.
    pub download_size_mb: u32,
}

/// Model status for the settings UI.
#[tauri::command]
pub async fn diarization_get_model_info() -> Result<DiarizationModelInfo, String> {
    let engine = get_engine()?;
    let seg = engine.pyannote_model_path().exists();
    let emb = engine.camplus_model_path().exists();
    Ok(DiarizationModelInfo {
        ready: seg && emb,
        segmentation_model_present: seg,
        embedding_model_present: emb,
        models_dir: engine.diarization_models_dir().to_string_lossy().to_string(),
        download_size_mb: 34, // ~7 MB segmentation archive + ~27 MB embedding model
    })
}

// ---------------------------------------------------------------------------
// Model download (runtime, first use) — official k2-fsa URLs, SHA256 pinned.
// ---------------------------------------------------------------------------

fn emit_download_progress<R: Runtime>(
    app: &AppHandle<R>,
    model: &str,
    downloaded: u64,
    total: u64,
    status: &str,
) {
    let progress = if total > 0 {
        ((downloaded as f64 / total as f64) * 100.0) as u32
    } else {
        0
    };
    let _ = app.emit(
        "diarization-model-download-progress",
        serde_json::json!({
            "model": model,
            "progress": progress,
            "downloaded_mb": downloaded as f64 / (1024.0 * 1024.0),
            "total_mb": total as f64 / (1024.0 * 1024.0),
            "status": status,
        }),
    );
}

/// Stream a URL to `dest` (via a temp file), emitting progress events.
async fn download_file<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    dest: &Path,
    model_label: &str,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    info!("[Diarization] Downloading {} from {}", model_label, url);

    let client = reqwest::Client::new();
    let outcome = client.get(url).send().await;
    crate::network::observe(
        crate::network::Purpose::ModelDownload,
        url,
        "GET",
        0,
        &outcome,
    );
    let response = outcome.map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Download failed with HTTP {} for {}",
            response.status(),
            url
        ));
    }

    let total = response.content_length().unwrap_or(0);
    let tmp_path = dest.with_extension("download");

    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_percent: u32 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write model data: {}", e))?;
        downloaded += chunk.len() as u64;

        if total > 0 {
            let percent = ((downloaded as f64 / total as f64) * 100.0) as u32;
            if percent != last_percent {
                last_percent = percent;
                emit_download_progress(app, model_label, downloaded, total, "downloading");
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Failed to flush model file: {}", e))?;
    drop(file);

    tokio::fs::rename(&tmp_path, dest)
        .await
        .map_err(|e| format!("Failed to finalize model file: {}", e))?;

    Ok(())
}

/// Verify a file's SHA256 against the pinned hash; delete the file on mismatch.
async fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let path_owned = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || -> Result<String, String> {
        use std::io::Read;
        let mut file = std::fs::File::open(&path_owned)
            .map_err(|e| format!("Failed to open file for hashing: {}", e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await
    .map_err(|e| format!("Hash task join error: {}", e))??;

    if actual != expected {
        let _ = tokio::fs::remove_file(path).await;
        return Err(format!(
            "SHA256 mismatch for {} (expected {}, got {}); file deleted. \
             The download may be corrupted or tampered with.",
            path.display(),
            expected,
            actual
        ));
    }
    Ok(())
}

/// Extract `model.onnx` from the pyannote tar.bz2 archive into the model dir.
fn extract_pyannote_archive(archive_path: &Path, target_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| format!("Failed to open archive: {}", e))?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let expected_entry = PathBuf::from(PYANNOTE_MODEL_DIR).join(PYANNOTE_MODEL_FILE);
    let target_path = target_dir.join(PYANNOTE_MODEL_DIR).join(PYANNOTE_MODEL_FILE);

    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create model directory: {}", e))?;
    }

    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read archive: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("Invalid archive entry path: {}", e))?
            .to_path_buf();

        // ends_with (component-wise) tolerates archives that prefix entries
        // with "./" while still requiring the exact dir/file component pair.
        if entry_path.ends_with(&expected_entry) {
            entry
                .unpack(&target_path)
                .map_err(|e| format!("Failed to extract model: {}", e))?;
            info!(
                "[Diarization] Extracted {} to {}",
                expected_entry.display(),
                target_path.display()
            );
            return Ok(());
        }
    }

    Err(format!(
        "Archive did not contain expected entry {}",
        expected_entry.display()
    ))
}

/// Download both diarization models (pyannote segmentation + CAM++ embedding)
/// from official k2-fsa release URLs, verifying pinned SHA256 hashes.
#[tauri::command]
pub async fn diarization_download_models<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    if DOWNLOAD_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A diarization model download is already in progress".to_string());
    }

    let result = download_models_inner(&app).await;
    DOWNLOAD_IN_PROGRESS.store(false, Ordering::SeqCst);

    match &result {
        Ok(()) => {
            let _ = app.emit(
                "diarization-model-download-complete",
                serde_json::json!({ "status": "completed" }),
            );
        }
        Err(e) => {
            warn!("[Diarization] Model download failed: {}", e);
            let _ = app.emit(
                "diarization-model-download-error",
                serde_json::json!({ "error": e }),
            );
        }
    }
    result
}

async fn download_models_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let engine = get_engine()?;
    let dir = engine.diarization_models_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create diarization models dir: {}", e))?;

    // 1. Pyannote segmentation model (tar.bz2 archive -> model.onnx).
    if !engine.pyannote_model_path().exists() {
        let archive_path = dir.join("sherpa-onnx-pyannote-segmentation-3-0.tar.bz2");
        download_file(app, PYANNOTE_ARCHIVE_URL, &archive_path, "pyannote-segmentation").await?;
        verify_sha256(&archive_path, PYANNOTE_ARCHIVE_SHA256).await?;

        let dir_clone = dir.clone();
        let archive_clone = archive_path.clone();
        tokio::task::spawn_blocking(move || extract_pyannote_archive(&archive_clone, &dir_clone))
            .await
            .map_err(|e| format!("Extract task join error: {}", e))??;

        let _ = tokio::fs::remove_file(&archive_path).await;

        if !engine.pyannote_model_path().exists() {
            return Err("Segmentation model missing after extraction".to_string());
        }
    }

    // 2. CAM++ speaker embedding model (raw .onnx file).
    if !engine.camplus_model_path().exists() {
        let model_path = engine.camplus_model_path();
        download_file(app, CAMPLUS_MODEL_URL, &model_path, "campplus-embedding").await?;
        verify_sha256(&model_path, CAMPLUS_MODEL_SHA256).await?;
    }

    info!("[Diarization] All models downloaded and verified");
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings (enabled toggle, default ON) — stored in diarization_settings.
// ---------------------------------------------------------------------------

async fn is_diarization_enabled(pool: &sqlx::SqlitePool) -> bool {
    match sqlx::query_scalar::<_, i64>("SELECT enabled FROM diarization_settings WHERE id = 1")
        .fetch_optional(pool)
        .await
    {
        Ok(Some(v)) => v != 0,
        Ok(None) => true, // default on
        Err(e) => {
            warn!("[Diarization] Failed to read settings ({}); defaulting to enabled", e);
            true
        }
    }
}

#[tauri::command]
pub async fn diarization_get_settings(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let enabled = is_diarization_enabled(state.db_manager.pool()).await;
    Ok(serde_json::json!({ "enabled": enabled }))
}

#[tauri::command]
pub async fn diarization_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO diarization_settings (id, enabled) VALUES (1, ?) \
         ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled",
    )
    .bind(enabled as i64)
    .execute(state.db_manager.pool())
    .await
    .map_err(|e| format!("Failed to save diarization setting: {}", e))?;
    info!("[Diarization] Enabled set to {}", enabled);
    Ok(())
}

// ---------------------------------------------------------------------------
// Diarization workflow
// ---------------------------------------------------------------------------

fn emit_stage<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, stage: &str, message: &str) {
    let _ = app.emit(
        "transcript-diarization-progress",
        serde_json::json!({
            "meeting_id": meeting_id,
            "stage": stage,
            "message": message,
        }),
    );
}

/// Auto-trigger entry point, spawned in the background after a recorded meeting
/// is saved to the database. Checks the enabled toggle and model availability;
/// all failures are non-fatal (log + non-blocking event only).
pub async fn maybe_run_auto_diarization<R: Runtime>(app: AppHandle<R>, meeting_id: String) {
    let pool = {
        let state = app.state::<AppState>();
        state.db_manager.pool().clone()
    };

    if !is_diarization_enabled(&pool).await {
        info!(
            "[Diarization] Skipping auto diarization for {} (disabled in settings)",
            meeting_id
        );
        return;
    }

    let ready = match get_engine() {
        Ok(engine) => engine.is_ready(),
        Err(e) => {
            warn!("[Diarization] Engine unavailable: {}", e);
            false
        }
    };
    if !ready {
        info!(
            "[Diarization] Skipping auto diarization for {} (models not downloaded)",
            meeting_id
        );
        return;
    }

    if let Err(e) = run_and_report(&app, &meeting_id).await {
        warn!(
            "[Diarization] Auto diarization failed for {} (recording and transcripts unaffected): {}",
            meeting_id, e
        );
    }
}

/// Manually trigger speaker diarization for a saved meeting. Emits the same
/// events as the automatic flow so existing listeners handle UI updates.
#[tauri::command]
pub async fn run_speaker_diarization<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<(), String> {
    info!("[Diarization] Manual trigger for meeting {}", meeting_id);
    run_and_report(&app, &meeting_id).await
}

/// Run diarization and emit started / diarized / error events around it.
async fn run_and_report<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<(), String> {
    if DIARIZATION_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A diarization run is already in progress".to_string());
    }

    let _ = app.emit(
        "transcript-diarization-started",
        serde_json::json!({ "meeting_id": meeting_id }),
    );

    let result = run_diarization_for_meeting(app, meeting_id).await;
    DIARIZATION_IN_PROGRESS.store(false, Ordering::SeqCst);

    if let Err(e) = &result {
        let _ = app.emit(
            "transcript-diarization-error",
            serde_json::json!({ "meeting_id": meeting_id, "error": e }),
        );
    }
    result
}

/// Core diarization pass for a saved meeting:
/// decode audio -> diarize -> align with transcript rows -> update speaker
/// labels in the database -> emit `transcript-diarized`.
///
/// Only rows NOT attributed to the local microphone ("You") are refined.
async fn run_diarization_for_meeting<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
) -> Result<(), String> {
    let pool = {
        let state = app.state::<AppState>();
        state.db_manager.pool().clone()
    };

    // 1. Resolve the meeting folder from the database.
    let meeting = MeetingsRepository::get_meeting_metadata(&pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to query meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let folder_path_str = meeting
        .folder_path
        .ok_or_else(|| format!("Meeting {} has no recording folder", meeting_id))?;
    let folder = Path::new(&folder_path_str);
    if !folder.exists() {
        return Err(format!("Meeting folder does not exist: {}", folder.display()));
    }

    // 2. Locate the audio file inside the meeting folder.
    let audio_path = find_audio_file(folder)
        .ok_or_else(|| format!("No audio file found in {}", folder.display()))?;
    info!("[Diarization] Using audio file: {}", audio_path.display());

    // 3. Load transcript rows that can be aligned.
    let rows: Vec<(String, Option<f64>, Option<f64>, Option<String>)> = sqlx::query_as(
        "SELECT id, audio_start_time, audio_end_time, speaker FROM transcripts \
         WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(meeting_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("Failed to query transcripts: {}", e))?;

    if rows.is_empty() {
        return Err(format!("No transcripts found for meeting {}", meeting_id));
    }

    // Refine everything except mic-attributed "You" lines.
    let chunks: Vec<TranscriptChunkForAlignment> = rows
        .iter()
        .filter(|(_, start, end, speaker)| {
            speaker.as_deref() != Some("You") && start.is_some() && end.is_some()
        })
        .map(|(id, start, end, _)| TranscriptChunkForAlignment {
            id: id.clone(),
            audio_start_time: start.unwrap_or(0.0),
            audio_end_time: end.unwrap_or(0.0),
            speaker: None,
        })
        .collect();

    if chunks.is_empty() {
        info!(
            "[Diarization] No refinable transcript lines for meeting {} (all mic-attributed)",
            meeting_id
        );
        let _ = app.emit(
            "transcript-diarized",
            serde_json::json!({
                "meeting_id": meeting_id,
                "num_speakers": 0,
                "updated_count": 0,
            }),
        );
        return Ok(());
    }

    // 4. Load the diarization models (lazy, blocking thread).
    let engine = get_engine()?;
    emit_stage(app, meeting_id, "loading_models", "Loading speaker models...");
    {
        let engine_clone = engine.clone();
        tokio::task::spawn_blocking(move || engine_clone.load())
            .await
            .map_err(|e| format!("Load join error: {}", e))??;
    }

    // 5. Decode the recording to 16kHz mono PCM (blocking thread).
    emit_stage(app, meeting_id, "decoding_audio", "Decoding recording audio...");
    let decode_path = audio_path.clone();
    let decoded = tokio::task::spawn_blocking(move || {
        crate::audio::decoder::decode_audio_file(&decode_path)
    })
    .await
    .map_err(|e| format!("Decode join error: {}", e))?
    .map_err(|e| format!("Failed to decode audio: {}", e))?;

    if decoded.duration_seconds < 1.0 {
        return Err(format!(
            "Audio too short for diarization ({:.2}s)",
            decoded.duration_seconds
        ));
    }

    let samples = decoded.to_whisper_format();
    info!(
        "[Diarization] Decoded {} samples ({:.1}s)",
        samples.len(),
        decoded.duration_seconds
    );

    // 6. Run diarization (blocking thread).
    emit_stage(app, meeting_id, "diarizing", "Identifying speakers...");
    let segments = {
        let engine_clone = engine.clone();
        tokio::task::spawn_blocking(move || engine_clone.diarize(&samples))
            .await
            .map_err(|e| format!("Diarize join error: {}", e))??
    };

    if segments.is_empty() {
        return Err("No speech segments detected".to_string());
    }

    // 7. Align speaker segments with transcript rows and persist labels.
    emit_stage(app, meeting_id, "aligning", "Aligning speakers with transcript...");
    let aligned = align_transcripts_with_speakers(chunks, &segments);

    let mut updated_count: usize = 0;
    for chunk in &aligned {
        if let Some(speaker_id) = chunk.speaker {
            let label = format!("Speaker {}", speaker_id + 1);
            let result = sqlx::query(
                "UPDATE transcripts SET speaker = ? WHERE id = ? AND meeting_id = ?",
            )
            .bind(&label)
            .bind(&chunk.id)
            .bind(meeting_id)
            .execute(&pool)
            .await;
            match result {
                Ok(_) => updated_count += 1,
                Err(e) => warn!(
                    "[Diarization] Failed to update speaker for transcript {}: {}",
                    chunk.id, e
                ),
            }
        }
    }

    let num_speakers = segments
        .iter()
        .map(|s| s.speaker)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);

    info!(
        "[Diarization] Success for meeting {}: {} speakers, {} lines relabeled",
        meeting_id, num_speakers, updated_count
    );

    let _ = app.emit(
        "transcript-diarized",
        serde_json::json!({
            "meeting_id": meeting_id,
            "num_speakers": num_speakers,
            "updated_count": updated_count,
        }),
    );

    Ok(())
}

/// Find an audio file inside a meeting folder.
///
/// Tries the `audio_file` field in `metadata.json` first, then falls back to
/// scanning the folder for common audio extensions (`.wav` preferred).
fn find_audio_file(folder: &Path) -> Option<PathBuf> {
    // 1. Try metadata.json -> audio_file
    let metadata_path = folder.join("metadata.json");
    if let Ok(content) = std::fs::read_to_string(&metadata_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(audio_file) = json.get("audio_file").and_then(|v| v.as_str()) {
                if !audio_file.is_empty() {
                    let path = folder.join(audio_file);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }

    // 2. Fallback: scan the folder for audio files, preferring .wav then .mp4.
    let preferred_order = [
        "wav", "mp4", "m4a", "mp3", "flac", "ogg", "aac", "mkv", "webm", "wma",
    ];
    let entries: Vec<_> = std::fs::read_dir(folder).ok()?.flatten().collect();

    for ext in &preferred_order {
        for entry in &entries {
            let path = entry.path();
            if let Some(ext_str) = path.extension().and_then(|e| e.to_str()) {
                if ext_str.eq_ignore_ascii_case(ext) {
                    return Some(path);
                }
            }
        }
    }

    None
}

/// Fire-and-forget hook used by `api_save_transcript` after a recorded meeting
/// (one that has a folder_path) is saved. Never blocks or fails the save.
pub fn spawn_auto_diarization<R: Runtime>(app: &AppHandle<R>, meeting_id: String) {
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        maybe_run_auto_diarization(app_clone, meeting_id).await;
    });
}
