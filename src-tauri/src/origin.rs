//! Where a download came from (spec 5.1.1). Browsers on Windows write a
//! `Zone.Identifier` alternate data stream with the host the file was fetched
//! from; this module reads it and turns the host into a label she recognizes.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// "Messenger", "Gmail", "Google Drive", "DepEd", a bare domain, or "Unknown source".
    pub label: String,
    /// The host the label was derived from, when known.
    pub host: Option<String>,
}

impl Origin {
    pub fn unknown() -> Self {
        Self { label: "Unknown source".to_string(), host: None }
    }
}

/// Reads the stream and maps it. Missing or unreadable streams are a normal
/// case (apps that save files directly), not an error.
pub fn read_origin(path: &Path) -> Origin {
    let stream = format!("{}:Zone.Identifier", path.to_string_lossy());
    match std::fs::read_to_string(stream) {
        Ok(text) => from_zone_text(&text),
        Err(_) => Origin::unknown(),
    }
}

/// Parses the INI-like stream text. `HostUrl` is preferred; `ReferrerUrl`
/// fills in when the host is a bare storage domain (Gmail attachments come
/// from googleusercontent.com with a mail.google.com referrer).
pub fn from_zone_text(text: &str) -> Origin {
    let mut host_url = None;
    let mut referrer = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("HostUrl=") {
            host_url = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("ReferrerUrl=") {
            referrer = Some(value.trim());
        }
    }
    let host = host_url.and_then(host_of);
    let referrer_host = referrer.and_then(host_of);
    match (host, referrer_host) {
        (Some(h), r) => {
            let label = label_for_host(&h);
            if label == h {
                // Unlabelled storage host: the referrer usually names the real site.
                if let Some(r) = r {
                    let via = label_for_host(&r);
                    if via != r {
                        return Origin { label: via, host: Some(r) };
                    }
                }
            }
            Origin { label, host: Some(h) }
        }
        (None, Some(r)) => Origin { label: label_for_host(&r), host: Some(r) },
        (None, None) => Origin::unknown(),
    }
}

/// The host part of a URL, lowercased, without a port or userinfo.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// The table from the spec. Anything unmatched becomes the bare domain.
pub fn label_for_host(host: &str) -> String {
    const TABLE: &[(&str, &str)] = &[
        ("messenger.com", "Messenger"),
        ("facebook.com", "Messenger"),
        ("fbcdn.net", "Messenger"),
        ("fbsbx.com", "Messenger"),
        ("mail.google.com", "Gmail"),
        ("mail-attachment.googleusercontent.com", "Gmail"),
        ("drive.google.com", "Google Drive"),
        ("docs.google.com", "Google Drive"),
        ("drive.usercontent.google.com", "Google Drive"),
        ("deped.gov.ph", "DepEd"),
    ];
    for (needle, label) in TABLE {
        if host == *needle || host.ends_with(&format!(".{needle}")) {
            return (*label).to_string();
        }
    }
    bare_domain(host)
}

/// `www.example.com` becomes `example.com`; other subdomains are kept.
fn bare_domain(host: &str) -> String {
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_hosts() {
        assert_eq!(label_for_host("www.messenger.com"), "Messenger");
        assert_eq!(label_for_host("scontent.xx.fbcdn.net"), "Messenger");
        assert_eq!(label_for_host("mail.google.com"), "Gmail");
        assert_eq!(label_for_host("drive.google.com"), "Google Drive");
        assert_eq!(label_for_host("lrmds.deped.gov.ph"), "DepEd");
        assert_eq!(label_for_host("www.example.com"), "example.com");
    }

    #[test]
    fn parses_zone_text_and_uses_the_referrer_for_storage_hosts() {
        let text = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://mail.google.com/mail/u/0/\r\nHostUrl=https://mail-attachment.googleusercontent.com/attachment/u/0/?x=1\r\n";
        let origin = from_zone_text(text);
        assert_eq!(origin.label, "Gmail");

        let text = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://www.facebook.com/messages/t/123\r\nHostUrl=https://cdn.fbsbx.com/v/t59/file.docx?x=1\r\n";
        assert_eq!(from_zone_text(text).label, "Messenger");

        let text = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://www.facebook.com/\r\nHostUrl=https://files.somecdn.io/abc\r\n";
        assert_eq!(from_zone_text(text).label, "Messenger");

        assert_eq!(from_zone_text("[ZoneTransfer]\r\nZoneId=3\r\n"), Origin::unknown());
    }

    #[test]
    fn host_of_handles_ports_and_paths() {
        assert_eq!(host_of("https://Example.com:8080/a/b").unwrap(), "example.com");
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn missing_stream_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(read_origin(&file), Origin::unknown());
    }
}
