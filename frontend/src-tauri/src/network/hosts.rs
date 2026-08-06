//! The complete set of hosts this build can ever contact, and what each is for.
//!
//! This is a hand-maintained inventory, not a generated one, and the panel that
//! shows it says so. It exists so an operator can diff it against their own
//! firewall or DNS log: if something appears there that is not here, that is a
//! finding worth chasing, and the list is short enough to check by eye.
//!
//! Pure data plus pure functions, so the classification rules are testable
//! without a build of the app.

use serde::Serialize;

/// Why the app would talk to a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Downloading a transcription, diarization, summary, or embedding model.
    ModelDownload,
    /// Sending prompt text to a language model.
    LlmCall,
    /// Sending recorded audio to a transcription service.
    Transcription,
    /// Microsoft Graph: calendar reads and sign-in.
    GraphApi,
    /// Posting a summary to a Slack or Teams webhook the operator configured.
    ShareWebhook,
    /// Asking whether a newer release exists.
    UpdateCheck,
    /// Listing a provider's available models, or checking a key works.
    ProviderMetadata,
    /// The licence and profile endpoint.
    LicenseCheck,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelDownload => "model_download",
            Self::LlmCall => "llm_call",
            Self::Transcription => "transcription",
            Self::GraphApi => "graph_api",
            Self::ShareWebhook => "share_webhook",
            Self::UpdateCheck => "update_check",
            Self::ProviderMetadata => "provider_metadata",
            Self::LicenseCheck => "license_check",
        }
    }

    /// One plain-English line, for a panel a non-engineer has to read.
    pub fn describe(self) -> &'static str {
        match self {
            Self::ModelDownload => "Downloading a speech or language model file",
            Self::LlmCall => "Sending text to a language model to write a summary or answer",
            Self::Transcription => "Sending recorded audio out to be transcribed",
            Self::GraphApi => "Reading your Microsoft 365 calendar, or signing in to it",
            Self::ShareWebhook => "Posting a summary to a chat channel you configured",
            Self::UpdateCheck => "Checking whether a newer version of the app exists",
            Self::ProviderMetadata => "Listing a provider's models, or testing that a key works",
            Self::LicenseCheck => "Checking the licence attached to this install",
        }
    }

    /// Whether a request for this purpose carries recorded audio off the device.
    pub fn carries_audio(self) -> bool {
        matches!(self, Self::Transcription)
    }

    /// Whether a request for this purpose carries transcript or summary text off
    /// the device.
    pub fn carries_transcript(self) -> bool {
        matches!(self, Self::LlmCall | Self::ShareWebhook)
    }
}

/// One entry in the inventory.
#[derive(Debug, Clone, Serialize)]
pub struct ExpectedHost {
    pub host: String,
    pub purpose: Purpose,
    /// What the app does with this host, in plain English.
    pub what_for: String,
    /// True when the app only reaches this host because the operator configured
    /// or asked for it (a cloud provider they selected, a webhook they pasted).
    pub only_when_configured: bool,
    /// True when the traffic never leaves the machine.
    pub on_device: bool,
}

fn entry(
    host: &str,
    purpose: Purpose,
    what_for: &str,
    only_when_configured: bool,
    on_device: bool,
) -> ExpectedHost {
    ExpectedHost {
        host: host.to_string(),
        purpose,
        what_for: what_for.to_string(),
        only_when_configured,
        on_device,
    }
}

