// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// SPDX-License-Identifier: Apache-2.0

//! Verification of signed trust manifests under the `did-web-v1` signer profile.
//!
//! For every trust manifest this checks the release binding to its entry, the
//! signature object, the detached ES256 JWS over the manifest's JCS payload, the
//! signer's root `did:web` document, and the signature's validity period. It also
//! reports whether the signer has publisher authority for a `urn:air` entry, and
//! checks inline `data` against the entry digest.

use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use ai_catalog::{AiCatalog, CatalogEntry, TrustManifest};
use ai_catalog_trust::{canonicalize, canonicalize_trust_manifest, verify_digest};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::Serialize;
use serde_json::Value;

/// The only signer profile this command implements.
pub const DID_WEB_PROFILE: &str = "did-web-v1";
/// Protected `typ` header required on trust manifest signatures.
pub const TRUST_MANIFEST_JWS_TYPE: &str = "ai-catalog-trust-manifest+jws";

const MAX_DID_DOCUMENT_BYTES: u64 = 256 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const FORBIDDEN_HEADERS: [&str; 5] = ["jku", "jwk", "x5u", "x5c", "b64"];

/// Supplies the DID document for a root `did:web` DID.
pub trait DidResolver {
    fn resolve(&self, did: &str) -> Result<Value, Inconclusive>;
}

/// Why a check could not be completed, as opposed to failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inconclusive(pub String);

/// Resolves DID documents from local overrides first, then over HTTPS unless offline.
pub struct DidWebResolver {
    pub documents: BTreeMap<String, Value>,
    pub offline: bool,
}

