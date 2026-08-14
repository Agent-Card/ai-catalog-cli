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

use ai_catalog::CatalogEntry;

use crate::cache::CacheManager;
use crate::error::{Error, Result};
use crate::resolver::{cli_extension, find_entry_by_id_in_url, resolve_and_cache};

pub const CATALOG_MIME_TYPE: &str = "application/ai-catalog+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
}

/// Resolve an entry within a specific scope (registered catalog name or URI).
pub(crate) async fn find_entry_in_scope(
    identifier: &str,
    scope: &str,
    cache: &CacheManager,
    client: &reqwest::Client,
) -> Result<Option<CatalogEntry>> {
    if scope.contains("://") {
        if scope.starts_with("file://") {
            find_entry_by_id_in_url(identifier, scope, cache)
        } else {
            cache.ensure_dirs()?;
            resolve_and_cache(scope, client, cache).await?;
            let url_to_hash = cache.read_refs()?;
            if let Some(hash) = url_to_hash.get(scope) {
                find_entry_by_id_in_url(identifier, &cache.object_file_url(hash), cache)
            } else {
                Ok(None)
            }
        }
    } else {
        let registry = cache.read_registry()?;
        let catalog_entry = registry.entries.iter().find(|e| {
            e.display_name
                .as_deref()
                .map(|n| n.eq_ignore_ascii_case(scope))
                .unwrap_or(false)
                || cli_extension(e)
                    .and_then(|m| m.get("sourceUrl"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.eq_ignore_ascii_case(scope))
                    .unwrap_or(false)
        });
        let catalog_entry = catalog_entry.ok_or_else(|| {
            Error::CatalogNotFound(format!(
                "no catalog matching \"{scope}\" found. Use `ai-catalog catalog list` to see registered catalogs."
            ))
        })?;
        let file_url = catalog_entry
            .url
            .as_deref()
            .ok_or_else(|| Error::Other(format!("catalog \"{scope}\" has no local file URL")))?;
        find_entry_by_id_in_url(identifier, file_url, cache)
    }
}
