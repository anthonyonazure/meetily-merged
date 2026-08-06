//! The on-device embedding model: where it comes from, and how text becomes a
//! vector.
//!
//! ## Why a dedicated model rather than the built-in AI sidecar
//!
//! The built-in AI sidecar (`summary::summary_engine`) is llama.cpp-based and
//! could in principle produce embeddings, but its wire protocol
//! (`llama-helper`) exposes exactly one request type — `generate` — and returns
//! text. Teaching it to return vectors means changing the sidecar binary and its
//! protocol, and the sidecar's models are 1B-class generators whose embeddings
//! are both slower and worse for retrieval than a purpose-built sentence encoder.
//! So this uses a small dedicated embedding model, downloaded and verified the
//! same way the whisper, parakeet, and diarization models already are.
//!
//! ## Which model, and why
//!
//! `sentence-transformers/all-MiniLM-L6-v2`, ONNX export:
//!
//! * **Licence**: Apache-2.0, so it ships with a commercial MSP product without a
//!   licence question.
//! * **Size**: 90 MB, 384 dimensions, 6 layers. Small enough to run on a
//!   technician's laptop CPU alongside a recording, which is the whole point of
//!   local-first.
//! * **Ubiquity**: the default sentence-transformers retrieval model, with a
//!   decade of downstream use. Its behaviour on short English passages is a known
//!   quantity, which matters more than a marginally better benchmark score.
//! * **Runtime already present**: this tree already links ONNX Runtime (`ort`) for
//!   Parakeet transcription, so nothing new is added to the build for inference.
//!
//! Both files are pinned by SHA256, verified after download, and deleted on
//! mismatch — the precedent set by `speaker_diarization_engine`.

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use ndarray::Array2;
use ort::execution_providers::CPUExecutionProvider;
use ort::inputs;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use super::tokenizer::WordPiece;
use super::vector;

/// The model identifier written into `embeddings.model`. Changing the model means
/// changing this string, which makes every existing row visibly from a different
/// model instead of silently mixing incompatible vectors.
pub const MODEL_ID: &str = "all-MiniLM-L6-v2";

/// Output width of the model.
pub const DIM: usize = 384;

/// Token window. The model's positional embeddings run to 512, but it was trained
/// on 256 and passages are chunked to fit well inside that.
pub const MAX_SEQ_LEN: usize = 256;

/// How many passages are pushed through the graph at once.
const BATCH_SIZE: usize = 8;

const MODEL_FILE: &str = "all-MiniLM-L6-v2.onnx";
const VOCAB_FILE: &str = "all-MiniLM-L6-v2-vocab.txt";

/// Official source: the model author's own Hugging Face repository. Hugging Face
/// is already the host the whisper and parakeet model downloads use, so this adds
/// no new outbound destination.
const MODEL_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";
/// SHA256 of the ONNX graph (90,405,214 bytes), pinned from the official URL
/// above on 2026-08-06.
const MODEL_SHA256: &str = "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452";

const VOCAB_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/vocab.txt";
/// SHA256 of the WordPiece vocabulary (231,508 bytes, 30,522 tokens), pinned from
/// the official URL above on 2026-08-06.
const VOCAB_SHA256: &str = "07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3";

/// Approximate download size shown in the UI before the operator commits to it.
pub const DOWNLOAD_SIZE_MB: u32 = 87;

/// Models directory, set during app setup (the whisper/parakeet pattern).
static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Guard against two concurrent downloads of the same files.
static DOWNLOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// The loaded model. Held behind a std Mutex and only touched from
/// `spawn_blocking`, because ONNX inference is CPU-bound and must not occupy an
/// async worker.
static MODEL: Lazy<Mutex<Option<EmbeddingModel>>> = Lazy::new(|| Mutex::new(None));

pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let app_data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("[Embeddings] Failed to get app data dir: {}", e);
            return;
        }
    };
    let dir = app_data_dir.join("models").join("embeddings");
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error!("[Embeddings] Failed to create models directory: {}", e);
            return;
        }
    }
    log::info!("[Embeddings] Models directory set to: {}", dir.display());
    *MODELS_DIR.lock().unwrap() = Some(dir);
}

pub fn models_directory() -> Result<PathBuf, String> {
    MODELS_DIR
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Embedding models directory not initialised".to_string())
}

pub fn model_path() -> Result<PathBuf, String> {
    Ok(models_directory()?.join(MODEL_FILE))
}