impl DidResolver for DidWebResolver {
    fn resolve(&self, did: &str) -> Result<Value, Inconclusive> {
        if let Some(document) = self.documents.get(did) {
            return Ok(document.clone());
        }
        if self.offline {
            return Err(Inconclusive(format!(
                "no DID document supplied for {did} and --offline is set"
            )));
        }
        let host = root_did_web_host(did).map_err(Inconclusive)?;
        fetch_did_document(host).map_err(Inconclusive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Every check passed.
    Verified,
    /// At least one check failed.
    Failed,
    /// No check failed, but at least one could not be completed.
    Inconclusive,
    /// The manifest carries no signature; only its release binding was checked.
    Unsigned,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub code: &'static str,
    /// `true` passed, `false` failed, `None` could not be completed.
    pub passed: Option<bool>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestReport {
    pub path: String,
    pub contributor: String,
    /// Whether the signer is the `urn:air` publisher of the entry, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher_authority: Option<bool>,
    pub outcome: Outcome,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryReport {
    pub path: String,
    pub identifier: String,
    /// Inline `data` checked against `digest`; `None` when not checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Check>,
    pub manifests: Vec<ManifestReport>,
}

/// Verifies every trust manifest of every root entry in `catalog`.
pub fn verify_catalog(
    catalog: &AiCatalog,
    resolver: &dyn DidResolver,
    now: DateTime<Utc>,
) -> Vec<EntryReport> {
    catalog
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let path = format!("catalog.entries[{index}]");
            let manifests = entry
                .trust_manifests
                .iter()
                .enumerate()
                .map(|(m, manifest)| {
                    verify_manifest(
                        entry,
                        manifest,
                        format!("{path}.trustManifests[{m}]"),
                        resolver,
                        now,
                    )
                })
                .collect();
            EntryReport {
                artifact: check_inline_artifact(entry),
                identifier: entry.identifier.clone(),
                path,
                manifests,
            }
        })
        .collect()
}

fn check_inline_artifact(entry: &CatalogEntry) -> Option<Check> {
    let (data, digest) = (entry.data.as_ref()?, entry.digest.as_deref()?);
    let check = match serde_json::to_vec(data)
        .map_err(|error| error.to_string())
        .and_then(|bytes| canonicalize(bytes).map_err(|error| error.to_string()))
        .and_then(|jcs| verify_digest(digest, jcs.as_bytes()).map_err(|error| error.to_string()))
    {
        Ok(true) => pass("artifact-digest", "inline data matches the entry digest"),
        Ok(false) => fail(
            "artifact-digest",
            "inline data does not match the entry digest",
        ),
        Err(error) => fail(
            "artifact-digest",
            format!("cannot check the entry digest: {error}"),
        ),
    };
    Some(check)
}

fn verify_manifest(
    entry: &CatalogEntry,
    manifest: &TrustManifest,
    path: String,
    resolver: &dyn DidResolver,
    now: DateTime<Utc>,
) -> ManifestReport {
    let mut checks = vec![check_release_binding(entry, manifest)];
    let mut publisher_authority = None;

    if let Some(signature) = &manifest.signature {
        checks.push(if signature.signer == manifest.contributor {
            pass(
                "signer-matches-contributor",
                "signature.signer equals contributor",
            )
        } else {
            fail(
                "signer-matches-contributor",
                format!(
                    "signature.signer {:?} does not equal contributor {:?}",
                    signature.signer, manifest.contributor
                ),
            )
        });
        checks.push(check_validity_period(
            &signature.issued_at,
            signature.expires_at.as_deref(),
            now,
        ));

        if signature.profile == DID_WEB_PROFILE {
            checks.extend(check_did_web_signature(manifest, resolver));
            publisher_authority = publisher_domain(&entry.identifier)
                .map(|domain| signature.signer == format!("did:web:{domain}"));
        } else {
            checks.push(unknown(
                "profile",
                format!("signer profile {:?} is not supported", signature.profile),
            ));
        }
    }

    let outcome = if manifest.signature.is_none() {
        if checks.iter().any(|check| check.passed == Some(false)) {
            Outcome::Failed
        } else {
            Outcome::Unsigned
        }
    } else if checks.iter().any(|check| check.passed == Some(false)) {
        Outcome::Failed
    } else if checks.iter().any(|check| check.passed.is_none()) {
        Outcome::Inconclusive
    } else {
        Outcome::Verified
    };

    ManifestReport {
        path,
        contributor: manifest.contributor.clone(),
        publisher_authority,
        outcome,
        checks,
    }
}

fn check_release_binding(entry: &CatalogEntry, manifest: &TrustManifest) -> Check {
    let Some(subject) = &manifest.subject else {
        return if manifest.signature.is_some() {
            fail("release-binding", "a signed manifest must carry a subject")
        } else {
            unknown("release-binding", "unsigned manifest without subject")
        };
    };
    let mut mismatches = Vec::new();
    if subject.identifier != entry.identifier {
        mismatches.push("identifier");
    }
    if subject.r#type != entry.entry_type {
        mismatches.push("type");
    }
    if Some(subject.digest.as_str()) != entry.digest.as_deref() {
        mismatches.push("digest");
    }
    if subject.version != entry.version {
        mismatches.push("version");
    }
    if mismatches.is_empty() {
        pass("release-binding", "subject matches the entry")
    } else {
        fail(
            "release-binding",
            format!(
                "subject does not match the entry: {}",
                mismatches.join(", ")
            ),
        )
    }
}

fn check_validity_period(issued_at: &str, expires_at: Option<&str>, now: DateTime<Utc>) -> Check {
    let parse = |value: &str| DateTime::parse_from_rfc3339(value).map(|t| t.with_timezone(&Utc));
    let Ok(issued) = parse(issued_at) else {
        return fail(
            "validity-period",
            format!("issuedAt {issued_at:?} is not RFC 3339"),
        );
    };
    if now < issued {
        return fail("validity-period", format!("not valid before {issued_at}"));
    }
    match expires_at.map(parse) {
        None => pass(
            "validity-period",
            format!("issued at {issued_at}, no expiry"),
        ),
        Some(Err(_)) => fail("validity-period", "expiresAt is not RFC 3339"),
        Some(Ok(expires)) if expires <= issued => {
            fail("validity-period", "expiresAt is not later than issuedAt")
        }
        Some(Ok(expires)) if now >= expires => fail(
            "validity-period",
            format!("expired at {}", expires_at.unwrap_or_default()),
        ),
        Some(Ok(_)) => pass(
            "validity-period",
            format!("valid until {}", expires_at.unwrap_or_default()),
        ),
    }
}

/// Header, key resolution, and signature checks of the `did:web` signer profile.
fn check_did_web_signature(manifest: &TrustManifest, resolver: &dyn DidResolver) -> Vec<Check> {
    let signature = manifest
        .signature
        .as_ref()
        .expect("caller checks signature");
    let signer = signature.signer.as_str();
    let mut checks = Vec::new();

    if let Err(reason) = root_did_web_host(signer) {
        checks.push(fail("signer-did", reason));
        return checks;
    }

    let header = match parse_detached_jws(&signature.jws) {
        Ok(parts) => parts,
        Err(reason) => {
            checks.push(fail("jws-format", reason));
            return checks;
        }
    };
    let kid = match check_header(&header.protected, signer) {
        Ok(kid) => {
            checks.push(pass("jws-header", "alg ES256, typ and kid as required"));
            kid
        }
        Err(reason) => {
            checks.push(fail("jws-header", reason));
            return checks;
        }
    };

    let document = match resolver.resolve(signer) {
        Ok(document) => document,
        Err(Inconclusive(reason)) => {
            checks.push(unknown("did-document", reason));
            return checks;
        }
    };
    let key = match select_assertion_key(&document, signer, &kid) {
        Ok(key) => {
            checks.push(pass(
                "did-key",
                format!("{kid} is an assertionMethod P-256 key"),
            ));
            key
        }
        Err(reason) => {
            checks.push(fail("did-key", reason));
            return checks;
        }
    };

    let payload = match canonicalize_trust_manifest(manifest) {
        Ok(payload) => payload,
        Err(error) => {
            checks.push(fail(
                "jws-signature",
                format!("cannot canonicalize: {error}"),
            ));
            return checks;
        }
    };
    let signing_input = format!(
        "{}.{}",
        header.encoded_protected,
        URL_SAFE_NO_PAD.encode(payload.as_bytes())
    );
    checks.push(
        match Signature::from_slice(&header.signature)
            .map_err(|_| "signature is not a 64-byte ES256 value".to_owned())
            .and_then(|sig| {
                key.verify(signing_input.as_bytes(), &sig)
                    .map_err(|_| "signature does not verify over the manifest payload".to_owned())
            }) {
            Ok(()) => pass("jws-signature", "ES256 signature verifies"),
            Err(reason) => fail("jws-signature", reason),
        },
    );
    checks
}

struct DetachedJws {
    encoded_protected: String,
    protected: Value,
    signature: Vec<u8>,
}

fn parse_detached_jws(jws: &str) -> Result<DetachedJws, String> {
    let parts: Vec<&str> = jws.split('.').collect();
    let [protected, payload, signature] = parts.as_slice() else {
        return Err("jws must have three dot-separated parts".to_owned());
    };
    if !payload.is_empty() {
        return Err("jws must be detached (empty payload segment)".to_owned());
    }
    let header_bytes = URL_SAFE_NO_PAD
        .decode(protected)
        .map_err(|_| "protected header is not base64url".to_owned())?;
    let header: Value = serde_json::from_slice(&header_bytes)
        .map_err(|_| "protected header is not JSON".to_owned())?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| "signature is not base64url".to_owned())?;
    Ok(DetachedJws {
        encoded_protected: (*protected).to_owned(),
        protected: header,
        signature,
    })
}