/// Every host this build can contact.
pub fn expected() -> Vec<ExpectedHost> {
    vec![
        entry(
            "huggingface.co",
            Purpose::ModelDownload,
            "Downloads the Whisper, Parakeet, built-in AI, and semantic search model files. Downloads only; nothing is uploaded.",
            false,
            false,
        ),
        entry(
            "cdn-lfs.huggingface.co",
            Purpose::ModelDownload,
            "Where huggingface.co redirects large model files. Same download, different server.",
            false,
            false,
        ),
        entry(
            "cdn-lfs-us-1.hf.co",
            Purpose::ModelDownload,
            "A second Hugging Face file server that large downloads can be redirected to.",
            false,
            false,
        ),
        entry(
            "github.com",
            Purpose::ModelDownload,
            "Downloads the speaker diarization models from the k2-fsa project's releases, and holds the release file the update check reads.",
            false,
            false,
        ),
        entry(
            "objects.githubusercontent.com",
            Purpose::ModelDownload,
            "Where github.com redirects release file downloads.",
            false,
            false,
        ),
        entry(
            "meetily.towardsgeneralintelligence.com",
            Purpose::ModelDownload,
            "A mirror for one Parakeet transcription model, and the licence endpoint.",
            false,
            false,
        ),
        entry(
            "api.openai.com",
            Purpose::LlmCall,
            "OpenAI. Reached only if you pick OpenAI as your summary model or your transcription provider; carries transcript text, or audio when used for transcription.",
            true,
            false,
        ),
        entry(
            "api.anthropic.com",
            Purpose::LlmCall,
            "Anthropic (Claude). Reached only if you pick Claude as your summary model; carries transcript text.",
            true,
            false,
        ),
        entry(
            "api.groq.com",
            Purpose::LlmCall,
            "Groq. Reached only if you pick Groq as your summary model; carries transcript text.",
            true,
            false,
        ),
        entry(
            "openrouter.ai",
            Purpose::LlmCall,
            "OpenRouter. Reached only if you pick OpenRouter as your summary model; carries transcript text.",
            true,
            false,
        ),
        entry(
            "localhost:11434",
            Purpose::LlmCall,
            "Ollama running on this machine. Traffic stays on the loopback interface and never reaches a network.",
            true,
            true,
        ),
        entry(
            "127.0.0.1",
            Purpose::LlmCall,
            "The built-in AI sidecar running on this machine. Loopback only.",
            false,
            true,
        ),
        entry(
            "graph.microsoft.com",
            Purpose::GraphApi,
            "Microsoft Graph. Reached only if you connect a Microsoft 365 account; reads calendar entries. No meeting content is sent.",
            true,
            false,
        ),
        entry(
            "login.microsoftonline.com",
            Purpose::GraphApi,
            "Microsoft sign-in. Reached only while connecting or refreshing a Microsoft 365 account.",
            true,
            false,
        ),
        entry(
            "hooks.slack.com",
            Purpose::ShareWebhook,
            "A Slack incoming webhook you pasted in. Reached only when you share a summary; carries the summary text.",
            true,
            false,
        ),
        entry(
            "*.webhook.office.com",
            Purpose::ShareWebhook,
            "A Microsoft Teams incoming webhook you pasted in. The exact host is whatever your webhook URL points at. Reached only when you share a summary; carries the summary text.",
            true,
            false,
        ),
        entry(
            "your own endpoint",
            Purpose::LlmCall,
            "If you configure a custom OpenAI-compatible endpoint or a self-hosted transcription endpoint, the app reaches whatever host you typed and nothing else.",
            true,
            false,
        ),
    ]
}

/// Splits a URL into the host (with a port when it is not the default) and the
/// scheme-plus-path, dropping credentials, query string, and fragment.
///
/// Query strings and fragments are dropped rather than stored because that is
/// exactly where API keys and search terms live; a transparency log that leaks
/// secrets when exported is worse than no log.
pub fn split_url(url: &str) -> (String, String) {
    let (scheme, rest) = match url.find("://") {
        Some(index) => (&url[..index], &url[index + 3..]),
        None => ("", url),
    };
    // Strip any userinfo (user:password@) before the host.
    let rest = match rest.find('@') {
        Some(at) if !rest[..at].contains('/') => &rest[at + 1..],
        _ => rest,
    };
    let end = rest
        .find(['/', '?', '#'])
        .unwrap_or(rest.len());
    let authority = &rest[..end];
    let path_end = rest[end..]
        .find(['?', '#'])
        .map(|offset| end + offset)
        .unwrap_or(rest.len());
    let path = &rest[end..path_end];

    let host = strip_default_port(scheme, authority);
    let sanitized = if scheme.is_empty() {
        format!("{}{}", authority, path)
    } else {
        format!("{}://{}{}", scheme, authority, path)
    };
    (host, sanitized)
}

