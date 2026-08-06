//! Network transparency: the app's own record of everything it sent out.
//!
//! The category's marketing habit is to say "local-first" and let the user take it
//! on faith. This module exists so the claim can be checked instead. Every place
//! the app makes an HTTP request records one row: when, to which host, why, how
//! many bytes each way, whether it carried audio or transcript text, and which
//! privacy profile was in force.
//!
//! ## What this is and is not
//!
//! It is the app's own instrumentation, so it reports what the app believes it
//! sent. It is not a packet capture and it cannot see traffic from anything else
//! on the machine. The panel says so, and points the operator at their own network
//! monitor for independent confirmation. Reporting facts and naming the limit is
//! the whole point; claiming more would defeat it.
//!
//! ## Why recording is fire-and-forget
//!
//! `record` is a synchronous call that returns immediately: it appends to an
//! in-memory ring for this session and hands the database write to a background
//! task. That is deliberate. Instrumentation that can slow down or fail a real
//! request would get removed the first time it caused a bug, and instrumentation
//! that needs a pool handle threaded through every call site never gets added to
//! the awkward sites at all. The pool is registered once at startup instead.

pub mod commands;
pub mod export;
pub mod hosts;
pub mod store;

use chrono::Utc;
use once_cell::sync::Lazy;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

pub use hosts::Purpose;
use store::{NetworkEventRow, NetworkEventsStore};

/// How many events this session keeps in memory. The database is the durable
/// record; this ring only exists so the panel can answer "this session" instantly
/// and still answer it if a database write failed.
const SESSION_RING_CAPACITY: usize = 500;

static POOL: OnceLock<SqlitePool> = OnceLock::new();

/// Identifies this run of the app, so "this session" means something after a
/// restart.
static SESSION_ID: Lazy<String> = Lazy::new(|| format!("session-{}", Uuid::new_v4()));

static SESSION_RING: Lazy<Mutex<VecDeque<NetworkEventRow>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(SESSION_RING_CAPACITY)));

tokio::task_local! {
    /// The meeting a request belongs to, when the layer that knows it wrapped the
    /// work in `with_meeting`. Deep call sites (the shared LLM client, for one)
    /// have no meeting id in scope and would need a new parameter on a function
    /// with five callers to get one; a task-local carries it down instead, and is
    /// simply absent when nobody set it.
    static MEETING: Option<String>;
}

/// Registers the pool the background writer uses. Called once at startup.
pub fn register_pool(pool: SqlitePool) {
    if POOL.set(pool).is_err() {
        log::warn!("[Network] pool already registered; keeping the first one");
    }
}

pub fn session_id() -> &'static str {
    &SESSION_ID
}

/// Runs `future` with `meeting_id` attached, so any request it makes is attributed
/// to that meeting.
pub async fn with_meeting<F>(meeting_id: &str, future: F) -> F::Output
where
    F: std::future::Future,
{
    MEETING.scope(Some(meeting_id.to_string()), future).await
}

fn current_meeting() -> Option<String> {
    MEETING.try_with(|meeting| meeting.clone()).ok().flatten()
}

