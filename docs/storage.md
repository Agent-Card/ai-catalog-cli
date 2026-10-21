# Local storage

`ai-catalog` keeps everything it fetches in one directory, `~/.ai-catalog/`.
Set `AI_CATALOG_CACHE_DIR` to use another one.

```
~/.ai-catalog/
├── catalog.json      # the registry: an AI Catalog listing the registered catalogs
├── refs.json         # source → hash of the blob fetched from it
└── objects/
    └── <sha256>.json # catalog documents, named by the SHA-256 of their bytes
```

`search`, `show` and `pull` look entries up here, so finding an entry works
offline. The network is used to register or update a catalog, for `--scope <url>`,
and by `pull` to download the artifact an entry points to.

## Sources

A catalog is registered from one of two sources, and both end up in the same
`objects/` and `catalog.json`.

| | Command | What is stored |
|---|---|---|
| URL or file | `catalog add <name> <url>` | The document as served, plus every nested catalog it references |
| OCI image layout | `oci add <name> <layout-dir>` | The catalog unpacked from the layout |

An [OCI image layout](https://github.com/opencontainers/image-spec/blob/main/image-layout.md)
is read once and copied. It stays where it is and is never modified. Only the
catalog document is imported; trust manifests and Cosign referrers in the
layout are not.

Nested catalogs follow the rules in the specification: `https` only (or a
`file://` reference from a local catalog), at most four levels deep, no
cycles, at most 10 MiB each. A catalog that breaks them is skipped with a
warning.

## The registry

`catalog.json` is a valid AI Catalog. Each entry is one registered catalog:
its `url` points at the blob in `objects/`, and the CLI keeps its own
bookkeeping under the `org.ai-catalog.ai-catalog-cli` key of `extensions`.

```json
{
  "identifier": "urn:ai-catalog:local:4b5e57f6",
  "displayName": "web",
  "type": "application/ai-catalog+json",
  "url": "file:///home/me/.ai-catalog/objects/69ea4777….json",
  "extensions": {
    "org.ai-catalog.ai-catalog-cli": {
      "sourceUrl": "https://example.com/ai-catalog.json",
      "contentHash": "69ea4777…",
      "entryCount": 1,
      "lastUpdated": "2026-10-21T13:41:19+00:00"
    }
  }
}
```

A catalog added with `oci add` also records `ociLayoutPath`, `ociRefName` and
`sourceType`, and its `sourceUrl` is `oci+layout:<layout-dir>:<tag>`.

`refs.json` maps each source (and each nested catalog URL) to the hash of the
blob fetched from it, which is how nested catalogs are found offline.

## Identifiers

A registry entry's identifier is `urn:ai-catalog:<kind>:<hash>`, where `<hash>`
is the first eight hex digits of the SHA-256 of the lowercased name. Names are
unique, so the identifier stays the same when a catalog is updated.

| Added with | `<kind>` |
|---|---|
| `catalog add` | `local` |
| `oci add` | `oci` |

## Scoping

`search`, `show` and `pull` cover every registered catalog. `oci search`,
`oci show` and `oci pull` cover only those with the `urn:ai-catalog:oci:`
prefix.

## Updating and removing

- `catalog update <name>` fetches the source again and stores the new blob if
  the content changed. It supports URL and file sources; to refresh a catalog
  added with `oci add`, remove it and add it again.
- `catalog remove <name>` deletes the catalog's blob unless another registered
  catalog uses the same one.
