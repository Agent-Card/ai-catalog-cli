// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// SPDX-License-Identifier: Apache-2.0

//! Fetching and reporting for `ai-catalog verify`.
//!
//! The checks themselves live in `ai_catalog_validate::verify`. This module
//! supplies the [`Fetch`] it needs, following the specification's safe-fetching
//! rules, and renders the result.

use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use ai_catalog_validate::{EntryTrust, Fetch, ManifestTrust, Outcome, Verification};
use serde_json::{Value, json};
use url::Url;

const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetches DID documents and artifacts over HTTPS, with local overrides.
pub struct SafeFetch {
    /// Bodies to return for these URLs instead of fetching them.
    pub overrides: BTreeMap<String, Vec<u8>>,
    /// Fail every fetch that has no override.
    pub offline: bool,
}

impl Fetch for SafeFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        if let Some(body) = self.overrides.get(url) {
            return Ok(body.clone());
        }
        if self.offline {
            return Err(format!("not fetched: {url} (--offline)"));
        }
        fetch(url)
    }
}

/// The URL a root `did:web` DID resolves to, for `--did-document` overrides.
pub fn did_document_url(did: &str) -> Result<String, String> {
    let host = did
        .strip_prefix("did:web:")
        .ok_or_else(|| format!("{did:?} is not a did:web DID"))?;
    let valid = !host.is_empty()
        && !host.contains([':', '%', '/'])
        && !host.ends_with('.')
        && host.parse::<IpAddr>().is_err()
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
    if valid {
        Ok(format!("https://{host}/.well-known/did.json"))
    } else {
        Err(format!(
            "{did:?} must be a root did:web DID with a lowercase domain"
        ))
    }
}

/// Parses `url` and checks it may be fetched: `https`, with a domain name host.
fn checked_target(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|error| format!("invalid URL {url}: {error}"))?;
    if parsed.scheme() != "https" {
        return Err(format!("refusing non-https URL {url}"));
    }
    match parsed.host() {
        Some(url::Host::Domain(_)) => Ok(parsed),
        Some(_) => Err(format!("refusing IP address host in {url}")),
        None => Err(format!("URL {url} has no host")),
    }
}

/// Fetches a URL under the safe-fetching rules: HTTPS only, no redirects, a
/// size and time limit, and no private, loopback, or link-local addresses. The
/// checked addresses are pinned so the connection cannot be re-resolved elsewhere.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let target = checked_target(url)?;
    let host = target.host_str().unwrap_or_default().to_owned();
    let host = host.as_str();
    let port = target.port_or_known_default().unwrap_or(443);
    let addresses: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("cannot resolve {host}: {error}"))?
        .collect();
    if addresses.is_empty() {
        return Err(format!("{host} has no addresses"));
    }
    if let Some(address) = addresses.iter().find(|address| is_disallowed(address.ip())) {
        return Err(format!(
            "{host} resolves to disallowed address {}",
            address.ip()
        ));
    }

    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(target)
        .send()
        .map_err(|error| format!("cannot fetch {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{url} returned {}", response.status()));
    }
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(format!("{url} exceeds {MAX_RESPONSE_BYTES} bytes"));
    }
    Ok(body)
}

fn is_disallowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_disallowed(IpAddr::V4(v4)))
        }
    }
}

/// Whether any check failed, and whether any could not run.
pub fn summarize(verification: &Verification) -> (bool, bool) {
    let mut outcomes: Vec<&Outcome> = verification.catalog_signature.iter().collect();
    for entry in &verification.entries {
        outcomes.extend(entry.artifact.iter());
        for manifest in &entry.manifests {
            outcomes.extend(manifest.signature.iter());
            outcomes.extend(manifest.release.iter());
        }
    }
    let failed = !verification.validation.is_valid || outcomes.iter().any(|o| o.is_failed());
    let unverifiable = outcomes
        .iter()
        .any(|o| matches!(o, Outcome::Unverifiable(_)));
    (failed, unverifiable)
}

fn outcome_json(outcome: Option<&Outcome>) -> Value {
    match outcome {
        None => Value::Null,
        Some(Outcome::Verified) => json!({ "result": "verified" }),
        Some(Outcome::Failed(reason)) => json!({ "result": "failed", "reason": reason }),
        Some(Outcome::Unverifiable(reason)) => {
            json!({ "result": "unverifiable", "reason": reason })
        }
    }
}