/// Records one completed request.
pub fn record(
    purpose: Purpose,
    url: &str,
    method: &str,
    outcome: &str,
    bytes_out: u64,
    bytes_in: u64,
    detail: &str,
) {
    let (host, sanitized) = hosts::split_url(url);
    let row = NetworkEventRow {
        id: format!("net-{}", Uuid::new_v4()),
        created_at: Utc::now(),
        session_id: SESSION_ID.clone(),
        host,
        url: sanitized,
        method: method.to_string(),
        purpose: purpose.as_str().to_string(),
        outcome: outcome.to_string(),
        bytes_out: bytes_out as i64,
        bytes_in: bytes_in as i64,
        meeting_id: current_meeting(),
        // Filled in by the background writer, which can afford a database read.
        profile_name: None,
        carried_audio: purpose.carries_audio(),
        carried_transcript: purpose.carries_transcript(),
        detail: detail.to_string(),
    };

    if let Ok(mut ring) = SESSION_RING.lock() {
        if ring.len() == SESSION_RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(row.clone());
    }

    let Some(pool) = POOL.get().cloned() else {
        // Before the database is up (very early startup) the ring is the record.
        return;
    };
    // Tauri's own runtime handle rather than `tokio::spawn`, so this also works from
    // a synchronous command running on a blocking thread — several provider calls do.
    tauri::async_runtime::spawn(async move {
        let mut row = row;
        row.profile_name = resolve_profile_name(&pool, row.meeting_id.as_deref()).await;
        if let Err(e) = NetworkEventsStore::insert(&pool, &row).await {
            log::warn!("[Network] failed to record a request to {}: {}", row.host, e);
        }
    });
}

/// The name of the privacy profile in force, resolved when the event is written.
async fn resolve_profile_name(pool: &SqlitePool, meeting_id: Option<&str>) -> Option<String> {
    let scope = match meeting_id {
        Some(id) => crate::profiles::enforce::Scope::meeting(id),
        None => crate::profiles::enforce::Scope::Workspace,
    };
    let effective = crate::profiles::enforce::resolve(pool, &scope).await;
    effective.profile_name().map(str::to_string)
}

/// Records a request that completed successfully.
pub fn record_success(purpose: Purpose, url: &str, method: &str, bytes_out: u64, bytes_in: u64) {
    record(purpose, url, method, "ok", bytes_out, bytes_in, "");
}

/// Records a request that failed. The detail is the error as the app saw it, which
/// is useful precisely because a blocked request is what a firewall test produces.
pub fn record_failure(purpose: Purpose, url: &str, method: &str, detail: &str) {
    record(purpose, url, method, "error", 0, 0, detail);
}

/// Records the outcome of a completed `reqwest` call, borrowing the result so the
/// caller can still consume the response afterwards.
///
/// This is the one-line form the instrumented call sites use: it reads the status
/// and the advertised body size without touching the body, so instrumentation
/// cannot change what the caller receives.
pub fn observe(
    purpose: Purpose,
    url: &str,
    method: &str,
    bytes_out: u64,
    result: &Result<reqwest::Response, reqwest::Error>,
) {
    match result {
        Ok(response) => {
            let status = response.status();
            let bytes_in = response.content_length().unwrap_or(0);
            if status.is_success() {
                record(purpose, url, method, "ok", bytes_out, bytes_in, "");
            } else {
                record(
                    purpose,
                    url,
                    method,
                    "error",
                    bytes_out,
                    bytes_in,
                    &format!("HTTP {}", status),
                );
            }
        }
        Err(e) => record_failure(purpose, url, method, &e.to_string()),
    }
}

/// The same as `observe`, for the handful of call sites that use reqwest's blocking
/// client. Separate because `reqwest::blocking::Response` is a distinct type.
pub fn observe_blocking(
    purpose: Purpose,
    url: &str,
    method: &str,
    bytes_out: u64,
    result: &Result<reqwest::blocking::Response, reqwest::Error>,
) {
    match result {
        Ok(response) => {
            let status = response.status();
            let bytes_in = response.content_length().unwrap_or(0);
            if status.is_success() {
                record(purpose, url, method, "ok", bytes_out, bytes_in, "");
            } else {
                record(
                    purpose,
                    url,
                    method,
                    "error",
                    bytes_out,
                    bytes_in,
                    &format!("HTTP {}", status),
                );
            }
        }
        Err(e) => record_failure(purpose, url, method, &e.to_string()),
    }
}

/// The events recorded during this run of the app, newest first.
pub fn session_events() -> Vec<NetworkEventRow> {
    SESSION_RING
        .lock()
        .map(|ring| ring.iter().rev().cloned().collect())
        .unwrap_or_default()
}
