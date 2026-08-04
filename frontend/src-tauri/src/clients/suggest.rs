//! Pure client-suggestion logic: attendee-domain matching and fuzzy title
//! matching. Kept free of I/O so it is unit-testable; the command in
//! `commands.rs` gathers the inputs (clients, calendar attendees) and calls in.

use crate::database::models::Client;

/// Why a client was suggested, surfaced verbatim in the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientSuggestion {
    pub client_id: String,
    pub client_name: String,
    pub reason: String,
}

/// Extracts the lowercased domain part of an email address.
pub fn email_domain(email: &str) -> Option<String> {
    let at = email.rfind('@')?;
    let domain = email[at + 1..].trim().trim_end_matches('.').to_lowercase();
    (!domain.is_empty() && domain.contains('.')).then_some(domain)
}

fn normalize_domain(domain: &str) -> String {
    domain
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("www.")
        .trim_end_matches('/')
        .to_lowercase()
}

/// First client whose configured domain matches any attendee email domain
/// (exact or subdomain match, case-insensitive).
pub fn suggest_by_domain<'a>(
    clients: &'a [Client],
    attendee_domains: &[String],
) -> Option<(&'a Client, String)> {
    for client in clients {
        let Some(raw) = client.domain.as_deref() else {
            continue;
        };
        let client_domain = normalize_domain(raw);
        if client_domain.is_empty() {
            continue;
        }
        for attendee in attendee_domains {
            let attendee = attendee.to_lowercase();
            if attendee == client_domain || attendee.ends_with(&format!(".{}", client_domain)) {
                return Some((client, format!("meeting attendee at @{}", attendee)));
            }
        }
    }
    None
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(str::to_string)
        .collect()
}

/// Client whose name appears in the meeting title: every name token (words of
/// 2+ characters) must appear as a title token. The longest-named match wins so
/// "Acme Europe" beats "Acme" when both fit.
pub fn suggest_by_title<'a>(clients: &'a [Client], title: &str) -> Option<(&'a Client, String)> {
    let title_tokens = tokens(title);
    if title_tokens.is_empty() {
        return None;
    }
    let mut best: Option<(&Client, usize)> = None;
    for client in clients {
        let name_tokens = tokens(&client.name);
        if name_tokens.is_empty() {
            continue;
        }
        let all_present = name_tokens.iter().all(|t| title_tokens.contains(t));
        if all_present {
            let score = name_tokens.len();
            if best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((client, score));
            }
        }
    }
    best.map(|(client, _)| {
        (
            client,
            format!("meeting title mentions \"{}\"", client.name),
        )
    })
}

/// Combines both signals: attendee domains first (stronger evidence), then
/// title match.
pub fn suggest(
    clients: &[Client],
    title: &str,
    attendee_domains: &[String],
) -> Option<ClientSuggestion> {
    let hit = suggest_by_domain(clients, attendee_domains)
        .or_else(|| suggest_by_title(clients, title));
    hit.map(|(client, reason)| ClientSuggestion {
        client_id: client.id.clone(),
        client_name: client.name.clone(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn client(id: &str, name: &str, domain: Option<&str>) -> Client {
        Client {
            id: id.to_string(),
            name: name.to_string(),
            domain: domain.map(str::to_string),
            notes: String::new(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn email_domain_parses_and_rejects() {
        assert_eq!(email_domain("dana@acme.com").as_deref(), Some("acme.com"));
        assert_eq!(email_domain("A@B.CO").as_deref(), Some("b.co"));
        assert_eq!(email_domain("not-an-email"), None);
        assert_eq!(email_domain("trailing@dot."), None);
        assert_eq!(email_domain("no@tld"), None);
    }

    #[test]
    fn domain_match_is_exact_or_subdomain() {
        let clients = vec![client("c1", "Acme", Some("acme.com"))];
        assert!(suggest_by_domain(&clients, &["acme.com".to_string()]).is_some());
        assert!(suggest_by_domain(&clients, &["mail.acme.com".to_string()]).is_some());
        assert!(suggest_by_domain(&clients, &["notacme.com".to_string()]).is_none());
        assert!(suggest_by_domain(&clients, &["acme.company".to_string()]).is_none());
    }

    #[test]
    fn domain_match_tolerates_url_style_configuration() {
        let clients = vec![client("c1", "Acme", Some("https://www.Acme.com/"))];
        let (matched, _) = suggest_by_domain(&clients, &["acme.com".to_string()]).unwrap();
        assert_eq!(matched.id, "c1");
    }

    #[test]
    fn clients_without_domains_are_skipped() {
        let clients = vec![client("c1", "Acme", None), client("c2", "Beta", Some(""))];
        assert!(suggest_by_domain(&clients, &["acme.com".to_string()]).is_none());
    }

    #[test]
    fn title_match_requires_all_name_tokens() {
        let clients = vec![
            client("c1", "Acme", None),
            client("c2", "Acme Europe", None),
        ];
        let (matched, _) = suggest_by_title(&clients, "Q3 review with Acme Europe team").unwrap();
        assert_eq!(matched.id, "c2", "longer name match should win");

        let (matched, _) = suggest_by_title(&clients, "Acme weekly sync").unwrap();
        assert_eq!(matched.id, "c1");

        assert!(suggest_by_title(&clients, "Internal standup").is_none());
    }

    #[test]
    fn title_match_is_case_and_punctuation_insensitive() {
        let clients = vec![client("c1", "Blue Sky Media", None)];
        assert!(suggest_by_title(&clients, "blue-sky/media: kickoff!").is_some());
    }

    #[test]
    fn combined_prefers_domain_over_title() {
        let clients = vec![
            client("c1", "Acme", Some("acme.com")),
            client("c2", "Beta", Some("beta.io")),
        ];
        let suggestion = suggest(&clients, "Acme sync", &["beta.io".to_string()]).unwrap();
        assert_eq!(suggestion.client_id, "c2");
        assert!(suggestion.reason.contains("@beta.io"));

        let suggestion = suggest(&clients, "Acme sync", &[]).unwrap();
        assert_eq!(suggestion.client_id, "c1");
    }

    #[test]
    fn no_signal_yields_no_suggestion() {
        let clients = vec![client("c1", "Acme", Some("acme.com"))];
        assert!(suggest(&clients, "1:1 with Sam", &[]).is_none());
    }
}
