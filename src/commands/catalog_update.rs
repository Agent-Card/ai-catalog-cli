// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use colored::Colorize;

use crate::cache::CacheManager;
use crate::error::{Error, Result};
use crate::resolver::{Resolver, cli_extension, make_entry_extensions};

pub fn execute(name: &str) -> Result<()> {
    let cache = CacheManager::new()?;
    cache.ensure_dirs()?;
    let mut registry = cache.read_registry()?;
    let idx = registry
        .entries
        .iter()
        .position(|e| {
            e.display_name
                .as_deref()
                .map(|n| n.eq_ignore_ascii_case(name))
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            Error::CatalogNotFound(format!(
                "no catalog named \"{name}\". Use `ai-catalog catalog list` to see registered catalogs."
            ))
        })?;
    let source_url = cli_extension(&registry.entries[idx])
        .and_then(|m| m.get("sourceUrl"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Other(format!("catalog \"{name}\" has no recorded sourceUrl")))?
        .to_string();
    let old_hash = cli_extension(&registry.entries[idx])
        .and_then(|m| m.get("contentHash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    print!("Updating \"{}\"... ", name.bold());
    let mut resolver = Resolver::online(&cache)?;
    let new_hash = resolver.fetch(&source_url)?;
    let entry_count = resolver.leaves().count();
    resolver.into_found();
    if old_hash.as_deref() == Some(&new_hash) {
        println!("{}", "(up to date)".dimmed());
        return Ok(());
    }
    let file_url = cache.object_file_url(&new_hash);
    let extensions = make_entry_extensions(&source_url, &new_hash, entry_count);
    registry.entries[idx].url = Some(file_url);
    registry.entries[idx].updated_at = Some(chrono::Utc::now().to_rfc3339());
    registry.entries[idx].extensions = Some(extensions);
    cache.write_registry(&registry)?;
    println!(
        "{} ({entry_count} entries, hash {})",
        "updated".green(),
        &new_hash[..8]
    );
    Ok(())
}
