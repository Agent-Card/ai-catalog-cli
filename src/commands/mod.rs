// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod catalog_add;
pub mod catalog_list;
pub mod catalog_remove;
pub mod catalog_update;
pub mod oci_add;
pub mod oci_pull;
pub mod oci_search;
pub mod oci_show;
pub mod pull;
pub mod search;
pub mod show;

use std::collections::HashSet;

use ai_catalog::{AiCatalog, CatalogEntry};

use crate::cache::CacheManager;
use crate::error::{Error, Result};
use crate::resolver::{REGISTRY, ResolvedEntry, Resolver, cli_extension};

pub const CATALOG_MIME_TYPE: &str = "application/ai-catalog+json";

/// Registry identifier prefix of catalogs registered from an OCI image layout.
pub const OCI_PREFIX: &str = "urn:ai-catalog:oci:";

/// Wrap search results in a catalog. Nested catalogs may legally reuse an
/// identifier, so the flattened list is de-duplicated to keep the document
/// valid: `(identifier, version)` must be unique.
pub(crate) fn catalog_of(entries: impl IntoIterator<Item = CatalogEntry>) -> AiCatalog {
    let mut seen = HashSet::new();
    let entries = entries
        .into_iter()
        .filter(|e| seen.insert((e.identifier.clone(), e.version.clone())))
        .collect();
    AiCatalog {
        spec_version: "1.0".to_string(),
        host: None,
        entries,
        signature: None,
        extensions: None,
        extra_fields: Default::default(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
}

/// Resolve an entry within a specific scope: a registered catalog (by name or
/// source), or a catalog URL, which is fetched and cached on the way.
pub(crate) fn find_entry_in_scope(
    identifier: &str,
    scope: &str,
    cache: &CacheManager,
) -> Result<Option<ResolvedEntry>> {
    if scope.contains("://") {
        let mut resolver = Resolver::online(cache)?;
        resolver.fetch(scope)?;
        return Ok(resolver
            .into_found()
            .into_iter()
            .find(|f| f.entry.identifier == identifier));
    }
    let registry = cache.read_registry()?;
    let catalog_entry = registry
        .entries
        .iter()
        .find(|e| {
            e.display_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(scope))
                || cli_extension(e)
                    .and_then(|m| m.get("sourceUrl"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(scope))
        })
        .ok_or_else(|| {
            Error::CatalogNotFound(format!(
                "no catalog matching \"{scope}\" found. Use `ai-catalog catalog list` to see registered catalogs."
            ))
        })?;
    let mut resolver = Resolver::offline(cache)?;
    resolver.descend(catalog_entry, REGISTRY)?;
    Ok(resolver
        .into_found()
        .into_iter()
        .find(|f| f.entry.identifier == identifier))
}

/// Find an entry by identifier in the registered catalogs, catalogs included.
pub(crate) fn find_entry_in_registry(
    identifier: &str,
    cache: &CacheManager,
    prefix: Option<&str>,
) -> Result<Option<ResolvedEntry>> {
    let mut resolver = Resolver::offline(cache)?;
    resolver.walk_registry(prefix)?;
    Ok(resolver.into_found().into_iter().find(|f| {
        f.entry.identifier == identifier && (prefix.is_none() || f.source_catalog_url != REGISTRY)
    }))
}

/// The entries that are not catalogs, of the registered catalogs.
pub(crate) fn leaf_entries_in_registry(
    cache: &CacheManager,
    prefix: Option<&str>,
) -> Result<Vec<ResolvedEntry>> {
    let mut resolver = Resolver::offline(cache)?;
    resolver.walk_registry(prefix)?;
    Ok(resolver
        .into_found()
        .into_iter()
        .filter(|f| !f.entry.is_nested_catalog())
        .collect())
}

/// The entries that are not catalogs, of the catalog `entry` points to or embeds.
pub(crate) fn leaf_entries_of(
    found: &ResolvedEntry,
    cache: &CacheManager,
) -> Result<Vec<ResolvedEntry>> {
    let mut resolver = Resolver::offline(cache)?;
    resolver.descend(&found.entry, &found.source_catalog_url)?;
    Ok(resolver
        .into_found()
        .into_iter()
        .filter(|f| !f.entry.is_nested_catalog())
        .collect())
}
