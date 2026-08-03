//! macOS EventKit bridge (objc2-event-kit).
//!
//! All functions here are synchronous and expected to run on a blocking
//! thread (`tokio::task::spawn_blocking`); Objective-C handles never cross
//! threads or await points. `EKEventStore` is documented thread-safe by
//! Apple, so using it off the main thread is supported.

use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
use objc2_foundation::{NSDate, NSError};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

/// Plain-data snapshot of one EKEvent, safe to send across threads.
pub struct RawCalendarEvent {
    pub id: String,
    pub title: String,
    pub start_unix: f64,
    pub end_unix: f64,
    pub organizer: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
}

/// Maps the EventKit authorization status to a stable string for the frontend.
pub fn permission_status() -> &'static str {
    // SAFETY: class method with no arguments; no preconditions beyond linking
    // EventKit, which objc2-event-kit guarantees.
    let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
    if status == EKAuthorizationStatus::NotDetermined {
        "not_determined"
    } else if status == EKAuthorizationStatus::Denied {
        "denied"
    } else if status == EKAuthorizationStatus::Restricted {
        "restricted"
    } else if status == EKAuthorizationStatus::WriteOnly {
        "write_only"
    } else {
        // FullAccess (== the deprecated Authorized alias).
        "full_access"
    }
}

/// Requests calendar access, blocking the current (non-main) thread until the
/// user answers or the request times out. macOS shows the consent dialog at
/// most once; subsequent calls return the stored decision immediately.
///
/// Uses the pre-macOS-14 `requestAccessToEntityType:completion:` selector
/// (deprecated but fully functional on 14+) because the modern
/// `requestFullAccessToEventsWithCompletion:` selector does not exist on
/// macOS 13, which this app still supports.
pub fn request_access_blocking() -> Result<bool, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<bool, String>>();

    // Keep the store alive in this scope while we wait: dropping it before the
    // completion fires could cancel the consent dialog.
    // SAFETY: `init` on a freshly allocated EKEventStore is the documented
    // initializer and has no preconditions.
    let store = unsafe { EKEventStore::init(EKEventStore::alloc()) };

    let tx_slot: Mutex<Option<std::sync::mpsc::Sender<Result<bool, String>>>> =
        Mutex::new(Some(tx));
    let block = RcBlock::new(move |granted: objc2::runtime::Bool, error: *mut NSError| {
        let result = if !error.is_null() {
            // SAFETY: non-null (checked above) and EventKit keeps the NSError
            // alive for the duration of this callback.
            let message = unsafe { (*error).localizedDescription().to_string() };
            Err(format!("Calendar access request failed: {}", message))
        } else {
            Ok(granted.as_bool())
        };
        if let Some(tx) = tx_slot.lock().ok().and_then(|mut guard| guard.take()) {
            let _ = tx.send(result);
        }
    });

    // SAFETY: the block pointer is valid for the duration of the call and
    // EventKit copies the block for the async completion; RcBlock's heap
    // allocation satisfies the block ABI.
    unsafe {
        store.requestAccessToEntityType_completion(
            EKEntityType::Event,
            &*block as *const _ as *mut _,
        );
    }

    // Generous timeout: the user may leave the consent dialog open a while.
    match rx.recv_timeout(Duration::from_secs(300)) {
        Ok(result) => {
            info!("Calendar access request completed: {:?}", result);
            result
        }
        Err(_) => {
            warn!("Calendar access request timed out");
            Err("Timed out waiting for the calendar permission dialog".to_string())
        }
    }
}

/// Fetches events starting within the next `window_hours`, including events
/// already in progress. Requires prior authorization; without it EventKit
/// simply returns an empty calendar set.
pub fn upcoming_events_blocking(window_hours: f64) -> Result<Vec<RawCalendarEvent>, String> {
    let status = permission_status();
    if status != "full_access" {
        return Err(format!(
            "Calendar access is not granted (status: {})",
            status
        ));
    }

    // SAFETY: documented initializer, no preconditions.
    let store = unsafe { EKEventStore::init(EKEventStore::alloc()) };

    let start = NSDate::date();
    let end = start.dateByAddingTimeInterval(window_hours * 3600.0);

    // SAFETY: dates are valid NSDate instances; `None` calendars means "all
    // calendars", per the EventKit contract for this predicate constructor.
    let predicate =
        unsafe { store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None) };
    // SAFETY: the predicate was created by the same store's predicate
    // constructor, which is the documented requirement.
    let events = unsafe { store.eventsMatchingPredicate(&predicate) };

    let mut results = Vec::new();
    for event in events.to_vec() {
        // SAFETY: property getters on a live EKEvent returned by the store;
        // EventKit guarantees these objects outlive this synchronous scope.
        unsafe {
            let id = event
                .eventIdentifier()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let title = event.title().to_string();
            let start_unix = event.startDate().timeIntervalSince1970();
            let end_unix = event.endDate().timeIntervalSince1970();
            // Kept closure-free: the generated getters are `unsafe fn`s and an
            // outer `unsafe` block does not extend into closure bodies.
            let organizer = match event.organizer() {
                Some(participant) => participant.name().map(|s| s.to_string()),
                None => None,
            };
            let location = event.location().map(|s| s.to_string());
            let notes = event.notes().map(|s| s.to_string());
            let url = event.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string());

            results.push(RawCalendarEvent {
                id,
                title,
                start_unix,
                end_unix,
                organizer,
                location,
                notes,
                url,
            });
        }
    }

    info!(
        "Calendar query returned {} event(s) in the next {} hours",
        results.len(),
        window_hours
    );
    Ok(results)
}