/// Returns the `kid` after checking the protected header against the profile.
fn check_header(header: &Value, signer: &str) -> Result<String, String> {
    let object = header
        .as_object()
        .ok_or("protected header must be an object")?;
    if object.get("alg").and_then(Value::as_str) != Some("ES256") {
        return Err("protected header alg must be ES256".to_owned());
    }
    if object.get("typ").and_then(Value::as_str) != Some(TRUST_MANIFEST_JWS_TYPE) {
        return Err(format!(
            "protected header typ must be {TRUST_MANIFEST_JWS_TYPE}"
        ));
    }
    if let Some(name) = FORBIDDEN_HEADERS
        .iter()
        .find(|name| object.contains_key(**name))
    {
        return Err(format!("protected header must not contain {name}"));
    }
    if object.contains_key("crit") {
        return Err("protected header crit parameters are not supported".to_owned());
    }
    let kid = object
        .get("kid")
        .and_then(Value::as_str)
        .ok_or("protected header must contain kid")?;
    match kid
        .strip_prefix(signer)
        .and_then(|rest| rest.strip_prefix('#'))
    {
        Some(fragment) if !fragment.is_empty() && !fragment.contains(['/', '?']) => {
            Ok(kid.to_owned())
        }
        _ => Err(format!("kid {kid:?} must be {signer}#<fragment>")),
    }
}

