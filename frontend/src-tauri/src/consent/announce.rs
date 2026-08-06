//! Spoken consent announcement.
//!
//! Plays a short text-to-speech line through the system's current output
//! device, so people on the call hear it the same way they hear anyone else.
//! There is no bot in the meeting, so this and the pasteable disclaimer are the
//! only two mechanisms that reach the other participants at all — and the
//! clipboard fallback is the one that works on every platform and every
//! meeting service, which is why it is always offered alongside this.
//!
//! Security note: the announcement text is operator-editable, so it is never
//! interpolated into a shell string. macOS passes it as a single `execve`
//! argument; Windows hands PowerShell a temp file path through the environment
//! and lets PowerShell read the file, so no part of the text is ever parsed as
//! script.

/// Announcements are one or two sentences. The cap keeps a paste accident from
/// tying up the speech synthesiser for minutes.
const MAX_ANNOUNCEMENT_CHARS: usize = 400;

/// Rejects text that cannot sensibly be spoken, and trims it to the cap.
/// Control characters are stripped rather than rejected so a copied line with a
/// stray newline still works.
pub fn sanitize(text: &str) -> Result<String, String> {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return Err("Announcement text is empty".to_string());
    }
    Ok(cleaned.chars().take(MAX_ANNOUNCEMENT_CHARS).collect())
}

/// Speaks `text` through the current output device.
///
/// Returns Err with a plain-English reason when the platform has no supported
/// path or the speech process fails; callers log the gap and fall back to the
/// clipboard disclaimer.
pub fn speak(text: &str) -> Result<(), String> {
    let text = sanitize(text)?;
    speak_platform(&text)
}

#[cfg(target_os = "macos")]
fn speak_platform(text: &str) -> Result<(), String> {
    use std::process::Command;

    // `say` is part of macOS and routes to the default output device. The text
    // is a single argument, never a shell word.
    let status = Command::new("/usr/bin/say")
        .arg("--")
        .arg(text)
        .status()
        .map_err(|e| format!("Could not run the macOS speech tool: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "The macOS speech tool exited with status {}",
            status.code().unwrap_or(-1)
        ))
    }
}

#[cfg(target_os = "windows")]
fn speak_platform(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Command;

    // Write the announcement to a temp file and let PowerShell read it, so the
    // operator's text never becomes part of the command line.
    let mut path = std::env::temp_dir();
    path.push(format!("meetily-consent-{}.txt", uuid::Uuid::new_v4()));

    {
        let mut file = std::fs::File::create(&path)
            .map_err(|e| format!("Could not stage the announcement text: {}", e))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("Could not stage the announcement text: {}", e))?;
    }

    const SCRIPT: &str = "Add-Type -AssemblyName System.Speech; \
         $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $synth.Speak([System.IO.File]::ReadAllText($env:MEETILY_CONSENT_TEXT))";

    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ])
        .env("MEETILY_CONSENT_TEXT", &path)
        .status()
        .map_err(|e| format!("Could not run Windows speech synthesis: {}", e));

    let _ = std::fs::remove_file(&path);

    match result? {
        status if status.success() => Ok(()),
        status => Err(format!(
            "Windows speech synthesis exited with status {}",
            status.code().unwrap_or(-1)
        )),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn speak_platform(_text: &str) -> Result<(), String> {
    Err("Spoken announcements are not supported on this platform yet. Copy the disclaimer instead.".to_string())
}

/// Whether this build has a spoken-announcement path at all, so the UI can
/// hide the Test button instead of offering a control that always fails.
pub fn is_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_collapses_whitespace_and_control_characters() {
        assert_eq!(
            sanitize("This meeting\nis being\t transcribed.").unwrap(),
            "This meeting is being transcribed."
        );
    }

    #[test]
    fn sanitize_rejects_empty_text() {
        assert!(sanitize("").is_err());
        assert!(sanitize("   \n\t ").is_err());
    }

    #[test]
    fn sanitize_caps_runaway_text() {
        let long = "a ".repeat(1000);
        assert_eq!(sanitize(&long).unwrap().chars().count(), MAX_ANNOUNCEMENT_CHARS);
    }

    #[test]
    fn sanitize_preserves_shell_metacharacters_verbatim() {
        // Nothing is escaped or stripped: the text is passed as an argument or
        // a file, never as shell input, so quoting it would only mangle it.
        let text = "Recording; $(whoami) `id` && echo \"hi\"";
        assert_eq!(sanitize(text).unwrap(), text);
    }
}
