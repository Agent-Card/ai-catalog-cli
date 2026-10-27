// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashMap};

use reqwest::blocking::Client;
use serde_json::json;

use ai_catalog::{AiCatalog, CatalogEntry};

use crate::cache::{CacheManager, content_hash};
use crate::error::{Error, Result};
use crate::fetch::{build_client, fetch_catalog, is_file_url, join};

/// Deepest nesting of catalogs within catalogs that is followed (spec: 4 recommended).
const MAX_DEPTH: usize = 4;

/// `ResolvedEntry::source_catalog_url` of the entries of the local registry itself.
pub const REGISTRY: &str = "registry";

/// A catalog entry and the URL of the catalog document it was listed in.
#[derive(Debug, Clone)]
pub struct ResolvedEntry {
    pub entry: CatalogEntry,
    pub source_catalog_url: String,
}

/// Walks a catalog and the catalogs nested in it.
///
/// An online resolver fetches nested catalogs and stores them in the cache; an
/// offline one reads only what the cache already holds. Problems with a nested
/// catalog are collected as warnings and never abort the walk.
pub struct Resolver<'a> {
    cache: &'a CacheManager,
    refs: HashMap<String, String>,
    client: Option<Client>,
    /// Catalog URLs on the current traversal path, to reject cycles.
    path: Vec<String>,
    found: Vec<ResolvedEntry>,
    warnings: Vec<String>,
}

impl<'a> Resolver<'a> {
    pub fn offline(cache: &'a CacheManager) -> Result<Self> {
        Ok(Resolver {
            cache,
            refs: cache.read_refs()?,
            client: None,
            path: Vec::new(),
            found: Vec::new(),
            warnings: Vec::new(),
        })
    }

    pub fn online(cache: &'a CacheManager) -> Result<Self> {
        cache.ensure_dirs()?;
        Ok(Resolver {
            client: Some(build_client()?),
            ..Self::offline(cache)?
        })
    }

    /// Fetch the catalog at `url` and everything nested in it, caching each
    /// document. Returns the content hash of the root document.
    pub fn fetch(&mut self, url: &str) -> Result<String> {
        let client = self
            .client
            .as_ref()
            .expect("fetch needs an online resolver");
        let bytes = fetch_catalog(url, client)?;
        self.fetch_bytes(url, &bytes)
    }

    /// Like [`Resolver::fetch`], for a root document the caller has fetched from `url`.
    pub fn fetch_bytes(&mut self, url: &str, bytes: &[u8]) -> Result<String> {
        let catalog = serde_json::from_slice(bytes)?;
        let hash = self.store(url, bytes)?;
        self.path.push(url.to_string());
        self.walk(&catalog, url, 0);
        self.path.pop();
        self.cache.write_refs(&self.refs)?;
        Ok(hash)
    }

    /// Walk the registered catalogs, optionally only those whose registry
    /// identifier starts with `prefix`.
    pub fn walk_registry(&mut self, prefix: Option<&str>) -> Result<()> {
        let mut registry = self.cache.read_registry()?;
        if let Some(prefix) = prefix {
            registry
                .entries
                .retain(|e| e.identifier.starts_with(prefix));
        }
        self.walk(&registry, REGISTRY, 0);
        Ok(())
    }

    /// Walk the catalog that `entry` (listed in `base`) points to or embeds.
    pub fn descend(&mut self, entry: &CatalogEntry, base: &str) -> Result<()> {
        if let Some((bytes, child_base)) = self.open(entry, base)? {
            let catalog = serde_json::from_slice(&bytes)?;
            self.path.extend([base.to_string(), child_base.clone()]);
            self.walk(&catalog, &child_base, 0);
            self.path.truncate(self.path.len() - 2);
        }
        Ok(())
    }

    /// The document of a catalog entry, as stored.
    pub fn catalog_bytes(&mut self, entry: &CatalogEntry, base: &str) -> Result<Vec<u8>> {
        self.open(entry, base)?
            .map(|(bytes, _)| bytes)
            .ok_or_else(|| Error::Other(format!("entry \"{}\" has no URL", entry.identifier)))
    }

    /// The first entry with this identifier, catalogs included.
    pub fn find(&self, identifier: &str) -> Option<&ResolvedEntry> {
        self.found.iter().find(|f| f.entry.identifier == identifier)
    }

    pub fn leaves(&self) -> impl Iterator<Item = &ResolvedEntry> {
        self.found.iter().filter(|f| !f.entry.is_nested_catalog())
    }

    /// Print the warnings and hand over everything found, catalogs included.
    pub fn into_found(self) -> Vec<ResolvedEntry> {
        for warning in &self.warnings {
            eprintln!("warning: {warning}");
        }
        self.found
    }

