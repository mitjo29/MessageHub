use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::error::Result;

/// Structured data extracted from a `05-People/*.md` frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPerson {
    pub file_path: String,
    pub name: String,
    pub role: Option<String>,
    pub tags: Vec<String>,
    pub last_contact: Option<String>,
    /// Discovered addresses grouped by channel (e.g. email → [a@b.com, c@d.com]).
    pub addresses: Vec<PersonAddress>,
    /// Full frontmatter preserved for any downstream consumer that needs it.
    pub frontmatter: serde_yaml::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonAddress {
    /// Channel identifier: "Email", "Telegram", "WhatsApp", "Sms", "Teams".
    /// Matches `types::Channel::to_db_str()` so a direct lookup join works.
    pub channel: String,
    pub address: String,
}

/// Extract a `VaultPerson` from a parsed 05-People file.
///
/// Returns `None` if the file doesn't look like a person profile
/// (no frontmatter name, or `type` present but != "person").
/// This gate lets the indexer safely call `extract_person` on every
/// 05-People file without custom routing logic upstream.
pub fn extract_person(file_path: &str, frontmatter: &Value) -> Result<Option<VaultPerson>> {
    // Require a name. If frontmatter lacks a name field, this isn't a person profile.
    let name = match frontmatter.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return Ok(None),
    };

    // If `type` is present, it must equal "person" (or be absent/None).
    if let Some(t) = frontmatter.get("type").and_then(|v| v.as_str()) {
        if t != "person" {
            return Ok(None);
        }
    }

    let role = frontmatter
        .get("role")
        .and_then(|v| v.as_str())
        .map(String::from);
    let last_contact = frontmatter
        .get("last-contact")
        .and_then(|v| v.as_str())
        .map(String::from);

    let tags = frontmatter
        .get("tags")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let addresses = extract_addresses(frontmatter);

    Ok(Some(VaultPerson {
        file_path: file_path.to_string(),
        name,
        role,
        tags,
        last_contact,
        addresses,
        frontmatter: frontmatter.clone(),
    }))
}

/// Extract addresses from common frontmatter fields.
///
/// Recognized fields:
/// - `email` / `emails` / `email-accounts[*].address` → Email
/// - `telegram` / `telegram-username` → Telegram
/// - `whatsapp` / `phone-whatsapp` → WhatsApp
/// - `sms` / `phone-sms` → Sms
/// - `teams` / `teams-email` → Teams
fn extract_addresses(frontmatter: &Value) -> Vec<PersonAddress> {
    let mut out = Vec::new();

    // Email
    collect_string_or_list(frontmatter.get("email"), &mut out, "Email");
    collect_string_or_list(frontmatter.get("emails"), &mut out, "Email");
    if let Some(accounts) = frontmatter.get("email-accounts").and_then(|v| v.as_sequence()) {
        for acct in accounts {
            if let Some(addr) = acct.get("address").and_then(|v| v.as_str()) {
                out.push(PersonAddress {
                    channel: "Email".to_string(),
                    address: addr.to_string(),
                });
            }
        }
    }

    // Telegram
    collect_string_or_list(frontmatter.get("telegram"), &mut out, "Telegram");
    collect_string_or_list(frontmatter.get("telegram-username"), &mut out, "Telegram");

    // WhatsApp
    collect_string_or_list(frontmatter.get("whatsapp"), &mut out, "WhatsApp");
    collect_string_or_list(frontmatter.get("phone-whatsapp"), &mut out, "WhatsApp");

    // SMS
    collect_string_or_list(frontmatter.get("sms"), &mut out, "Sms");
    collect_string_or_list(frontmatter.get("phone-sms"), &mut out, "Sms");

    // Teams
    collect_string_or_list(frontmatter.get("teams"), &mut out, "Teams");
    collect_string_or_list(frontmatter.get("teams-email"), &mut out, "Teams");

    // Deduplicate while preserving insertion order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|a| seen.insert((a.channel.clone(), a.address.clone())));
    out
}

fn collect_string_or_list(value: Option<&Value>, out: &mut Vec<PersonAddress>, channel: &str) {
    match value {
        Some(Value::String(s)) => {
            out.push(PersonAddress {
                channel: channel.to_string(),
                address: s.clone(),
            });
        }
        Some(Value::Sequence(seq)) => {
            for v in seq {
                if let Some(s) = v.as_str() {
                    out.push(PersonAddress {
                        channel: channel.to_string(),
                        address: s.to_string(),
                    });
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn test_extract_basic_person() {
        let fm = yaml(
            r#"
type: person
name: "Alix Moreau"
role: "Daughter"
tags: [person, family]
last-contact: "2026-04-12"
"#,
        );
        let person = extract_person("05-People/Alix Moreau.md", &fm)
            .unwrap()
            .unwrap();
        assert_eq!(person.name, "Alix Moreau");
        assert_eq!(person.role.as_deref(), Some("Daughter"));
        assert_eq!(person.tags, vec!["person", "family"]);
        assert_eq!(person.last_contact.as_deref(), Some("2026-04-12"));
    }

    #[test]
    fn test_extract_skips_non_person() {
        let fm = yaml(
            r#"
type: project
name: "Project X"
"#,
        );
        assert!(
            extract_person("01-Projects/Project X.md", &fm)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_extract_skips_missing_name() {
        let fm = yaml(
            r#"
type: person
role: "Unknown"
"#,
        );
        assert!(
            extract_person("05-People/Unknown.md", &fm)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_extract_emails_from_string() {
        let fm = yaml(
            r#"
name: "Test"
email: "test@example.com"
"#,
        );
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 1);
        assert_eq!(person.addresses[0].channel, "Email");
        assert_eq!(person.addresses[0].address, "test@example.com");
    }

    #[test]
    fn test_extract_emails_from_list() {
        let fm = yaml(
            r#"
name: "Test"
emails:
  - "a@example.com"
  - "b@example.com"
"#,
        );
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 2);
    }

    #[test]
    fn test_extract_email_accounts_structure() {
        let fm = yaml(
            r#"
name: "Jocelyn"
email-accounts:
  - address: "a@gmail.com"
    provider: gmail
  - address: "b@company.com"
    provider: ms365
"#,
        );
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 2);
        assert!(person.addresses.iter().all(|a| a.channel == "Email"));
    }

    #[test]
    fn test_extract_multiple_channels() {
        let fm = yaml(
            r#"
name: "Test"
email: "t@example.com"
telegram: "@testuser"
whatsapp: "+491234567"
"#,
        );
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        let channels: Vec<&str> = person.addresses.iter().map(|a| a.channel.as_str()).collect();
        assert!(channels.contains(&"Email"));
        assert!(channels.contains(&"Telegram"));
        assert!(channels.contains(&"WhatsApp"));
    }

    #[test]
    fn test_duplicates_are_deduplicated() {
        let fm = yaml(
            r#"
name: "Test"
email: "t@example.com"
emails:
  - "t@example.com"
  - "other@example.com"
"#,
        );
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 2);
    }
}