fn select_assertion_key(document: &Value, signer: &str, kid: &str) -> Result<VerifyingKey, String> {
    if document.get("id").and_then(Value::as_str) != Some(signer) {
        return Err(format!("DID document id is not {signer}"));
    }
    let absolute = |id: &str| {
        if id.starts_with('#') {
            format!("{signer}{id}")
        } else {
            id.to_owned()
        }
    };
    let methods: Vec<&Value> = document
        .get("verificationMethod")
        .and_then(Value::as_array)
        .map(|methods| methods.iter().collect())
        .unwrap_or_default();
    let mut selected = Vec::new();
    for item in document
        .get("assertionMethod")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let method = match item {
            Value::String(reference) if absolute(reference) == kid => {
                methods.iter().copied().find(|method| {
                    method
                        .get("id")
                        .and_then(Value::as_str)
                        .map(absolute)
                        .as_deref()
                        == Some(kid)
                })
            }
            Value::Object(_)
                if item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(absolute)
                    .as_deref()
                    == Some(kid) =>
            {
                Some(item)
            }
            _ => None,
        };
        selected.extend(method);
    }
    let [method] = selected.as_slice() else {
        return Err(format!(
            "expected exactly one assertionMethod key {kid}, found {}",
            selected.len()
        ));
    };
    if method.get("controller").and_then(Value::as_str) != Some(signer) {
        return Err(format!("verification method controller is not {signer}"));
    }
    let jwk = method
        .get("publicKeyJwk")
        .and_then(Value::as_object)
        .ok_or("verification method must contain publicKeyJwk")?;
    let field = |name: &str| jwk.get(name).and_then(Value::as_str);
    if field("kty") != Some("EC") || field("crv") != Some("P-256") {
        return Err("publicKeyJwk must be an EC P-256 key".to_owned());
    }
    if jwk.contains_key("d") {
        return Err("publicKeyJwk must not contain private key material".to_owned());
    }
    if field("alg").is_some_and(|alg| alg != "ES256") || field("use").is_some_and(|u| u != "sig") {
        return Err("publicKeyJwk alg/use do not allow ES256 signatures".to_owned());
    }
    if let Some(ops) = jwk.get("key_ops") {
        let allows_verify = ops
            .as_array()
            .is_some_and(|ops| ops.iter().any(|op| op.as_str() == Some("verify")));
        if !allows_verify {
            return Err("publicKeyJwk key_ops does not permit verify".to_owned());
        }
    }
    let coordinate = |name: &str| {
        field(name)
            .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
            .filter(|bytes| bytes.len() == 32)
            .ok_or_else(|| format!("publicKeyJwk {name} must be a 32-byte base64url value"))
    };
    let mut sec1 = vec![0x04];
    sec1.extend(coordinate("x")?);
    sec1.extend(coordinate("y")?);
    VerifyingKey::from_sec1_bytes(&sec1)
        .map_err(|_| "publicKeyJwk is not a valid P-256 point".to_owned())
}

/// The publisher domain of a standard `urn:air` identifier.
fn publisher_domain(identifier: &str) -> Option<&str> {
    let rest = identifier.strip_prefix("urn:air:")?;
    rest.split(':').next().filter(|domain| !domain.is_empty())
}

/// Returns the host of a root `did:web` DID that satisfies the profile's restrictions.
fn root_did_web_host(did: &str) -> Result<&str, String> {
    let host = did
        .strip_prefix("did:web:")
        .ok_or_else(|| format!("signer {did:?} is not a did:web DID"))?;
    let valid = !host.is_empty()
        && !host.contains([':', '%', '/'])
        && !host.ends_with('.')
        && host.parse::<IpAddr>().is_err()
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
    if valid {
        Ok(host)
    } else {
        Err(format!(
            "signer {did:?} must be a root did:web DID with a lowercase ASCII domain, no port or path"
        ))
    }
}