    fn walk(&mut self, catalog: &AiCatalog, base: &str, depth: usize) {
        let in_registry = base == REGISTRY;
        for entry in &catalog.entries {
            self.found.push(ResolvedEntry {
                entry: entry.clone(),
                source_catalog_url: base.to_string(),
            });
            if !entry.is_nested_catalog() {
                continue;
            }
            if !in_registry && depth >= MAX_DEPTH {
                self.warn(entry, Error::MaxDepthExceeded(MAX_DEPTH));
                continue;
            }
            let opened = self.open(entry, base).and_then(|opened| {
                opened
                    .map(|(bytes, child_base)| Ok((serde_json::from_slice(&bytes)?, child_base)))
                    .transpose()
            });
            match opened {
                Ok(Some((child, child_base))) => {
                    self.path.push(child_base.clone());
                    self.walk(&child, &child_base, if in_registry { 0 } else { depth + 1 });
                    self.path.pop();
                }
                Ok(None) => {}
                Err(e) => self.warn(entry, e),
            }
        }
    }

    fn warn(&mut self, entry: &CatalogEntry, reason: impl std::fmt::Display) {
        self.warnings.push(format!(
            "skipping nested catalog \"{}\": {reason}",
            entry.identifier
        ));
    }

    /// The bytes of the catalog `entry` refers to, and the base for its own references.
    fn open(&mut self, entry: &CatalogEntry, base: &str) -> Result<Option<(Vec<u8>, String)>> {
        if base == REGISTRY {
            let ext = cli_extension(entry)
                .ok_or_else(|| Error::Other("registry entry has no bookkeeping".into()))?;
            let field = |name: &str| ext.get(name).and_then(|v| v.as_str());
            let (Some(hash), Some(source)) = (field("contentHash"), field("sourceUrl")) else {
                return Err(Error::Other("registry entry is incomplete".into()));
            };
            return Ok(Some((self.cache.read_object(hash)?, source.to_string())));
        }
        if let Some(data) = &entry.data {
            return Ok(Some((serde_json::to_vec_pretty(data)?, base.to_string())));
        }
        let Some(reference) = &entry.url else {
            return Ok(None);
        };
        let url = join(base, reference)?;
        if !allowed(base, &url) {
            return Err(Error::Other(format!("{url} may not be loaded from {base}")));
        }
        if self.path.contains(&url) {
            return Err(Error::CircularReference(url));
        }
        Ok(Some((self.load(&url)?, url)))
    }

    fn load(&mut self, url: &str) -> Result<Vec<u8>> {
        let Some(client) = &self.client else {
            let hash = self.refs.get(url).ok_or_else(|| {
                Error::Other(format!(
                    "{url} is not in the local cache; run `ai-catalog catalog update`"
                ))
            })?;
            return self.cache.read_object(hash);
        };
        let bytes = fetch_catalog(url, client)?;
        serde_json::from_slice::<AiCatalog>(&bytes)?;
        self.store(url, &bytes)?;
        Ok(bytes)
    }

    fn store(&mut self, url: &str, bytes: &[u8]) -> Result<String> {
        let hash = content_hash(bytes);
        self.cache.store_object(&hash, bytes)?;
        self.refs.insert(url.to_string(), hash.clone());
        Ok(hash)
    }
}

/// A reference found in a catalog may point at an `https` URL, or at a local
/// file only when the catalog is a local file itself. A remote catalog must
/// never be able to make the CLI read from the local disk.
pub fn allowed(base: &str, url: &str) -> bool {
    url.starts_with("https://") || (is_file_url(url) && is_file_url(base))
}

/// Namespace for the CLI's own bookkeeping. The specification requires every
/// extension key to be a URL or a reverse-DNS string, so the CLI keeps its
/// fields under one key rather than writing them at the top level.
pub const CLI_EXTENSION_KEY: &str = "org.ai-catalog.ai-catalog-cli";

/// Identifier for a registry entry, derived from its name. Names are unique
/// (case-insensitively), so registering identical content twice, or updating
/// a catalog, never produces a duplicate or changing identifier.
pub fn registry_identifier(kind: &str, name: &str) -> String {
    let hash = content_hash(name.to_lowercase().as_bytes());
    format!("urn:ai-catalog:{kind}:{}", &hash[..8])
}

/// Build the `extensions` map the CLI attaches to a registry entry.
pub fn make_entry_extensions(
    source_url: &str,
    hash: &str,
    entry_count: usize,
) -> BTreeMap<String, serde_json::Value> {
    let mut extensions = BTreeMap::new();
    extensions.insert(
        CLI_EXTENSION_KEY.to_string(),
        json!({
            "sourceUrl": source_url,
            "lastUpdated": chrono::Utc::now().to_rfc3339(),
            "contentHash": hash,
            "entryCount": entry_count,
        }),
    );
    extensions
}

