// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use url::Url;

use crate::error::{Error, Result};

/// Largest catalog document the CLI will read.
const MAX_CATALOG_BYTES: u64 = 10 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;

pub fn build_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(format!("ai-catalog/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .redirect(Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("refusing to follow a redirect to a non-https URL")
            } else if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

pub fn is_file_url(url: &str) -> bool {
    url.starts_with("file://")
}

fn file_path(url: &str) -> Result<PathBuf> {
    Url::parse(url)
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .ok_or_else(|| Error::Other(format!("invalid file URL: {url}")))
}

/// Turn a bare local path into an absolute `file://` URL; anything with a scheme is returned as is.
pub fn local_path_to_file_url(path: &str) -> Result<String> {
    if path.contains("://") {
        return Ok(path.to_string());
    }
    let abs = std::fs::canonicalize(path)
        .map_err(|e| Error::Other(format!("cannot resolve path \"{path}\": {e}")))?;
    Url::from_file_path(&abs)
        .map(String::from)
        .map_err(|()| Error::Other(format!("cannot convert \"{path}\" to a URL")))
}

/// Resolve `reference` against the URL of the catalog that contains it.
pub fn join(base: &str, reference: &str) -> Result<String> {
    if Url::parse(reference).is_ok() {
        return Ok(reference.to_string());
    }
    Url::parse(base)
        .and_then(|b| b.join(reference))
        .map(String::from)
        .map_err(|e| {
            Error::Other(format!(
                "cannot resolve \"{reference}\" against \"{base}\": {e}"
            ))
        })
}

/// Fetch a catalog document, refusing anything over the size limit.
pub fn fetch_catalog(url: &str, client: &Client) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open(url, client)?
        .take(MAX_CATALOG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return Err(Error::Other(format!(
            "{url} is larger than {} MiB",
            MAX_CATALOG_BYTES / 1024 / 1024
        )));
    }
    Ok(bytes)
}

/// Fetch an artifact, which may legitimately be large.
pub fn fetch_artifact(url: &str, client: &Client) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open(url, client)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn open(url: &str, client: &Client) -> Result<Box<dyn Read>> {
    if is_file_url(url) {
        return Ok(Box::new(std::fs::File::open(file_path(url)?)?));
    }
    Ok(Box::new(client.get(url).send()?.error_for_status()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_resolves_relative_references() {
        assert_eq!(
            join("https://example.com/a/catalog.json", "b/c.json").unwrap(),
            "https://example.com/a/b/c.json"
        );
        assert_eq!(
            join("file:///tmp/a/catalog.json", "../b.json").unwrap(),
            "file:///tmp/b.json"
        );
        assert_eq!(
            join("https://example.com/a.json", "https://other.example/b.json").unwrap(),
            "https://other.example/b.json"
        );
    }

    #[test]
    fn fetch_catalog_rejects_oversized_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.json");
        std::fs::write(&path, vec![b' '; MAX_CATALOG_BYTES as usize + 1]).unwrap();
        let url = local_path_to_file_url(path.to_str().unwrap()).unwrap();
        assert!(fetch_catalog(&url, &build_client().unwrap()).is_err());
    }
}