pub fn vocab_path() -> Result<PathBuf, String> {
    Ok(models_directory()?.join(VOCAB_FILE))
}

/// True when both files are on disk. Says nothing about whether they are valid;
/// `ensure_downloaded` is what verifies them.
pub fn files_present() -> bool {
    match (model_path(), vocab_path()) {
        (Ok(model), Ok(vocab)) => model.exists() && vocab.exists(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Download and verification
// ---------------------------------------------------------------------------

fn emit_progress<R: Runtime>(
    app: &AppHandle<R>,
    file: &str,
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
        "embeddings-model-download-progress",
        serde_json::json!({
            "file": file,
            "progress": progress,
            "downloaded_mb": downloaded as f64 / (1024.0 * 1024.0),
            "total_mb": total as f64 / (1024.0 * 1024.0),
            "status": status,
        }),
    );
}

async fn download_file<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    dest: &Path,
    label: &str,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    log::info!("[Embeddings] Downloading {} from {}", label, url);

    let client = reqwest::Client::new();
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(e) => {
            crate::network::record_failure(
                crate::network::Purpose::ModelDownload,
                url,
                "GET",
                &e.to_string(),
            );
            return Err(format!("Download request failed: {}", e));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        crate::network::record_failure(
            crate::network::Purpose::ModelDownload,
            url,
            "GET",
            &format!("HTTP {}", status),
        );
        return Err(format!("Download failed with HTTP {} for {}", status, url));
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
                emit_progress(app, label, downloaded, total, "downloading");
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

    crate::network::record_success(
        crate::network::Purpose::ModelDownload,
        url,
        "GET",
        0,
        downloaded,
    );
    Ok(())
}

/// Verifies a file's SHA256 against the pinned hash; deletes the file on
/// mismatch so a corrupt or substituted download is never loaded.
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
    .map_err(|e| format!("Hashing task failed: {}", e))??;

    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    let _ = tokio::fs::remove_file(path).await;
    Err(format!(
        "Downloaded file {} does not match its pinned checksum (expected {}, got {}); the file was deleted",
        path.display(),
        expected,
        actual
    ))
}

/// Downloads and verifies both files if they are not already present.
pub async fn ensure_downloaded<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if files_present() {
        return Ok(());
    }
    if DOWNLOAD_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("The embedding model is already downloading".to_string());
    }
    let result = download_both(app).await;
    DOWNLOAD_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