fn strip_default_port(scheme: &str, authority: &str) -> String {
    let default = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => return authority.to_string(),
    };
    authority
        .strip_suffix(default)
        .unwrap_or(authority)
        .to_string()
}

/// True when a host is on the machine itself. Loopback traffic is reported (an
/// operator should see that Ollama was called) but flagged as never leaving.
pub fn is_on_device(host: &str) -> bool {
    let bare = host.split(':').next().unwrap_or(host);
    matches!(bare, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
        || bare.ends_with(".localhost")
}

/// True when a host appears in the inventory above, allowing for the wildcard
/// entries. A false answer is the interesting case: it means the app reached
/// somewhere the inventory does not describe.
pub fn is_expected(host: &str) -> bool {
    if is_on_device(host) {
        return true;
    }
    let bare = host.split(':').next().unwrap_or(host);
    expected().iter().any(|entry| {
        let pattern = entry.host.split(':').next().unwrap_or(&entry.host);
        match pattern.strip_prefix("*.") {
            Some(suffix) => bare == suffix || bare.ends_with(&format!(".{}", suffix)),
            None => bare == pattern,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_splits_into_host_and_path_with_the_query_dropped() {
        let (host, sanitized) =
            split_url("https://api.openai.com/v1/chat/completions?key=secret#frag");
        assert_eq!(host, "api.openai.com");
        assert_eq!(sanitized, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn credentials_in_a_url_never_reach_the_log() {
        let (host, sanitized) = split_url("https://user:pass@example.test/path");
        assert_eq!(host, "example.test");
        assert_eq!(sanitized, "https://example.test/path");
    }

    #[test]
    fn a_non_default_port_stays_part_of_the_host_and_a_default_one_does_not() {
        assert_eq!(split_url("http://localhost:11434/api/tags").0, "localhost:11434");
        assert_eq!(split_url("https://example.test:443/x").0, "example.test");
        assert_eq!(split_url("http://example.test:80/x").0, "example.test");
    }

    #[test]
    fn a_url_with_no_path_still_splits_cleanly() {
        let (host, sanitized) = split_url("https://hooks.slack.com");
        assert_eq!(host, "hooks.slack.com");
        assert_eq!(sanitized, "https://hooks.slack.com");
    }

    #[test]
    fn loopback_hosts_are_recognised_as_on_device() {
        assert!(is_on_device("localhost:11434"));
        assert!(is_on_device("127.0.0.1"));
        assert!(!is_on_device("api.openai.com"));
    }

    #[test]
    fn inventory_membership_covers_wildcards_and_rejects_strangers() {
        assert!(is_expected("api.anthropic.com"));
        assert!(is_expected("huggingface.co"));
        assert!(is_expected("contoso.webhook.office.com"));
        assert!(is_expected("localhost:11434"));
        assert!(!is_expected("telemetry.example.test"));
    }

    #[test]
    fn every_inventory_entry_says_what_it_is_for() {
        for entry in expected() {
            assert!(!entry.host.trim().is_empty());
            assert!(entry.what_for.len() > 20, "{} needs a real explanation", entry.host);
        }
    }

    #[test]
    fn only_audio_and_text_purposes_are_marked_as_carrying_content() {
        assert!(Purpose::Transcription.carries_audio());
        assert!(!Purpose::Transcription.carries_transcript());
        assert!(Purpose::LlmCall.carries_transcript());
        assert!(Purpose::ShareWebhook.carries_transcript());
        assert!(!Purpose::ModelDownload.carries_audio());
        assert!(!Purpose::ModelDownload.carries_transcript());
        assert!(!Purpose::UpdateCheck.carries_transcript());
    }
}
