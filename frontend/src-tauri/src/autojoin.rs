//! Calendar auto-join prompt: when an upcoming event (EventKit or M365) with
//! a meeting URL starts within two minutes, surface a notification and an
//! in-app banner whose Join action opens the link. Prompt-then-open only —
//! there is no headless joining and nothing is opened without a click.
//!
//! The "Prompt to join from calendar" toggle lives in the integrations store
//! (key `autojoin_prompt`, default on) and is read every tick, so flipping
//! it takes effect without a restart.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

use crate::calendar::CalendarEvent;
use crate::m365::INTEGRATIONS_STORE;
use crate::notifications::commands::NotificationManagerState;
use crate::notifications::{Notification, NotificationPriority, NotificationType};

/// How often the scheduler wakes up. The prompt window is two minutes wide,
/// so 30s gives several chances to catch each meeting exactly once.
const TICK: Duration = Duration::from_secs(30);
/// M365 events are cached this long between Graph reads to keep the
/// integration quiet on the network.
const M365_CACHE_SECS: i64 = 300;
/// Prompt when an event starts within this many seconds from now.
const LEAD_SECS: i64 = 120;
/// ...or started up to this many seconds ago (covers a tick landing just
/// after the start time).
const GRACE_SECS: i64 = 60;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Called from the app's Exit handler so the loop stops touching state
/// while Tauri tears down.
pub fn shutdown() {
    SHUTDOWN.store(true, Ordering::Release);
}

#[derive(Debug, Clone, Serialize)]
struct MeetingStartingPayload {
    title: String,
    url: String,
    start: String,
}

fn prompt_enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.store(INTEGRATIONS_STORE)
        .ok()
        .and_then(|store| store.get("autojoin_prompt"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

/// Spawns the scheduler task. Cheap when idle: with the toggle off or no
/// calendar source connected, each tick is a store read and nothing else.
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        // event key -> start unix, so old entries can be pruned.
        let mut prompted: HashMap<String, i64> = HashMap::new();
        let mut m365_cache: Vec<CalendarEvent> = Vec::new();
        let mut m365_fetched_at: i64 = 0;

        log::info!("Calendar auto-join prompt scheduler started");
        loop {
            // Sleep in short slices so shutdown is prompt.
            for _ in 0..6 {
                if SHUTDOWN.load(Ordering::Acquire) {
                    log::info!("Calendar auto-join scheduler exiting");
                    return;
                }
                tokio::time::sleep(TICK / 6).await;
            }
            if !prompt_enabled(&app) {
                continue;
            }

            let now = chrono::Utc::now().timestamp();
            let mut events: Vec<CalendarEvent> = Vec::new();

            // Local (EventKit) source — macOS only; errors mean "no source",
            // not a problem worth logging every 30s.
            #[cfg(target_os = "macos")]
            if let Ok(local) = crate::calendar::calendar_upcoming_events().await {
                events.extend(local);
            }

            // M365 source, if connected. Cached to avoid chatty Graph reads.
            let m365_connected = matches!(crate::m365::auth::read_tokens().await, Ok(Some(_)));
            if m365_connected {
                if now - m365_fetched_at > M365_CACHE_SECS {
                    match crate::m365::commands::upcoming_events_with_refresh().await {
                        Ok(remote) => {
                            m365_cache = remote;
                            m365_fetched_at = now;
                        }
                        Err(e) => log::warn!("Auto-join: M365 calendar read failed: {}", e),
                    }
                }
                events.extend(m365_cache.iter().cloned());
            } else {
                m365_cache.clear();
                m365_fetched_at = 0;
            }

            for event in events {
                let Some(url) = event.meeting_url.clone() else {
                    continue;
                };
                let Ok(start) = chrono::DateTime::parse_from_rfc3339(&event.start) else {
                    continue;
                };
                let start_unix = start.timestamp();
                let delta = start_unix - now;
                if delta > LEAD_SECS || delta < -GRACE_SECS {
                    continue;
                }
                // Dedupe across sources AND across ticks: normalized title +
                // start minute identifies "the same meeting" whether it came
                // from EventKit, Graph, or both.
                let key = format!(
                    "{}|{}",
                    event.title.trim().to_lowercase(),
                    start_unix / 60
                );
                if prompted.contains_key(&key) {
                    continue;
                }
                prompted.insert(key, start_unix);

                log::info!("Auto-join: prompting for '{}'", event.title);
                let _ = app.emit(
                    "calendar-meeting-starting",
                    MeetingStartingPayload {
                        title: event.title.clone(),
                        url,
                        start: event.start.clone(),
                    },
                );

                let manager_state = app.state::<NotificationManagerState<R>>();
                let guard = manager_state.read().await;
                if let Some(manager) = guard.as_ref() {
                    let minutes = (delta.max(0) + 59) / 60;
                    let body = if minutes > 0 {
                        format!("{} starts in about {} minute{}. Open Meetily to join.",
                            event.title, minutes, if minutes == 1 { "" } else { "s" })
                    } else {
                        format!("{} is starting now. Open Meetily to join.", event.title)
                    };
                    let notification = Notification::new(
                        "Meeting starting",
                        body,
                        NotificationType::MeetingReminder(minutes.max(0) as u64),
                    )
                    .with_priority(NotificationPriority::High);
                    if let Err(e) = manager.show_notification(notification).await {
                        log::warn!("Auto-join: notification failed: {}", e);
                    }
                }
            }

            // Keep the dedupe map from growing forever.
            prompted.retain(|_, start| now - *start < 24 * 3600);
        }
    });
}
