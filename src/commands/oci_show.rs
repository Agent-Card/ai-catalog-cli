// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::{OCI_PREFIX, OutputFormat, find_entry_in_registry, show::dispatch_show_entry};
use crate::cache::CacheManager;
use crate::error::{Error, Result};

/// Show full details of an OCI-sourced catalog entry by identifier.
///
/// Only searches catalogs added via `oci add`.
pub fn execute(identifier: &str, output: OutputFormat, media_type: Option<&str>) -> Result<()> {
    let cache = CacheManager::new()?;

    let entry = find_entry_in_registry(identifier, &cache, Some(OCI_PREFIX))?
        .ok_or_else(|| {
            Error::EntryNotFound(format!(
                "{identifier} — run `ai-catalog oci search {identifier}` or `ai-catalog oci add <name> <layout-dir>` first"
            ))
        })?;

    dispatch_show_entry(&entry, output, media_type, &cache)
}
