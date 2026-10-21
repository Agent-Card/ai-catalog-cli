// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::io::Write;
use std::path::{Path, PathBuf};

use colored::Colorize;

use ai_catalog::CatalogEntry;

use crate::cache::CacheManager;
use crate::error::{Error, Result};
use crate::fetch::{build_client, fetch_artifact, join};
use crate::resolver::{ResolvedEntry, Resolver, allowed};

use super::{CATALOG_MIME_TYPE, find_entry_in_registry, find_entry_in_scope, leaf_entries_of};

/// Pull a catalog entry by identifier and write its content to disk.
///
/// `scope` restricts the search to a specific catalog (by registered name or URI).
/// `media_type` disambiguates when the entry resolves to a catalog:
///   - None → error
///   - "application/ai-catalog+json" → write the catalog JSON itself
///   - other → find the single child entry of that type and write it
pub fn execute(
    identifier: &str,
    output_path: Option<&str>,
    scope: Option<&str>,
    media_type: Option<&str>,
) -> Result<()> {
    let cache = CacheManager::new()?;

    let found = if let Some(scope_val) = scope {
        find_entry_in_scope(identifier, scope_val, &cache)?
    } else {
        // First search the local registry without hitting the network.
        let found = find_entry_in_registry(identifier, &cache, None)?;
        if found.is_none() {
            // The identifier might itself be a URL – try fetching it as a catalog.
            if identifier.starts_with("http://") || identifier.starts_with("https://") {
                return pull_from_url(identifier, output_path, &cache);
            }
            return Err(Error::EntryNotFound(format!(
                "{identifier} — run `ai-catalog search {identifier}` or `ai-catalog catalog add <name> <url>` first"
            )));
        }
        found
    };

    let found = found.ok_or_else(|| Error::EntryNotFound(identifier.to_string()))?;

    dispatch_pull(&found, output_path, media_type, &cache)
}

/// Core dispatch: handle media_type gating and catalog-vs-leaf branching.
///
/// Also used by `oci_pull`.
pub(crate) fn dispatch_pull(
    found: &ResolvedEntry,
    output_path: Option<&str>,
    media_type: Option<&str>,
    cache: &CacheManager,
) -> Result<()> {
    let entry = &found.entry;
    if !entry.is_nested_catalog() {
        // Leaf entry — check media_type guard
        if let Some(mt) = media_type
            && entry.entry_type != mt
        {
            return Err(Error::Other(format!(
                "entry \"{}\" has type \"{}\" but --media-type \"{}\" was requested",
                entry.identifier, entry.entry_type, mt
            )));
        }
        return write_entry(found, output_path, cache);
    }

    // Catalog entry — branch on --media-type
    match media_type {
        None => Err(Error::Other(format!(
            "\"{}\" refers to a catalog, not a pullable artifact.\n  \
             Use --media-type {CATALOG_MIME_TYPE} to fetch the catalog JSON itself, \
             or --media-type <type> to pull the single entry of that type within it.",
            entry.identifier
        ))),
        Some(mt) if mt == CATALOG_MIME_TYPE => write_entry(found, output_path, cache),
        Some(mt) => {
            let leaves = leaf_entries_of(found, cache)?;
            let matches: Vec<&_> = leaves.iter().filter(|e| e.entry.entry_type == mt).collect();
            match matches.len() {
                0 => Err(Error::EntryNotFound(format!(
                    "no entries of type \"{mt}\" found in catalog \"{}\"",
                    entry.identifier
                ))),
                1 => write_entry(matches[0], output_path, cache),
                n => Err(Error::Other(format!(
                    "{n} entries of type \"{mt}\" found in catalog \"{}\"; \
                     use --scope to narrow the search",
                    entry.identifier
                ))),
            }
        }
    }
}

fn pull_from_url(url: &str, output_path: Option<&str>, cache: &CacheManager) -> Result<()> {
    println!("Fetching catalog from {}...", url.cyan());
    let mut resolver = Resolver::online(cache)?;
    let hash = resolver.fetch(url)?;
    let entries = resolver.into_found();
    println!(
        "{} fetched {} entries from {}",
        "✓".green(),
        entries
            .iter()
            .filter(|f| !f.entry.is_nested_catalog())
            .count(),
        url
    );
    let bytes = cache.read_object(&hash)?;
    if output_path == Some("-") {
        std::io::stdout().write_all(&bytes)?;
    } else {
        let dest = resolve_output_path(output_path, url, "json");
        std::fs::write(&dest, &bytes)?;
        println!("{} written to {}", "✓".green(), dest.display());
    }
    Ok(())
}