/// Read the CLI's own extension block from an entry.
pub fn cli_extension(entry: &CatalogEntry) -> Option<&serde_json::Value> {
    entry.extensions.as_ref()?.get(CLI_EXTENSION_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::local_path_to_file_url;
    use std::path::Path;

    fn write_catalog(dir: &Path, name: &str, entries: &[(&str, Option<&str>)]) -> String {
        let entries: Vec<_> = entries
            .iter()
            .map(|(id, nested)| match nested {
                Some(url) => json!({
                    "identifier": id, "type": "application/ai-catalog+json", "url": url
                }),
                None => json!({
                    "identifier": id, "type": "application/json", "url": "https://example.com/a.json"
                }),
            })
            .collect();
        let path = dir.join(name);
        let catalog = json!({ "specVersion": "1.0", "entries": entries });
        std::fs::write(&path, catalog.to_string()).unwrap();
        local_path_to_file_url(path.to_str().unwrap()).unwrap()
    }

    fn fetch<'a>(cache: &'a CacheManager, root: &str) -> Resolver<'a> {
        let mut resolver = Resolver::online(cache).unwrap();
        resolver.fetch(root).unwrap();
        resolver
    }

    fn leaf_ids<'a>(resolver: &'a Resolver) -> Vec<&'a str> {
        resolver
            .leaves()
            .map(|f| f.entry.identifier.as_str())
            .collect()
    }

    #[test]
    fn remote_catalogs_cannot_reach_local_files() {
        assert!(allowed(
            "https://a.example/c.json",
            "https://b.example/c.json"
        ));
        assert!(allowed("file:///a/c.json", "file:///a/d.json"));
        assert!(allowed("file:///a/c.json", "https://b.example/c.json"));
        assert!(!allowed("https://a.example/c.json", "file:///etc/passwd"));
        assert!(!allowed("file:///a/c.json", "http://b.example/c.json"));
    }

    #[test]
    fn follows_nested_catalogs_and_rejects_cycles() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = CacheManager::with_base_dir(dir.path().join("cache"));
        write_catalog(
            dir.path(),
            "b.json",
            &[("b", None), ("back", Some("a.json"))],
        );
        let root = write_catalog(
            dir.path(),
            "a.json",
            &[("a", None), ("child", Some("b.json"))],
        );

        let resolver = fetch(&cache, &root);

        assert_eq!(leaf_ids(&resolver), ["a", "b"]);
        assert!(resolver.warnings.iter().any(|w| w.contains("circular")));
    }

    #[test]
    fn rejects_a_cycle_that_does_not_include_the_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = CacheManager::with_base_dir(dir.path().join("cache"));
        write_catalog(
            dir.path(),
            "c.json",
            &[("c", None), ("back", Some("b.json"))],
        );
        write_catalog(
            dir.path(),
            "b.json",
            &[("b", None), ("next", Some("c.json"))],
        );
        let root = write_catalog(
            dir.path(),
            "a.json",
            &[("a", None), ("child", Some("b.json"))],
        );

        let resolver = fetch(&cache, &root);

        assert_eq!(leaf_ids(&resolver), ["a", "b", "c"]);
        assert!(resolver.warnings.iter().any(|w| w.contains("circular")));
    }

    #[test]
    fn a_catalog_may_appear_in_two_branches() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = CacheManager::with_base_dir(dir.path().join("cache"));
        write_catalog(dir.path(), "shared.json", &[("shared", None)]);
        write_catalog(dir.path(), "left.json", &[("left", Some("shared.json"))]);
        write_catalog(dir.path(), "right.json", &[("right", Some("shared.json"))]);
        let root = write_catalog(
            dir.path(),
            "root.json",
            &[("l", Some("left.json")), ("r", Some("right.json"))],
        );

        let resolver = fetch(&cache, &root);

        assert_eq!(leaf_ids(&resolver), ["shared", "shared"]);
        assert!(resolver.warnings.is_empty());
    }

    #[test]
    fn stops_at_the_maximum_depth() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = CacheManager::with_base_dir(dir.path().join("cache"));
        for level in (0..=MAX_DEPTH + 1).rev() {
            let next = format!("l{}.json", level + 1);
            let id = format!("leaf{level}");
            let mut entries = vec![(id.as_str(), None)];
            if level <= MAX_DEPTH {
                entries.push(("next", Some(next.as_str())));
            }
            write_catalog(dir.path(), &format!("l{level}.json"), &entries);
        }
        let root = local_path_to_file_url(dir.path().join("l0.json").to_str().unwrap()).unwrap();

        let resolver = fetch(&cache, &root);

        assert_eq!(resolver.leaves().count(), MAX_DEPTH + 1);
        assert!(
            resolver
                .warnings
                .iter()
                .any(|w| w.contains("nesting depth"))
        );
    }
}