async fn download_both<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let model = model_path()?;
    let vocab = vocab_path()?;

    if !vocab.exists() {
        download_file(app, VOCAB_URL, &vocab, "vocabulary").await?;
        verify_sha256(&vocab, VOCAB_SHA256).await?;
    }
    if !model.exists() {
        download_file(app, MODEL_URL, &model, "embedding model").await?;
        verify_sha256(&model, MODEL_SHA256).await?;
    }
    emit_progress(app, "embedding model", 1, 1, "complete");
    log::info!("[Embeddings] Model ready at {}", model.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Inference
// ---------------------------------------------------------------------------

pub struct EmbeddingModel {
    session: Session,
    tokenizer: WordPiece,
}

impl EmbeddingModel {
    fn load() -> Result<Self, String> {
        let model = model_path()?;
        let vocab = vocab_path()?;
        if !model.exists() || !vocab.exists() {
            return Err("The embedding model has not been downloaded yet".to_string());
        }

        let vocab_text = std::fs::read_to_string(&vocab)
            .map_err(|e| format!("Failed to read the embedding vocabulary: {}", e))?;
        let tokenizer = WordPiece::from_vocab_text(&vocab_text)?;

        let session = Session::builder()
            .map_err(|e| format!("Failed to create an ONNX session builder: {}", e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("Failed to set the ONNX optimization level: {}", e))?
            .with_execution_providers(vec![CPUExecutionProvider::default().build()])
            .map_err(|e| format!("Failed to set the ONNX execution provider: {}", e))?
            .commit_from_file(&model)
            .map_err(|e| format!("Failed to load the embedding model: {}", e))?;

        log::info!(
            "[Embeddings] Loaded {} ({} vocabulary tokens)",
            MODEL_ID,
            tokenizer.vocab_size()
        );
        Ok(Self { session, tokenizer })
    }

    /// Embeds a batch of passages, returning one unit-length vector each in input
    /// order.
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let mut out = Vec::with_capacity(texts.len());
        for group in texts.chunks(BATCH_SIZE) {
            out.extend(self.embed_group(group)?);
        }
        Ok(out)
    }

    fn embed_group(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let encoded: Vec<_> = texts
            .iter()
            .map(|text| self.tokenizer.encode(text, MAX_SEQ_LEN))
            .collect();

        // Pad only to the longest real sequence in this group rather than the full
        // window: a batch of one-line passages then costs a fraction of a batch of
        // full ones.
        let width = encoded
            .iter()
            .map(|item| item.attention_mask.iter().filter(|m| **m == 1).count())
            .max()
            .unwrap_or(0)
            .max(2);

        let rows = encoded.len();
        let mut ids = Vec::with_capacity(rows * width);
        let mut mask = Vec::with_capacity(rows * width);
        let mut types = Vec::with_capacity(rows * width);
        for item in &encoded {
            ids.extend_from_slice(&item.input_ids[..width]);
            mask.extend_from_slice(&item.attention_mask[..width]);
            types.extend_from_slice(&item.token_type_ids[..width]);
        }

        let ids = Array2::from_shape_vec((rows, width), ids)
            .map_err(|e| format!("Failed to shape the token ids: {}", e))?;
        let mask_array = Array2::from_shape_vec((rows, width), mask.clone())
            .map_err(|e| format!("Failed to shape the attention mask: {}", e))?;
        let types = Array2::from_shape_vec((rows, width), types)
            .map_err(|e| format!("Failed to shape the token type ids: {}", e))?;

        let session_inputs = inputs![
            "input_ids" => TensorRef::from_array_view(ids.view())
                .map_err(|e| format!("Failed to bind input_ids: {}", e))?,
            "attention_mask" => TensorRef::from_array_view(mask_array.view())
                .map_err(|e| format!("Failed to bind attention_mask: {}", e))?,
            "token_type_ids" => TensorRef::from_array_view(types.view())
                .map_err(|e| format!("Failed to bind token_type_ids: {}", e))?,
        ];

        let outputs = self
            .session
            .run(session_inputs)
            .map_err(|e| format!("Embedding inference failed: {}", e))?;
        let hidden = outputs
            .get("last_hidden_state")
            .ok_or_else(|| "The embedding model returned no last_hidden_state".to_string())?
            .try_extract_array::<f32>()
            .map_err(|e| format!("Failed to read the embedding output: {}", e))?;

        // Collected through the iterator rather than the raw buffer so the values
        // arrive in logical [row][token][dim] order whatever the tensor's memory
        // layout happens to be.
        let flat: Vec<f32> = hidden.iter().copied().collect();
        let per_row = width * DIM;
        if flat.len() < rows * per_row {
            return Err(format!(
                "The embedding model returned {} values, expected {}",
                flat.len(),
                rows * per_row
            ));
        }

        let mut vectors = Vec::with_capacity(rows);
        for row in 0..rows {
            let states = &flat[row * per_row..(row + 1) * per_row];
            let row_mask = &mask[row * width..(row + 1) * width];
            let mut pooled = vector::mean_pool(states, row_mask, DIM);
            vector::normalize(&mut pooled);
            vectors.push(pooled);
        }
        Ok(vectors)
    }
}

/// Embeds passages on a blocking worker, loading the model on first use.
///
/// Returns unit-length vectors in input order. An empty input is answered without
/// touching the model at all, so a caller with nothing to index never triggers a
/// 90 MB load.
pub async fn embed(texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>, String> {
        let mut guard = MODEL
            .lock()
            .map_err(|_| "The embedding model lock was poisoned".to_string())?;
        if guard.is_none() {
            *guard = Some(EmbeddingModel::load()?);
        }
        guard
            .as_mut()
            .expect("model was just loaded")
            .embed(&texts)
    })
    .await
    .map_err(|e| format!("Embedding task failed: {}", e))?
}

/// Drops the loaded model, freeing its memory. Called when semantic search is
/// switched off so an operator who disables the feature stops paying for it.
pub fn unload() {
    if let Ok(mut guard) = MODEL.lock() {
        if guard.take().is_some() {
            log::info!("[Embeddings] Model unloaded");
        }
    }
}

/// True when the model is loaded in memory right now.
pub fn is_loaded() -> bool {
    MODEL.lock().map(|guard| guard.is_some()).unwrap_or(false)
}
