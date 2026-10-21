// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::{Args, Parser, Subcommand};

/// Inspect, validate, publish and consume AI Catalog documents.
///
/// Use '-' as a path to read from stdin.
#[derive(Debug, Parser)]
#[command(name = "ai-catalog", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate a catalog against the specification
    Validate {
        /// Catalog file, or '-' for stdin
        path: String,
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Pretty-print a catalog
    Format {
        /// Catalog file, or '-' for stdin
        path: String,
    },
    /// Package catalogs as OCI artifacts
    Oci {
        #[command(subcommand)]
        command: OciCommand,
    },
    /// Manage the registered catalogs
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
    /// Search the registered catalogs
    Search(SearchArgs),
    /// Show an entry
    Show {
        #[command(flatten)]
        args: ShowArgs,
        /// Only look in this registered catalog (name or source), or in this catalog URL
        #[arg(short, long)]
        scope: Option<String>,
    },
    /// Download an entry
    Pull {
        #[command(flatten)]
        args: PullArgs,
        /// Only look in this registered catalog (name or source), or in this catalog URL
        #[arg(short, long)]
        scope: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CatalogCommand {
    /// Register a catalog from a URL or a file
    Add {
        /// Name to register the catalog under
        name: String,
        /// URL or path of the catalog
        url: String,
    },
    /// List the registered catalogs
    List {
        /// Print the registry as a catalog document
        #[arg(short, long)]
        json: bool,
    },
    /// Remove a registered catalog
    Remove {
        /// Registered name or source of the catalog
        name_or_url: String,
    },
    /// Fetch a registered catalog again
    Update {
        /// Registered name of the catalog
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum OciCommand {
    /// Pack a catalog into the JSON artifact set the SDK uses
    Pack {
        /// Catalog file, or '-' for stdin
        path: String,
    },
    /// Unpack a JSON artifact set into a catalog
    Unpack {
        /// Artifact set file, or '-' for stdin
        path: String,
    },
    /// Write a catalog as an OCI image layout
    ExportLayout {
        #[command(flatten)]
        publish: PublishArgs,
        /// Directory to write the layout to
        layout_dir: String,
    },
    /// Read the catalog out of an OCI image layout
    UnpackLayout {
        /// OCI image layout directory
        layout_dir: String,
        /// Tag to read (default: the first manifest in the layout)
        #[arg(long)]
        ref_name: Option<String>,
    },
    /// Push a catalog to an OCI registry using oras
    Push {
        #[command(flatten)]
        publish: PublishArgs,
        /// Target reference, for example registry.example.com/team/catalog
        target: String,
        /// Copy to an OCI image layout directory instead of a registry
        #[arg(long)]
        to_oci_layout_path: Option<String>,
        /// Use plain HTTP for the registry
        #[arg(long)]
        plain_http: bool,
        /// Skip TLS verification
        #[arg(long)]
        insecure: bool,
    },
    /// Register a catalog from an OCI image layout
    Add {
        /// Name to register the catalog under
        name: String,
        /// OCI image layout directory
        layout_dir: String,
        /// Tag to read (default: the first manifest in the layout)
        #[arg(short = 'r', long)]
        ref_name: Option<String>,
    },
    /// Search the catalogs registered from OCI layouts
    Search(SearchArgs),
    /// Show an entry of a catalog registered from an OCI layout
    Show(ShowArgs),
    /// Download an entry of a catalog registered from an OCI layout
    Pull(PullArgs),
}

/// What `oci export-layout` and `oci push` publish, and how.
#[derive(Debug, Args)]
pub struct PublishArgs {
    /// Catalog file, or '-' for stdin
    pub path: String,
    /// Tag to give the catalog in the layout
    #[arg(long, default_value = "latest")]
    pub tag: String,
    /// Sign trust manifests with this Cosign private key
    #[arg(long)]
    pub cosign_key: Option<String>,
    /// Public key to attach next to the signatures (defaults to the one for --cosign-key)
    #[arg(long, requires = "cosign_key")]
    pub cosign_public_key: Option<String>,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Text to look for in identifiers, names, descriptions and tags
    pub keyword: String,
    /// Treat the keyword as a regular expression
    #[arg(short = 'r', long)]
    pub regex: bool,
    /// Maximum number of results
    #[arg(short = 'n', long, default_value_t = 50)]
    pub limit: usize,
    /// Print the results as a catalog document
    #[arg(short, long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Identifier of the entry
    pub identifier: String,
    /// Show the entry of this media type when the identifier names a catalog
    #[arg(short, long)]
    pub media_type: Option<String>,
    /// Print the entry as JSON
    #[arg(short, long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PullArgs {
    /// Identifier of the entry
    pub identifier: String,
    /// Pull the entry of this media type when the identifier names a catalog
    #[arg(short, long)]
    pub media_type: Option<String>,
    /// File or directory to write to, or '-' for stdout
    #[arg(short, long)]
    pub output: Option<String>,
}