/// Fetches `https://<host>/.well-known/did.json` under the spec's safe-fetching rules:
/// HTTPS only, no redirects, a size and time limit, and no private or local addresses.
/// The resolved addresses are pinned so the connection uses the addresses checked.
fn fetch_did_document(host: &str) -> Result<Value, String> {
    let addresses: Vec<SocketAddr> = (host, 443)
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
        .get(format!("https://{host}/.well-known/did.json"))
        .send()
        .map_err(|error| format!("cannot fetch DID document for {host}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "DID document for {host} returned {}",
            response.status()
        ));
    }
    let mut body = Vec::new();
    response
        .take(MAX_DID_DOCUMENT_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() as u64 > MAX_DID_DOCUMENT_BYTES {
        return Err(format!(
            "DID document for {host} exceeds {MAX_DID_DOCUMENT_BYTES} bytes"
        ));
    }
    ai_catalog_trust::parse_json(&body)
        .map_err(|error| format!("DID document is not JSON: {error}"))
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

fn pass(code: &'static str, message: impl Into<String>) -> Check {
    Check {
        code,
        passed: Some(true),
        message: message.into(),
    }
}

fn fail(code: &'static str, message: impl Into<String>) -> Check {
    Check {
        code,
        passed: Some(false),
        message: message.into(),
    }
}

fn unknown(code: &'static str, message: impl Into<String>) -> Check {
    Check {
        code,
        passed: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;
    use serde_json::json;

    const SIGNER: &str = "did:web:apexretail.com";
    const KID: &str = "did:web:apexretail.com#release-key";
    const ARTIFACT_DIGEST: &str =
        "sha256:f30fccb91499a1a7dfaaab42fdb41c7874ff9e2d21ee279088096ed71289b06c";

    fn key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).expect("valid scalar")
    }

    fn did_document(key: &SigningKey, relationship: &str) -> Value {
        let point = key.verifying_key().to_sec1_point(false);
        let (x, y) = point.as_bytes()[1..].split_at(32);
        json!({
            "id": SIGNER,
            "verificationMethod": [{
                "id": KID,
                "type": "JsonWebKey2020",
                "controller": SIGNER,
                "publicKeyJwk": {
                    "kty": "EC",
                    "crv": "P-256",
                    "x": URL_SAFE_NO_PAD.encode(x),
                    "y": URL_SAFE_NO_PAD.encode(y),
                }
            }],
            relationship: [KID],
        })
    }

    fn entry_json(signature: Value) -> Value {
        json!({
            "identifier": "urn:air:apexretail.com:claims:refund-bot",
            "version": "1.2.0",
            "type": "application/a2a-agent-card+json",
            "url": "https://agents.apexretail.com/agent.json",
            "digest": ARTIFACT_DIGEST,
            "trustManifests": [{
                "contributor": SIGNER,
                "subject": {
                    "identifier": "urn:air:apexretail.com:claims:refund-bot",
                    "version": "1.2.0",
                    "type": "application/a2a-agent-card+json",
                    "digest": ARTIFACT_DIGEST,
                },
                "attestations": [{ "type": "SOC2-Type2", "uri": "https://example.com/soc2.pdf" }],
                "signature": signature,
            }]
        })
    }

    /// Builds a catalog whose single manifest is signed with `key` and `header`.
    fn signed_catalog(key: &SigningKey, header: Value, edit: impl FnOnce(&mut Value)) -> AiCatalog {
        let signature = json!({
            "signer": SIGNER,
            "profile": DID_WEB_PROFILE,
            "issuedAt": "2026-09-10T18:30:00Z",
            "expiresAt": "2027-09-10T18:30:00Z",
            "jws": "",
        });
        let mut document = json!({ "specVersion": "1.0", "entries": [entry_json(signature)] });
        let unsigned = ai_catalog::parse_str(&document.to_string()).expect("parses");
        let payload = canonicalize_trust_manifest(&unsigned.entries[0].trust_manifests[0])
            .expect("canonicalizes");
        let encoded_header = URL_SAFE_NO_PAD.encode(header.to_string());
        let signing_input = format!("{encoded_header}.{}", URL_SAFE_NO_PAD.encode(payload));
        let signature: Signature = key.sign(signing_input.as_bytes());
        document["entries"][0]["trustManifests"][0]["signature"]["jws"] = json!(format!(
            "{encoded_header}..{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ));
        edit(&mut document);
        ai_catalog::parse_str(&document.to_string()).expect("parses")
    }

    fn header() -> Value {
        json!({ "alg": "ES256", "kid": KID, "typ": TRUST_MANIFEST_JWS_TYPE })
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-05T12:00:00Z")
            .expect("time")
            .with_timezone(&Utc)
    }

    fn resolver(document: Value) -> DidWebResolver {
        DidWebResolver {
            documents: BTreeMap::from([(SIGNER.to_owned(), document)]),
            offline: true,
        }
    }

    fn report(catalog: &AiCatalog, resolver: &DidWebResolver) -> ManifestReport {
        verify_catalog(catalog, resolver, now())[0].manifests[0].clone()
    }

    fn failed_codes(report: &ManifestReport) -> Vec<&'static str> {
        report
            .checks
            .iter()
            .filter(|check| check.passed == Some(false))
            .map(|check| check.code)
            .collect()
    }

    #[test]
    fn verifies_a_valid_publisher_signature() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |_| {});
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert_eq!(report.outcome, Outcome::Verified, "{:?}", report.checks);
        assert_eq!(report.publisher_authority, Some(true));
    }

    #[test]
    fn rejects_a_tampered_claim() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |doc| {
            doc["entries"][0]["trustManifests"][0]["attestations"][0]["type"] = json!("ISO27001");
        });
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(failed_codes(&report), ["jws-signature"]);
    }

    #[test]
    fn rejects_a_relabelled_release() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |doc| {
            doc["entries"][0]["version"] = json!("2.0.0");
        });
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert_eq!(failed_codes(&report), ["release-binding"]);
    }

    #[test]
    fn rejects_a_key_outside_assertion_method() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |_| {});
        let report = report(&catalog, &resolver(did_document(&key, "authentication")));

        assert_eq!(failed_codes(&report), ["did-key"]);
    }

    #[test]
    fn rejects_a_kid_for_another_did() {
        let key = key();
        let mut foreign = header();
        foreign["kid"] = json!("did:web:attacker.example#key");
        let catalog = signed_catalog(&key, foreign, |_| {});
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert_eq!(failed_codes(&report), ["jws-header"]);
    }

    #[test]
    fn rejects_a_header_without_typ() {
        let key = key();
        let catalog = signed_catalog(&key, json!({ "alg": "ES256", "kid": KID }), |_| {});
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert_eq!(failed_codes(&report), ["jws-header"]);
    }

    #[test]
    fn rejects_an_expired_signature() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |_| {});
        let resolver = resolver(did_document(&key, "assertionMethod"));
        let later = DateTime::parse_from_rfc3339("2028-01-01T00:00:00Z")
            .expect("time")
            .with_timezone(&Utc);
        let report = &verify_catalog(&catalog, &resolver, later)[0].manifests[0];

        assert_eq!(failed_codes(report), ["validity-period"]);
    }

    #[test]
    fn rejects_a_signer_that_is_not_the_contributor() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |doc| {
            doc["entries"][0]["trustManifests"][0]["contributor"] = json!("did:web:other.example");
        });
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert!(failed_codes(&report).contains(&"signer-matches-contributor"));
        assert_eq!(report.outcome, Outcome::Failed);
    }

    #[test]
    fn is_inconclusive_offline_without_a_document() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |_| {});
        let offline = DidWebResolver {
            documents: BTreeMap::new(),
            offline: true,
        };
        let report = report(&catalog, &offline);

        assert_eq!(report.outcome, Outcome::Inconclusive);
    }

    #[test]
    fn reports_assessor_signatures_without_publisher_authority() {
        let key = key();
        let catalog = signed_catalog(&key, header(), |doc| {
            doc["entries"][0]["identifier"] = json!("urn:air:bank.example:agent:x");
            doc["entries"][0]["trustManifests"][0]["subject"]["identifier"] =
                json!("urn:air:bank.example:agent:x");
        });
        let report = report(&catalog, &resolver(did_document(&key, "assertionMethod")));

        assert_eq!(report.publisher_authority, Some(false));
    }

    #[test]
    fn rejects_non_root_or_unsafe_did_web_signers() {
        for did in [
            "did:web:example.com:users:alice",
            "did:web:example.com%3A8443",
            "did:web:Example.com",
            "did:web:127.0.0.1",
            "did:key:z6Mk",
        ] {
            assert!(root_did_web_host(did).is_err(), "{did} should be rejected");
        }
        assert_eq!(root_did_web_host("did:web:example.com"), Ok("example.com"));
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
