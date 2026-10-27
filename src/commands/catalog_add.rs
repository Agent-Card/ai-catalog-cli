// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use colored::Colorize;

use ai_catalog::CatalogEntry;
use ai_catalog_discover::Fetcher;

use crate::cache::CacheManager;
use crate::error::{Error, Result};
use crate::fetch::{Located, build_fetcher, locate};
use crate::resolver::{Resolver, cli_extension, make_entry_extensions, registry_identifier};

pub fn execute(name: &str, input: &str) -> Result<()> {
    let cache = CacheManager::new()?;
    add(&cache, &build_fetcher()?, name, input)
}

fn add(cache: &CacheManager, fetcher: &dyn Fetcher, name: &str, input: &str) -> Result<()> {
    cache.ensure_dirs()?;
    ensure_name_is_free(cache, name)?;
    println!("Fetching catalog from {}...", input.trim().cyan());
    let mut resolver = Resolver::online(cache)?;
    let (url, root_hash) = match locate(input, fetcher)? {
        Located::Url(url) => {
            ensure_url_is_free(cache, &url)?;
            let hash = resolver.fetch(&url)?;
            (url, hash)
        }
        Located::Discovered { url, bytes, method } => {
            if method != ai_catalog_discover::Method::Direct {
                println!("Found the catalog at {} ({method})", url.cyan());
            }
            ensure_url_is_free(cache, &url)?;
            let hash = resolver.fetch_bytes(&url, &bytes)?;
            (url, hash)
        }
    };
    let entry_count = resolver.leaves().count();
    resolver.into_found();
    let file_url = cache.object_file_url(&root_hash);
    let identifier = registry_identifier("local", name);
    let extensions = make_entry_extensions(&url, &root_hash, entry_count);
    let new_entry = CatalogEntry {
        identifier,
        entry_type: "application/ai-catalog+json".to_string(),
        display_name: Some(name.to_string()),
        description: None,
        tags: Vec::new(),
        url: Some(file_url),
        data: None,
        version: None,
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extensions: Some(extensions),
        publisher: None,
        trust_manifest: None,
        extra_fields: Default::default(),
    };
    let mut registry = cache.read_registry()?;
    registry.entries.push(new_entry);
    cache.write_registry(&registry)?;
    println!(
        "{} \"{}\" added ({} entries)",
        "✓".green(),
        name.bold(),
        entry_count
    );
    Ok(())
}

fn ensure_name_is_free(cache: &CacheManager, name: &str) -> Result<()> {
    let registry = cache.read_registry()?;
    let taken = registry.entries.iter().any(|entry| {
        entry
            .display_name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    });
    if taken {
        return Err(Error::Other(format!(
            "a catalog named \"{name}\" is already registered. Use `ai-catalog catalog update {name}` to refresh it."
        )));
    }
    Ok(())
}

fn ensure_url_is_free(cache: &CacheManager, url: &str) -> Result<()> {
    let registry = cache.read_registry()?;
    for entry in &registry.entries {
        if let Some(source_url) = cli_extension(entry)
            .and_then(|m| m.get("sourceUrl"))
            .and_then(|v| v.as_str())
            && source_url.eq_ignore_ascii_case(url)
        {
            let name = entry.display_name.as_deref().unwrap_or("?");
            return Err(Error::Other(format!(
                "URL is already registered as \"{name}\". Use `ai-catalog catalog update {name}` to refresh it."
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use ai_catalog_discover::Response;
    use url::Url;

    use super::*;

    const CATALOG: &str = r#"{
        "specVersion": "1.0",
        "entries": [{"identifier": "a", "type": "application/json", "url": "https://example.com/a.json"}]
    }"#;

    /// A site with a home page that advertises its catalog in a `Link` header.
    struct Site {
        requests: Cell<usize>,
    }

    impl Fetcher for Site {
        fn get(&self, url: &Url) -> std::result::Result<Response, String> {
            self.requests.set(self.requests.get() + 1);
            let (status, content_type, links, body) = match url.as_str() {
                "https://example.com/" => (
                    200,
                    "text/html",
                    vec![r#"</catalog/ai.json>; rel="ai-catalog""#.to_string()],
                    "<html></html>",
                ),
                "https://example.com/catalog/ai.json" => {
                    (200, "application/ai-catalog+json", vec![], CATALOG)
                }
                _ => (404, "text/plain", vec![], ""),
            };
            Ok(Response {
                url: url.clone(),
                status,
                content_type: Some(content_type.to_string()),
                link_headers: links,
                body: body.as_bytes().to_vec(),
            })
        }
    }

    fn cache(dir: &tempfile::TempDir) -> CacheManager {
        CacheManager::with_base_dir(dir.path().join("cache"))
    }

    fn registered(cache: &CacheManager) -> Vec<(String, String)> {
        cache
            .read_registry()
            .unwrap()
            .entries
            .iter()
            .map(|e| {
                let source = cli_extension(e).unwrap()["sourceUrl"].as_str().unwrap();
                (e.display_name.clone().unwrap(), source.to_string())
            })
            .collect()
    }

    #[test]
    fn a_bare_domain_registers_the_catalog_it_advertises() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(&dir);
        let site = Site {
            requests: Cell::new(0),
        };

        add(&cache, &site, "example", "example.com").unwrap();

        assert_eq!(
            registered(&cache),
            [(
                "example".to_string(),
                "https://example.com/catalog/ai.json".to_string()
            )]
        );
    }

    #[test]
    fn the_catalog_url_is_what_gets_updated_later() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(&dir);
        let site = Site {
            requests: Cell::new(0),
        };

        add(&cache, &site, "example", "https://example.com").unwrap();

        let entry = &cache.read_registry().unwrap().entries[0];
        let ext = cli_extension(entry).unwrap();
        assert_eq!(ext["sourceUrl"], "https://example.com/catalog/ai.json");
        assert_eq!(ext["entryCount"], 1);
    }

    #[test]
    fn a_catalog_url_is_fetched_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(&dir);
        let site = Site {
            requests: Cell::new(0),
        };

        add(
            &cache,
            &site,
            "example",
            "https://example.com/catalog/ai.json",
        )
        .unwrap();

        assert_eq!(site.requests.get(), 1);
    }

    #[test]
    fn the_same_catalog_cannot_be_registered_under_two_names() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(&dir);
        let site = Site {
            requests: Cell::new(0),
        };
        add(&cache, &site, "first", "example.com").unwrap();

        let error = add(
            &cache,
            &site,
            "second",
            "https://example.com/catalog/ai.json",
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("already registered as \"first\""), "{error}");
        assert_eq!(registered(&cache).len(), 1);
    }

    #[test]
    fn a_name_cannot_be_used_twice() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(&dir);
        let site = Site {
            requests: Cell::new(0),
        };
        add(&cache, &site, "example", "example.com").unwrap();
        let before = site.requests.get();

        let error = add(&cache, &site, "EXAMPLE", "example.com")
            .unwrap_err()
            .to_string();

        assert!(error.contains("already registered"), "{error}");
        assert_eq!(site.requests.get(), before, "nothing should be fetched");
    }

    #[test]
    fn a_site_without_a_catalog_is_not_registered() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(&dir);
        let site = Site {
            requests: Cell::new(0),
        };

        let error = add(&cache, &site, "nothing", "https://example.com/missing")
            .unwrap_err()
            .to_string();

        assert!(error.contains("no AI Catalog found"), "{error}");
        assert!(registered(&cache).is_empty());
    }
}