fn manifest_json(manifest: &ManifestTrust) -> Value {
    json!({
        "index": manifest.index,
        "contributor": manifest.contributor,
        "signer": manifest.signer,
        "publisher": manifest.publisher,
        "signature": outcome_json(manifest.signature.as_ref()),
        "release": outcome_json(manifest.release.as_ref()),
    })
}

fn entry_json(entry: &EntryTrust) -> Value {
    json!({
        "index": entry.index,
        "identifier": entry.identifier,
        "publisherEndorsed": entry.publisher_endorsed,
        "artifact": outcome_json(entry.artifact.as_ref()),
        "manifests": entry.manifests.iter().map(manifest_json).collect::<Vec<_>>(),
    })
}

/// The JSON report printed by `verify --json`.
pub fn report_json(verification: &Verification) -> Value {
    json!({
        "valid": verification.validation.is_valid,
        "conformance": verification.conformance_level.to_string(),
        "errors": verification
            .validation
            .errors
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>(),
        "catalogSignature": outcome_json(verification.catalog_signature.as_ref()),
        "catalogAuthorized": verification.catalog_authorized,
        "entries": verification.entries.iter().map(entry_json).collect::<Vec<_>>(),
    })
}

fn outcome_line(label: &str, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Verified => format!("[ok] {label}"),
        Outcome::Failed(reason) => format!("[FAIL] {label}: {reason}"),
        Outcome::Unverifiable(reason) => format!("[??] {label}: {reason}"),
    }
}

/// The text report printed by `verify`.
pub fn report_text(verification: &Verification) -> String {
    let mut lines = vec![format!("conformance: {}", verification.conformance_level)];
    if let Some(outcome) = &verification.catalog_signature {
        let authorized = if verification.catalog_authorized {
            "authorized operator"
        } else {
            "operator not authorized"
        };
        lines.push(format!(
            "{} ({authorized})",
            outcome_line("catalog signature", outcome)
        ));
    }
    for entry in &verification.entries {
        let endorsed = if entry.publisher_endorsed {
            ", publisher-endorsed"
        } else {
            ""
        };
        lines.push(format!(
            "{} (catalog.entries[{}]){endorsed}",
            entry.identifier, entry.index
        ));
        if let Some(outcome) = &entry.artifact {
            lines.push(format!("  {}", outcome_line("artifact digest", outcome)));
        }
        if entry.manifests.is_empty() {
            lines.push("  no trust manifests".to_owned());
        }
        for manifest in &entry.manifests {
            let role = if manifest.publisher {
                ", publisher"
            } else {
                ""
            };
            lines.push(format!("  {}{role}", manifest.contributor));
            match &manifest.signature {
                Some(outcome) => lines.push(format!("    {}", outcome_line("signature", outcome))),
                None => lines.push("    unsigned".to_owned()),
            }
            if let Some(outcome) = &manifest.release {
                lines.push(format!("    {}", outcome_line("release binding", outcome)));
            }
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_root_did_web_dids_to_their_document_url() {
        assert_eq!(
            did_document_url("did:web:example.com").as_deref(),
            Ok("https://example.com/.well-known/did.json")
        );
        for did in [
            "did:web:example.com:users:alice",
            "did:web:example.com%3A8443",
            "did:web:Example.com",
            "did:web:127.0.0.1",
            "did:key:z6Mk",
        ] {
            assert!(did_document_url(did).is_err(), "{did} should be rejected");
        }
    }

    #[test]
    fn overrides_are_served_and_offline_fetches_fail() {
        let fetch = SafeFetch {
            overrides: BTreeMap::from([("https://example.com/a".to_owned(), b"body".to_vec())]),
            offline: true,
        };

        assert_eq!(fetch.get("https://example.com/a"), Ok(b"body".to_vec()));
        assert!(fetch.get("https://example.com/b").is_err());
    }

    #[test]
    fn refuses_unsafe_urls_before_connecting() {
        for url in [
            "http://example.com/did.json",
            "https://127.0.0.1/did.json",
            "https://[::1]/did.json",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(checked_target(url).is_err(), "{url} should be refused");
        }
        assert!(checked_target("https://example.com/.well-known/did.json").is_ok());
    }

    #[test]
    fn disallows_local_and_private_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(
                is_disallowed(ip.parse().expect("ip")),
                "{ip} should be disallowed"
            );
        }
        assert!(!is_disallowed("93.184.216.34".parse().expect("ip")));
    }
}
