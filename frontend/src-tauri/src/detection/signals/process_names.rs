//! Process-name signal: reports meeting apps whose processes indicate an
//! active (or likely) call, mapped onto the same canonical bundle IDs the
//! matcher already understands so the mic-activity state machine, priority
//! ranking, dismissal cooldowns, and notifications all apply unchanged.
//!
//! Ported from MaxwellJryao's `meeting_detector.rs` (af5f8e2) and integrated
//! into the existing DetectionService instead of a parallel poll loop. App
//! list trimmed to Zoom / Teams / Webex / Slack / Discord.
//!
//! Signal strength varies per app:
//! - Zoom and Webex expose a dedicated in-meeting helper process
//!   (`cpthost`, `ciscocollabhost`), a strong "call in progress" signal.
//! - Teams, Slack, and Discord only reveal that the app is running, a weak
//!   signal. This sampler is therefore an opt-in augmentation (see the
//!   "Detect by running apps" preference), default off.

use std::sync::Mutex;

use anyhow::Result;
use sysinfo::System;

use crate::detection::types::MicSnapshot;

/// Detection rule: the app process must be present; if `meeting_indicators`
/// is non-empty one of those must also be present (active-call helper).
struct MeetingApp {
    /// Canonical bundle ID understood by `detection::matcher`.
    bundle_id: &'static str,
    app_processes: &'static [&'static str],
    meeting_indicators: &'static [&'static str],
}

const MEETING_APPS: &[MeetingApp] = &[
    MeetingApp {
        bundle_id: "us.zoom.xos",
        app_processes: &["zoom.us"],
        meeting_indicators: &["cpthost"],
    },
    MeetingApp {
        bundle_id: "com.microsoft.teams2",
        app_processes: &["microsoft teams", "ms-teams", "teams"],
        meeting_indicators: &[],
    },
    MeetingApp {
        bundle_id: "com.cisco.webexmeetingsapp",
        app_processes: &["webex", "webexmta"],
        meeting_indicators: &["ciscocollabhost"],
    },
    MeetingApp {
        bundle_id: "com.tinyspeck.slackmacgap",
        app_processes: &["slack"],
        meeting_indicators: &[],
    },
    MeetingApp {
        bundle_id: "com.hnc.Discord",
        app_processes: &["discord"],
        meeting_indicators: &[],
    },
];

fn has_process(system: &System, patterns: &[&str]) -> bool {
    for process in system.processes().values() {
        let name = process.name().to_string_lossy().to_lowercase();
        for &p in patterns {
            if name.contains(p) {
                return true;
            }
        }
    }
    false
}

/// Polls the process table and reports active meeting apps as pseudo
/// mic-holders (canonical bundle IDs).
pub struct ProcessNameSampler {
    system: Mutex<System>,
}

impl ProcessNameSampler {
    pub fn new() -> Self {
        Self {
            system: Mutex::new(System::new()),
        }
    }

    pub fn snapshot(&self) -> Result<MicSnapshot> {
        let mut system = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("process sampler lock poisoned: {e}"))?;
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut active = Vec::new();
        for app in MEETING_APPS {
            if !has_process(&system, app.app_processes) {
                continue;
            }
            if app.meeting_indicators.is_empty() || has_process(&system, app.meeting_indicators) {
                active.push(app.bundle_id.to_string());
            }
        }

        Ok(MicSnapshot { active_bundles: active })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_bundle_ids_are_known_to_matcher() {
        // The whole design hinges on process hits mapping onto matcher
        // aliases; a typo here would silently downgrade the app to the
        // generic "a meeting" banner with unknown-app sustain timing.
        for app in MEETING_APPS {
            assert!(
                crate::detection::matcher::is_known(app.bundle_id),
                "process-signal bundle id '{}' is not in the matcher allowlist",
                app.bundle_id
            );
        }
    }
}
