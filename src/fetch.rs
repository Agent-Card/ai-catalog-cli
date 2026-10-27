// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ai_catalog_discover::{Fetcher, HttpFetcher, Method, discover_with};
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

/// Where the catalog that a user pointed the CLI at actually is.
#[derive(Debug)]
pub enum Located {
    /// A catalog URL, to be fetched.
    Url(String),
    /// A catalog that discovery has fetched already.
    Discovered {
        url: String,
        bytes: Vec<u8>,
        method: Method,
    },
}

/// Turn what the user typed (a local path, a `file://` or `http://` URL, an `https://`
/// URL or a bare domain) into a catalog.
///
/// Local files and explicit `http://` URLs are used as they are. An `https://` URL or a
/// bare domain goes through discovery (<https://ai-catalog.io/spec/#discovery>), which
/// accepts a URL that is a catalog itself and otherwise looks for the catalog the site
/// advertises through a `Link` header, an HTML `<link>` element or the well-known URI.
pub fn locate(input: &str, fetcher: &dyn Fetcher) -> Result<Located> {
    let input = input.trim();
    if input.starts_with("file://") || input.starts_with("http://") || Path::new(input).exists() {
        return Ok(Located::Url(local_path_to_file_url(input)?));
    }
    let bare = !input.contains("://");
    if bare && !looks_like_host(input) {
        return Ok(Located::Url(local_path_to_file_url(input)?));
    }
    match discover_with(fetcher, input) {
        Ok(found) => Ok(Located::Discovered {
            url: found.url,
            bytes: found.bytes,
            method: found.method,
        }),
        Err(e) if bare => Err(Error::Other(format!(
            "{e}\n  (there is no local file \"{input}\" either)"
        ))),
        Err(e) => Err(Error::Other(e.to_string())),
    }
}

/// A bare argument is a domain, not a mistyped path, when it starts like a host name
/// and its first segment has a dot or a port.
fn looks_like_host(input: &str) -> bool {
    let host = input.split('/').next().unwrap_or("");
    host.starts_with(|c: char| c.is_ascii_alphanumeric())
        && (host.contains('.') || host.contains(':') || host == "localhost")
}

/// The fetcher discovery uses, with the CLI's own client.
pub fn build_fetcher() -> Result<HttpFetcher> {
    Ok(HttpFetcher::with_client(build_client()?))
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

    use std::cell::RefCell;

    use ai_catalog_discover::Response;

    const CATALOG: &str = r#"{"specVersion": "1.0", "entries": []}"#;

    /// Serves canned responses, and records the requests.
    #[derive(Default)]
    struct Mock {
        pages: Vec<(&'static str, &'static str, Vec<&'static str>, &'static str)>,
        requests: RefCell<Vec<String>>,
    }

    impl Mock {
        /// A page at `url`, with a content type, `Link` headers and a body.
        fn page(
            mut self,
            url: &'static str,
            content_type: &'static str,
            links: &[&'static str],
            body: &'static str,
        ) -> Self {
            self.pages.push((url, content_type, links.to_vec(), body));
            self
        }
    }

    impl Fetcher for Mock {
        fn get(&self, url: &Url) -> std::result::Result<Response, String> {
            self.requests.borrow_mut().push(url.to_string());
            let page = self.pages.iter().find(|p| p.0 == url.as_str());
            Ok(Response {
                url: url.clone(),
                status: if page.is_some() { 200 } else { 404 },
                content_type: page.map(|p| p.1.to_string()),
                link_headers: page
                    .map(|p| p.2.iter().map(|l| l.to_string()).collect())
                    .unwrap_or_default(),
                body: page.map(|p| p.3.as_bytes().to_vec()).unwrap_or_default(),
            })
        }
    }

    #[test]
    fn locate_leaves_files_and_http_urls_alone() {
        let mock = Mock::default();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("catalog.json");
        std::fs::write(&path, CATALOG).unwrap();

        for input in [
            path.to_str().unwrap(),
            "file:///tmp/catalog.json",
            "http://localhost:8080/catalog.json",
        ] {
            let Located::Url(url) = locate(input, &mock).unwrap() else {
                panic!("{input} should not be discovered");
            };
            assert!(url.contains("://"), "{url}");
        }
        assert!(mock.requests.borrow().is_empty());
    }

    #[test]
    fn locate_discovers_the_catalog_of_a_bare_domain() {
        let mock = Mock::default()
            .page(
                "https://example.com/",
                "text/html",
                &[],
                r#"<head><link rel="ai-catalog" href="/ai.json"></head>"#,
            )
            .page(
                "https://example.com/ai.json",
                "application/ai-catalog+json",
                &[],
                CATALOG,
            );

        let Located::Discovered { url, bytes, method } = locate("example.com", &mock).unwrap()
        else {
            panic!("expected discovery");
        };

        assert_eq!(url, "https://example.com/ai.json");
        assert_eq!(bytes, CATALOG.as_bytes());
        assert_eq!(method, Method::HtmlLink);
    }

    #[test]
    fn locate_does_not_fetch_a_catalog_url_twice() {
        let mock = Mock::default().page(
            "https://example.com/ai.json",
            "application/json",
            &[],
            CATALOG,
        );

        let Located::Discovered { method, .. } =
            locate("https://example.com/ai.json", &mock).unwrap()
        else {
            panic!("expected discovery");
        };

        assert_eq!(method, Method::Direct);
        assert_eq!(mock.requests.borrow().len(), 1);
    }

    #[test]
    fn locate_explains_a_failed_discovery() {
        let mock = Mock::default().page("https://example.com/", "text/html", &[], "<html>");

        let error = locate("example.com", &mock).unwrap_err().to_string();

        assert!(error.contains("no AI Catalog found"), "{error}");
        assert!(error.contains("/.well-known/ai-catalog.json"), "{error}");
        assert!(error.contains("no local file"), "{error}");
        let error = locate("https://example.com", &mock)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("no local file"), "{error}");
    }

    #[test]
    fn locate_keeps_the_error_for_a_mistyped_path() {
        let mock = Mock::default();

        for input in ["catalogs/missing.json", "./missing.json", "~/missing.json"] {
            let error = locate(input, &mock).unwrap_err().to_string();
            assert!(error.contains("cannot resolve path"), "{error}");
        }
        assert!(mock.requests.borrow().is_empty());
    }

    #[test]
    fn only_names_that_look_like_hosts_are_discovered() {
        for host in ["example.com", "example.com/docs", "localhost:8443", "a.b"] {
            assert!(looks_like_host(host), "{host}");
        }
        for path in [
            "catalog",
            "catalogs/a.json",
            "./a.json",
            "../a.json",
            "~/a.json",
            "/a.json",
        ] {
            assert!(!looks_like_host(path), "{path}");
        }
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