fn write_entry(
    found: &ResolvedEntry,
    output_path: Option<&str>,
    cache: &CacheManager,
) -> Result<()> {
    let entry = &found.entry;
    let (bytes, served) = if let Some(data) = &entry.data {
        (serde_json::to_vec_pretty(data)?, None)
    } else if entry.is_nested_catalog() {
        let bytes = Resolver::offline(cache)?.catalog_bytes(entry, &found.source_catalog_url)?;
        (bytes.clone(), Some(bytes))
    } else if let Some(reference) = &entry.url {
        let url = join(&found.source_catalog_url, reference)?;
        if !allowed(&found.source_catalog_url, &url) {
            return Err(Error::Other(format!(
                "{url} may not be loaded from {}",
                found.source_catalog_url
            )));
        }
        let bytes = fetch_artifact(&url, &build_client()?)?;
        (bytes.clone(), Some(bytes))
    } else {
        return Err(Error::Other(format!(
            "entry \"{}\" has neither inline data nor a URL to fetch",
            entry.identifier
        )));
    };
    let verified = verify_subject_digest(entry, served.as_deref())?;

    if output_path == Some("-") {
        std::io::stdout().write_all(&bytes)?;
        return Ok(());
    }

    let ext = if entry.is_nested_catalog() {
        "json"
    } else {
        extension_for_type(&entry.entry_type)
    };
    let dest = resolve_output_path(output_path, &entry.identifier, ext);
    std::fs::write(&dest, &bytes)?;
    println!(
        "{} \"{}\" written to {}",
        "✓".green(),
        entry.identifier.bold(),
        dest.display()
    );
    if verified {
        println!("  digest verified");
    }
    Ok(())
}

/// Check the content against `subject.digest` of the entry's trust manifest.
///
/// For an artifact behind a `url` the digest covers the bytes as served
/// (`served`); for inline `data` it covers the JCS form of the value. Returns
/// whether there was a digest to check. This says nothing about who published
/// the content: signatures are not verified.
fn verify_subject_digest(entry: &CatalogEntry, served: Option<&[u8]>) -> Result<bool> {
    let Some(subject) = entry
        .trust_manifest
        .as_ref()
        .and_then(|manifest| manifest.subject.as_ref())
    else {
        return Ok(false);
    };
    let canonical;
    let content = match (&entry.data, served) {
        (Some(data), _) => {
            canonical = ai_catalog_trust::canonicalize(serde_json::to_vec(data)?).map_err(|e| {
                Error::Other(format!("cannot canonicalize \"{}\": {e}", entry.identifier))
            })?;
            canonical.as_bytes()
        }
        (None, Some(bytes)) => bytes,
        (None, None) => return Ok(false),
    };
    match ai_catalog_trust::verify_digest(&subject.digest, content) {
        Ok(true) => Ok(true),
        Ok(false) => Err(Error::Other(format!(
            "content of \"{}\" does not match its subject.digest {}",
            entry.identifier, subject.digest
        ))),
        Err(e) => Err(Error::Other(format!(
            "cannot check subject.digest of \"{}\": {e}",
            entry.identifier
        ))),
    }
}

fn resolve_output_path(output_path: Option<&str>, stem: &str, ext: &str) -> PathBuf {
    let filename = safe_filename(stem, ext);
    match output_path {
        None => PathBuf::from(&filename),
        Some(p) => {
            let path = Path::new(p);
            if path.is_dir() {
                path.join(filename)
            } else {
                path.to_path_buf()
            }
        }
    }
}

fn safe_filename(stem: &str, ext: &str) -> String {
    let base = stem
        .rsplit(':')
        .next()
        .or_else(|| stem.rsplit('/').next())
        .unwrap_or(stem);
    let sanitised: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{sanitised}.{ext}")
}

fn extension_for_type(mime: &str) -> &'static str {
    match mime {
        "application/json" | "application/ai-catalog+json" => "json",
        "application/yaml" | "text/yaml" => "yaml",
        "text/plain" => "txt",
        "application/octet-stream" => "bin",
        _ => "json",
    }
}
