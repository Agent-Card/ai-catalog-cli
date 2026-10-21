// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::{OCI_PREFIX, find_entry_in_registry, pull::dispatch_pull};
use crate::cache::CacheManager;
use crate::error::{Error, Result};

/// Pull an OCI-sourced catalog entry by identifier and write its content to disk.
///
/// Only searches catalogs added via `oci add`. If `output_path` is a directory
/// (or omitted), a filename is derived from the identifier.
pub fn execute(
    identifier: &str,
    output_path: Option<&str>,
    media_type: Option<&str>,
) -> Result<()> {
    let cache = CacheManager::new()?;

    let entry = find_entry_in_registry(identifier, &cache, Some(OCI_PREFIX))?
        .ok_or_else(|| {
            Error::EntryNotFound(format!(
                "{identifier} — run `ai-catalog oci search {identifier}` or `ai-catalog oci add <name> <layout-dir>` first"
            ))
        })?;

    dispatch_pull(&entry, output_path, media_type, &cache)
}
