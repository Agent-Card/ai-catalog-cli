# ai-catalog Privacy Policy

`ai-catalog` is a command-line tool for reading AI Catalog documents, caching
them locally, and publishing catalog content to OCI registries. It has no
server component, no telemetry, and no vendor operated by the AI-Catalog
project collects any data from running it. Requests go to the addresses you
name and to any address referenced by a catalog document you resolve.

## What it stores on your machine

The cache lives in `~/.ai-catalog/`, or wherever `AI_CATALOG_CACHE_DIR`
points. It holds the catalogs you have added with the time you added them
(`catalog.json`), a map from every source URL fetched to the content hash
retrieved from it (`refs.json`), and the catalog documents themselves,
verbatim, under their SHA-256 hash (`objects/`). So the cache records which
catalogs you have used and when, in plain text, along with any personal or
proprietary data contained in a document that was fetched. Nothing is
encrypted, and files are written with your account's default permissions.

`oci push` also writes the content being pushed to a temporary directory
named `ai-catalog-oras-layout-*` under your system temp location, with
default permissions. It is deleted when the push finishes, but cleanup is
best-effort and an interrupted push can leave it behind.

## What it sends, and to whom

Resolving a catalog by `http://` or `https://` URL issues a GET to it with
the user agent `ai-catalog/<version>`; the server sees that request, your IP
address, and that user agent. Redirects are followed.

A catalog document may reference nested catalogs and artifacts by URL, and
those are fetched too, recursively, up to four levels deep. Those URLs are
chosen by whoever wrote the document and may point at unrelated hosts; each
fetched document is cached and its URL recorded. A catalog read from a local
path or `file://` URL makes no request of its own, but any `http://` or
`https://` URL it references is still fetched.

`oci push` uploads the content you select to the registry reference you name.

Passing `--cosign-key` signs the content by invoking `cosign sign-blob`.
`ai-catalog` does not disable cosign's transparency-log upload, so cosign's
default applies: current versions publish the signature, a digest of the
signed content, and a timestamp to the public Sigstore transparency log,
where entries are permanent and cannot be deleted. The content itself is not
uploaded there. Without `--cosign-key`, cosign is never invoked.

## Registry authentication and external tools

`ai-catalog` contains no credential handling. It does not read, store, prompt
for, or transmit registry passwords, tokens, or configuration files. Registry
operations are delegated to [`oras`][oras] and signing to [`cosign`][cosign],
each invoked as a subprocess that inherits your environment. Those tools
authenticate using their own configuration — typically the Docker credential
store or a helper — so credentials are never parsed by `ai-catalog` and are
never written to its cache.

## Insecure transport modes

`oci push` accepts `--plain-http` (send to the destination registry over
unencrypted HTTP) and `--insecure` (skip its TLS certificate verification).
Both are off by default. With either, the content being pushed and any
credentials `oras` sends to authenticate can be read or modified by anyone on
the network path. They exist for local and development registries.

## Retention and deletion

The cache has no expiry. `ai-catalog catalog remove <name-or-url>` removes a
catalog from `catalog.json` and deletes the cached copy of that document, but
not documents cached from nested catalogs it referenced, nor their entries in
`refs.json` — so `refs.json` accumulates a record of every catalog URL the
tool has fetched. `rm -rf ~/.ai-catalog` erases everything stored locally;
check your system temp directory for leftover `ai-catalog-oras-layout-*`
directories as well. Local deletion has no effect on data pushed to a
registry or on transparency-log entries created by signing.

## Contact

Report a privacy concern via the issue tracker:
<https://github.com/Agent-Card/ai-catalog-cli/issues>.

[oras]: https://oras.land/
[cosign]: https://github.com/sigstore/cosign
