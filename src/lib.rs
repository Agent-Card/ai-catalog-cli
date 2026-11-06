// Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod cache;
mod cli;
pub mod commands;
pub mod error;
pub mod fetch;
pub mod resolver;
pub mod verify;

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ai_catalog::{AiCatalog, parse_file, parse_reader, write_writer};
use clap::{CommandFactory, Parser};

use ai_catalog_oci::{
    OciArtifactSet, attach_cosign_verification_artifacts, export_layout, import_layout,
    pack_catalog, unpack_catalog,
};
use ai_catalog_trust::canonicalize_trust_manifest;
use ai_catalog_validate::{ConformanceLevel, Diagnostic, validate};
use cli::{CatalogCommand, Cli, Command, OciCommand, PublishArgs, SearchArgs, SigstoreService};
use serde_json::json;
use sha2::{Digest as _, Sha256};

const ORAS_BIN_ENV: &str = "AI_CATALOG_ORAS_BIN";
const COSIGN_BIN_ENV: &str = "AI_CATALOG_COSIGN_BIN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

/// The streams a command reads from and writes to.
pub struct Io<'a> {
    pub stdin: &'a mut dyn Read,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CosignBlobBundle {
    message_signature: CosignMessageSignature,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CosignMessageSignature {
    signature: String,
}

pub fn run<I, S, R, W, E>(args: I, stdin: &mut R, stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    R: Read,
    W: Write,
    E: Write,
{
    let mut io = Io {
        stdin,
        out: stdout,
        err: stderr,
    };

    let cli = match Cli::try_parse_from(args.into_iter().map(|arg| arg.as_ref().to_owned())) {
        Ok(cli) => cli,
        Err(error) => {
            let rendered = error.render();
            if error.use_stderr() {
                write!(io.err, "{rendered}")?;
            } else {
                write!(io.out, "{rendered}")?;
            }
            return Ok(error.exit_code());
        }
    };

    let Some(command) = cli.command else {
        writeln!(io.out, "{}", Cli::command().render_help())?;
        return Ok(0);
    };

    match command {
        Command::Validate { path, json } => validate_command(&path, json, &mut io),
        Command::Format { path } => format_command(&path, &mut io),
        Command::Verify {
            path,
            json,
            did_documents,
            offline,
            strict,
        } => verify_command(&path, json, &did_documents, offline, strict, &mut io),
        Command::Oci { command } => oci_command(command, &mut io),
        Command::Catalog { command } => {
            let result = match command {
                CatalogCommand::Add { name, url } => commands::catalog_add::execute(&name, &url),
                CatalogCommand::List { json } => {
                    commands::catalog_list::execute(consumer_format(json))
                }
                CatalogCommand::Remove { name_or_url } => {
                    commands::catalog_remove::execute(&name_or_url)
                }
                CatalogCommand::Update { name } => commands::catalog_update::execute(&name),
            };
            consumer_result(result, &mut io)
        }
        Command::Search(args) => consumer_result(search(&args, commands::search::execute), &mut io),
        Command::Show { args, scope } => consumer_result(
            commands::show::execute(
                &args.identifier,
                consumer_format(args.json),
                scope.as_deref(),
                args.media_type.as_deref(),
            ),
            &mut io,
        ),
        Command::Pull { args, scope } => consumer_result(
            commands::pull::execute(
                &args.identifier,
                args.output.as_deref(),
                scope.as_deref(),
                args.media_type.as_deref(),
            ),
            &mut io,
        ),
    }
}

fn oci_command(command: OciCommand, io: &mut Io) -> io::Result<i32> {
    match command {
        OciCommand::Pack { path } => oci_pack_command(&path, io),
        OciCommand::Unpack { path } => oci_unpack_command(&path, io),
        OciCommand::ExportLayout {
            publish,
            layout_dir,
        } => oci_export_layout_command(&publish, &layout_dir, io),
        OciCommand::UnpackLayout {
            layout_dir,
            ref_name,
        } => oci_unpack_layout_command(&layout_dir, ref_name.as_deref(), io),
        OciCommand::Push {
            publish,
            target,
            to_oci_layout_path,
            plain_http,
            insecure,
        } => {
            let destination = PushDestination {
                target: &target,
                to_oci_layout_path: to_oci_layout_path.as_deref(),
                plain_http,
                insecure,
            };
            oci_push_command(&publish, &destination, io)
        }
        OciCommand::Add {
            name,
            layout_dir,
            ref_name,
        } => consumer_result(
            commands::oci_add::execute(&name, &layout_dir, ref_name.as_deref()),
            io,
        ),
        OciCommand::Search(args) => {
            consumer_result(search(&args, commands::oci_search::execute), io)
        }
        OciCommand::Show(args) => consumer_result(
            commands::oci_show::execute(
                &args.identifier,
                consumer_format(args.json),
                args.media_type.as_deref(),
            ),
            io,
        ),
        OciCommand::Pull(args) => consumer_result(
            commands::oci_pull::execute(
                &args.identifier,
                args.output.as_deref(),
                args.media_type.as_deref(),
            ),
            io,
        ),
    }
}

fn consumer_format(json: bool) -> commands::OutputFormat {
    if json {
        commands::OutputFormat::Json
    } else {
        commands::OutputFormat::Table
    }
}

fn search(
    args: &SearchArgs,
    execute: fn(&str, bool, usize, commands::OutputFormat) -> error::Result<()>,
) -> error::Result<()> {
    execute(
        &args.keyword,
        args.regex,
        args.limit,
        consumer_format(args.json),
    )
}

fn consumer_result(result: error::Result<()>, io: &mut Io) -> io::Result<i32> {
    match result {
        Ok(()) => Ok(0),
        Err(error) => {
            writeln!(io.err, "error: {error}")?;
            Ok(1)
        }
    }
}

fn validate_command(path: &str, json: bool, io: &mut Io) -> io::Result<i32> {
    let output_format = if json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    let catalog = match read_catalog(path, io.stdin) {
        Ok(catalog) => catalog,
        Err(error) => {
            write_parse_error(output_format, path, error, &mut io.out, &mut io.err)?;
            return Ok(1);
        }
    };

    let result = validate(&catalog);

    match output_format {
        OutputFormat::Text => {
            if result.is_valid {
                writeln!(io.out, "catalog is valid")?;
                writeln!(
                    io.out,
                    "conformance: {}",
                    conformance_level_name(result.conformance_level)
                )?;

                if !result.warnings.is_empty() {
                    write_diagnostics(&mut io.out, "warnings", &result.warnings)?;
                }

                Ok(0)
            } else {
                writeln!(io.err, "catalog is invalid")?;
                writeln!(
                    io.err,
                    "conformance: {}",
                    conformance_level_name(result.conformance_level)
                )?;
                write_diagnostics(&mut io.err, "errors", &result.errors)?;

                if !result.warnings.is_empty() {
                    write_diagnostics(&mut io.err, "warnings", &result.warnings)?;
                }

                Ok(1)
            }
        }
        OutputFormat::Json => {
            write_validation_result_json(
                &mut io.out,
                result.is_valid,
                Some(result.conformance_level),
                diagnostics_to_json(&result.errors),
                diagnostics_to_json(&result.warnings),
            )?;

            Ok(if result.is_valid { 0 } else { 1 })
        }
    }
}

fn format_command(path: &str, io: &mut Io) -> io::Result<i32> {
    let catalog = match read_catalog(path, io.stdin) {
        Ok(catalog) => catalog,
        Err(error) => {
            writeln!(io.err, "{}", parse_error_message(path, &error))?;
            return Ok(1);
        }
    };

    write_writer(&mut io.out, &catalog).map_err(io::Error::other)?;
    writeln!(io.out)?;

    Ok(0)
}

fn oci_pack_command(path: &str, io: &mut Io) -> io::Result<i32> {
    let catalog = match read_catalog(path, io.stdin) {
        Ok(catalog) => catalog,
        Err(error) => {
            writeln!(io.err, "{}", parse_error_message(path, &error))?;
            return Ok(1);
        }
    };

    let artifacts = match pack_catalog(&catalog) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            writeln!(io.err, "failed to pack '{path}' as OCI artifacts: {error}")?;
            return Ok(1);
        }
    };

    serde_json::to_writer_pretty(&mut io.out, &artifacts).map_err(io::Error::other)?;
    writeln!(io.out)?;

    Ok(0)
}

fn oci_unpack_command(path: &str, io: &mut Io) -> io::Result<i32> {
    let bytes = match read_bytes(path, io.stdin) {
        Ok(bytes) => bytes,
        Err(error) => {
            writeln!(io.err, "failed to read {}: {error}", input_label(path))?;
            return Ok(1);
        }
    };

    let artifacts: OciArtifactSet = match serde_json::from_slice(&bytes) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            writeln!(
                io.err,
                "failed to parse OCI artifact set from {}: {error}",
                input_label(path)
            )?;
            return Ok(1);
        }
    };

    let catalog = match unpack_catalog(&artifacts) {
        Ok(catalog) => catalog,
        Err(error) => {
            writeln!(
                io.err,
                "failed to unpack OCI artifact set from {}: {error}",
                input_label(path)
            )?;
            return Ok(1);
        }
    };

    write_writer(&mut io.out, &catalog).map_err(io::Error::other)?;
    writeln!(io.out)?;

    Ok(0)
}

/// Reads the catalog and packs it, attaching Cosign material when a key is given.
/// `None` means the error has been reported.
fn pack(publish: &PublishArgs, io: &mut Io) -> io::Result<Option<OciArtifactSet>> {
    let catalog = match read_catalog(&publish.path, io.stdin) {
        Ok(catalog) => catalog,
        Err(error) => {
            writeln!(io.err, "{}", parse_error_message(&publish.path, &error))?;
            return Ok(None);
        }
    };

    let mut artifacts = match pack_catalog(&catalog) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            writeln!(
                io.err,
                "failed to pack '{}' as OCI artifacts: {error}",
                publish.path
            )?;
            return Ok(None);
        }
    };

    if let Some(cosign_key) = &publish.cosign_key {
        let services = &publish.sigstore_services;
        if services.contains(&SigstoreService::Tlog) {
            writeln!(
                io.err,
                "note: each signature is recorded in the public Rekor transparency log; entries are permanent"
            )?;
        }
        // Temporary: remove this note once the layout stores Cosign bundles (#64, #66).
        if services.contains(&SigstoreService::Tsa) {
            writeln!(
                io.err,
                "note: timestamps are kept in the Cosign bundle, which the layout does not store yet"
            )?;
        }
        let signing_config = match &publish.signing_config {
            Some(path) => SigningConfig::File(path.clone()),
            None => SigningConfig::Generated(sigstore_signing_config(services)),
        };
        match attach_cosign_material(
            &catalog,
            &mut artifacts,
            cosign_key,
            publish.cosign_public_key.as_deref(),
            &signing_config,
        ) {
            Ok(attached) => {
                if attached > 0 {
                    writeln!(
                        io.out,
                        "attached Cosign verification artifacts for {attached} trust manifest(s)"
                    )?;
                }
            }
            Err(error) => {
                writeln!(
                    io.err,
                    "failed to attach Cosign verification artifacts: {error}"
                )?;
                return Ok(None);
            }
        }
    }

    Ok(Some(artifacts))
}

fn oci_export_layout_command(
    publish: &PublishArgs,
    layout_dir: &str,
    io: &mut Io,
) -> io::Result<i32> {
    let Some(artifacts) = pack(publish, io)? else {
        return Ok(1);
    };

    match export_layout(&artifacts, layout_dir, &publish.tag) {
        Ok(()) => {
            writeln!(
                io.out,
                "exported OCI image layout to '{layout_dir}' with tag '{}'",
                publish.tag
            )?;
            Ok(0)
        }
        Err(error) => {
            writeln!(
                io.err,
                "failed to export OCI layout to '{layout_dir}': {error}"
            )?;
            Ok(1)
        }
    }
}

fn oci_unpack_layout_command(
    layout_dir: &str,
    ref_name: Option<&str>,
    io: &mut Io,
) -> io::Result<i32> {
    let artifacts = match import_layout(layout_dir, ref_name) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            writeln!(
                io.err,
                "failed to import OCI layout from '{layout_dir}': {error}"
            )?;
            return Ok(1);
        }
    };

    let catalog = match unpack_catalog(&artifacts) {
        Ok(catalog) => catalog,
        Err(error) => {
            writeln!(
                io.err,
                "failed to unpack OCI layout from '{layout_dir}': {error}"
            )?;
            return Ok(1);
        }
    };

    write_writer(&mut io.out, &catalog).map_err(io::Error::other)?;
    writeln!(io.out)?;

    Ok(0)
}

/// Where `oci push` sends the artifacts, and how.
struct PushDestination<'a> {
    target: &'a str,
    to_oci_layout_path: Option<&'a str>,
    plain_http: bool,
    insecure: bool,
}

fn oci_push_command(
    publish: &PublishArgs,
    destination: &PushDestination,
    io: &mut Io,
) -> io::Result<i32> {
    let Some(artifacts) = pack(publish, io)? else {
        return Ok(1);
    };

    let layout_dir = temporary_layout_dir("ai-catalog-oras-layout");
    let result = push_artifacts_via_oras(&artifacts, &layout_dir, &publish.tag, destination, io);
    let _ = fs::remove_dir_all(&layout_dir);

    result
}

fn push_artifacts_via_oras(
    artifacts: &ai_catalog_oci::OciArtifactSet,
    layout_dir: &Path,
    tag: &str,
    destination: &PushDestination,
    io: &mut Io,
) -> io::Result<i32> {
    if let Err(error) = export_layout(artifacts, layout_dir, tag) {
        writeln!(
            io.err,
            "failed to export temporary OCI layout for '{}': {error}",
            destination.target
        )?;
        return Ok(1);
    }

    let args = build_oras_cp_args(
        layout_dir,
        tag,
        destination.target,
        destination.to_oci_layout_path,
        destination.plain_http,
        destination.insecure,
    );
    let output = match execute_oras(&args) {
        Ok(output) => output,
        Err(error) => {
            writeln!(io.err, "failed to invoke oras: {error}")?;
            return Ok(1);
        }
    };

    write_process_output(&mut io.out, &output.stdout)?;
    write_process_output(&mut io.err, &output.stderr)?;

    if output.status.success() {
        let repository = target_repository(destination.target);

        for digest in artifacts.referrers.keys() {
            let referrer_args = build_oras_cp_referrer_args(
                layout_dir,
                digest,
                repository,
                destination.to_oci_layout_path,
                destination.plain_http,
                destination.insecure,
            );
            let referrer_output = match execute_oras(&referrer_args) {
                Ok(output) => output,
                Err(error) => {
                    writeln!(io.err, "failed to invoke oras: {error}")?;
                    return Ok(1);
                }
            };

            write_process_output(&mut io.out, &referrer_output.stdout)?;
            write_process_output(&mut io.err, &referrer_output.stderr)?;

            if !referrer_output.status.success() {
                if referrer_output.stdout.is_empty() && referrer_output.stderr.is_empty() {
                    writeln!(io.err, "oras exited with status {}", referrer_output.status)?;
                }

                return Ok(referrer_output.status.code().unwrap_or(1));
            }
        }

        Ok(0)
    } else {
        if output.stdout.is_empty() && output.stderr.is_empty() {
            writeln!(io.err, "oras exited with status {}", output.status)?;
        }

        Ok(output.status.code().unwrap_or(1))
    }
}

fn verify_command(
    path: &str,
    json: bool,
    did_documents: &[String],
    offline: bool,
    strict: bool,
    io: &mut Io,
) -> io::Result<i32> {
    // The document is verified as served: signatures cover members the model does not keep.
    let bytes = match read_bytes(path, io.stdin) {
        Ok(bytes) => bytes,
        Err(error) => {
            writeln!(io.err, "failed to read {}: {error}", input_label(path))?;
            return Ok(1);
        }
    };

    let mut overrides = std::collections::BTreeMap::new();
    for spec in did_documents {
        let Some((did, file)) = spec.split_once('=') else {
            writeln!(io.err, "--did-document must be DID=PATH, found '{spec}'")?;
            return Ok(2);
        };
        let url = match verify::did_document_url(did) {
            Ok(url) => url,
            Err(error) => {
                writeln!(io.err, "--did-document: {error}")?;
                return Ok(2);
            }
        };
        match fs::read(file) {
            Ok(document) => {
                overrides.insert(url, document);
            }
            Err(error) => {
                writeln!(io.err, "failed to read DID document '{file}': {error}")?;
                return Ok(2);
            }
        }
    }

    let fetch = verify::SafeFetch { overrides, offline };
    let verification =
        match ai_catalog_validate::verify(&bytes, &ai_catalog_validate::Options::default(), &fetch)
        {
            Ok(verification) => verification,
            Err(error) => {
                writeln!(io.err, "failed to verify {}: {error}", input_label(path))?;
                return Ok(1);
            }
        };

    let (failed, unverifiable) = verify::summarize(&verification);
    if json {
        writeln!(
            io.out,
            "{}",
            serde_json::to_string_pretty(&verify::report_json(&verification))
                .map_err(io::Error::other)?
        )?;
    } else {
        if !verification.validation.is_valid {
            writeln!(io.err, "catalog is invalid")?;
            write_diagnostics(&mut io.err, "errors", &verification.validation.errors)?;
        }
        writeln!(io.out, "{}", verify::report_text(&verification))?;
    }

    Ok(if failed || (strict && unverifiable) {
        1
    } else {
        0
    })
}

fn read_catalog(path: &str, stdin: &mut dyn Read) -> ai_catalog::Result<ai_catalog::AiCatalog> {
    if path == "-" {
        parse_reader(stdin)
    } else {
        parse_file(path)
    }
}

fn read_bytes(path: &str, stdin: &mut dyn Read) -> io::Result<Vec<u8>> {
    if path == "-" {
        let mut bytes = Vec::new();
        stdin.read_to_end(&mut bytes)?;
        Ok(bytes)
    } else {
        fs::read(path)
    }
}

fn build_oras_cp_args(
    layout_dir: &Path,
    tag: &str,
    target: &str,
    to_oci_layout_path: Option<&str>,
    plain_http: bool,
    insecure: bool,
) -> Vec<String> {
    let mut args = vec![
        "cp".to_owned(),
        "-r".to_owned(),
        "--from-oci-layout".to_owned(),
    ];

    if let Some(layout_path) = to_oci_layout_path {
        args.push("--to-oci-layout-path".to_owned());
        args.push(layout_path.to_owned());
    }

    if plain_http {
        args.push("--to-plain-http".to_owned());
    }

    if insecure {
        args.push("--to-insecure".to_owned());
    }

    args.push(format!("{}:{tag}", layout_dir.display()));
    args.push(target.to_owned());
    args
}

fn build_oras_cp_referrer_args(
    layout_dir: &Path,
    digest: &str,
    target_repository: &str,
    to_oci_layout_path: Option<&str>,
    plain_http: bool,
    insecure: bool,
) -> Vec<String> {
    let mut args = vec![
        "cp".to_owned(),
        "-r".to_owned(),
        "--from-oci-layout".to_owned(),
    ];

    if let Some(layout_path) = to_oci_layout_path {
        args.push("--to-oci-layout-path".to_owned());
        args.push(layout_path.to_owned());
    }

    if plain_http {
        args.push("--to-plain-http".to_owned());
    }

    if insecure {
        args.push("--to-insecure".to_owned());
    }

    args.push(format!("{}@{digest}", layout_dir.display()));
    args.push(target_repository.to_owned());
    args
}

fn target_repository(target: &str) -> &str {
    if let Some((repository, _)) = target.split_once('@') {
        return repository;
    }

    let last_slash = target.rfind('/');
    let last_colon = target.rfind(':');

    match last_colon {
        Some(index) if last_slash.is_none_or(|slash| index > slash) => &target[..index],
        _ => target,
    }
}

fn execute_oras(args: &[String]) -> io::Result<std::process::Output> {
    let oras_bin = std::env::var(ORAS_BIN_ENV).unwrap_or_else(|_| "oras".to_owned());

    std::process::Command::new(oras_bin).args(args).output()
}

fn attach_cosign_material(
    catalog: &AiCatalog,
    artifacts: &mut OciArtifactSet,
    cosign_key: &str,
    cosign_public_key: Option<&str>,
    signing_config: &SigningConfig,
) -> io::Result<usize> {
    let targets = catalog
        .entries
        .iter()
        .zip(artifacts.index.manifests.iter())
        .flat_map(|(entry, descriptor)| {
            entry
                .trust_manifests
                .iter()
                .map(|trust_manifest| (descriptor.digest.clone(), trust_manifest.clone()))
        })
        .collect::<Vec<_>>();

    if targets.is_empty() {
        return Ok(0);
    }

    let public_key = load_cosign_public_key(cosign_key, cosign_public_key)?;

    for (subject_digest, trust_manifest) in targets {
        let canonical_manifest =
            canonicalize_trust_manifest(&trust_manifest).map_err(io::Error::other)?;
        let canonical_bytes = canonical_manifest.into_bytes();
        let payload_digest = format!(
            "sha256:{}",
            digest_hex(Sha256::digest(&canonical_bytes).as_slice())
        );
        let signature = sign_blob_with_cosign(&canonical_bytes, cosign_key, signing_config)?;

        attach_cosign_verification_artifacts(
            artifacts,
            &subject_digest,
            &trust_manifest.contributor,
            &payload_digest,
            &signature,
            &public_key,
        )
        .map_err(io::Error::other)?;
    }

    Ok(catalog
        .entries
        .iter()
        .map(|entry| entry.trust_manifests.len())
        .sum())
}

fn load_cosign_public_key(
    cosign_key: &str,
    cosign_public_key: Option<&str>,
) -> io::Result<Vec<u8>> {
    if let Some(path) = cosign_public_key {
        return fs::read(path);
    }

    let args = build_cosign_public_key_args(cosign_key);
    let output = execute_cosign(&args)?;

    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(command_failure("cosign public-key", &output))
    }
}

/// The Sigstore signing config `cosign sign-blob` signs with.
enum SigningConfig {
    /// Generated from `--sigstore-services`.
    Generated(String),
    /// Given with `--signing-config`.
    File(String),
}

// Public Sigstore endpoints, as listed in the `signing_config.v0.2.json` target of the
// Sigstore TUF repository (github.com/sigstore/root-signing).
const SIGSTORE_REKOR_URL: &str = "https://rekor.sigstore.dev";
const SIGSTORE_REKOR_VALID_FROM: &str = "2021-01-12T11:53:27.000Z";
const SIGSTORE_TSA_URL: &str = "https://timestamp.sigstore.dev/api/v1/timestamp";
const SIGSTORE_TSA_VALID_FROM: &str = "2025-07-04T00:00:00Z";

/// A signing config that uses only the selected public Sigstore services. Certificate
/// authorities are never listed, since signing always uses `--key`.
fn sigstore_signing_config(services: &[SigstoreService]) -> String {
    let service = |url: &str, valid_from: &str| {
        serde_json::json!({
            "url": url,
            "majorApiVersion": 1,
            "validFor": { "start": valid_from },
            "operator": "sigstore.dev",
        })
    };
    let rekor = if services.contains(&SigstoreService::Tlog) {
        vec![service(SIGSTORE_REKOR_URL, SIGSTORE_REKOR_VALID_FROM)]
    } else {
        Vec::new()
    };
    let tsa = if services.contains(&SigstoreService::Tsa) {
        vec![service(SIGSTORE_TSA_URL, SIGSTORE_TSA_VALID_FROM)]
    } else {
        Vec::new()
    };

    serde_json::json!({
        "mediaType": "application/vnd.dev.sigstore.signingconfig.v0.2+json",
        "caUrls": [],
        "oidcUrls": [],
        "rekorTlogUrls": rekor,
        "tsaUrls": tsa,
        "rekorTlogConfig": { "selector": "ANY" },
        "tsaConfig": { "selector": "ANY" },
    })
    .to_string()
}

fn sign_blob_with_cosign(
    payload: &[u8],
    cosign_key: &str,
    signing_config: &SigningConfig,
) -> io::Result<Vec<u8>> {
    let work_dir = temporary_layout_dir("ai-catalog-cosign");
    let payload_path = work_dir.join("trust-manifest.json");
    let bundle_path = work_dir.join("trust-manifest.bundle.json");
    let generated_config_path = work_dir.join("signing-config.json");

    fs::create_dir_all(&work_dir)?;
    fs::write(&payload_path, payload)?;
    let signing_config_path = match signing_config {
        SigningConfig::Generated(config) => {
            fs::write(&generated_config_path, config)?;
            generated_config_path.clone()
        }
        SigningConfig::File(path) => PathBuf::from(path),
    };

    let args = build_cosign_sign_blob_args(
        cosign_key,
        &payload_path,
        &bundle_path,
        Some(&signing_config_path),
    );
    let output = execute_cosign(&args)?;
    let result = if output.status.success() {
        let bundle_bytes = fs::read(&bundle_path)?;
        let bundle: CosignBlobBundle =
            serde_json::from_slice(&bundle_bytes).map_err(io::Error::other)?;

        Ok(bundle.message_signature.signature.into_bytes())
    } else {
        Err(command_failure("cosign sign-blob", &output))
    };

    let _ = fs::remove_dir_all(&work_dir);
    result
}

fn build_cosign_sign_blob_args(
    cosign_key: &str,
    payload_path: &Path,
    bundle_path: &Path,
    signing_config: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "sign-blob".to_owned(),
        "--yes".to_owned(),
        "--key".to_owned(),
        cosign_key.to_owned(),
        "--bundle".to_owned(),
        bundle_path.display().to_string(),
    ];
    if let Some(path) = signing_config {
        args.push("--signing-config".to_owned());
        args.push(path.display().to_string());
    }
    args.push(payload_path.display().to_string());
    args
}

fn build_cosign_public_key_args(cosign_key: &str) -> Vec<String> {
    vec![
        "public-key".to_owned(),
        "--key".to_owned(),
        cosign_key.to_owned(),
    ]
}

fn execute_cosign(args: &[String]) -> io::Result<std::process::Output> {
    let cosign_bin = std::env::var(COSIGN_BIN_ENV).unwrap_or_else(|_| "cosign".to_owned());

    std::process::Command::new(cosign_bin).args(args).output()
}

fn command_failure(command: &str, output: &std::process::Output) -> io::Error {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let details = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exited with status {}", output.status)
    };

    io::Error::other(format!("{command} failed: {details}"))
}

fn digest_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        use std::fmt::Write as _;

        let _ = write!(&mut hex, "{byte:02x}");
    }

    hex
}

fn write_process_output(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }

    writer.write_all(bytes)?;

    if !bytes.ends_with(b"\n") {
        writeln!(writer)?;
    }

    Ok(())
}

fn temporary_layout_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();

    std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
}

fn write_diagnostics(
    writer: &mut impl Write,
    label: &str,
    diagnostics: &[Diagnostic],
) -> io::Result<()> {
    writeln!(writer, "{label}:")?;

    for diagnostic in diagnostics {
        writeln!(writer, "  - {}: {}", diagnostic.path, diagnostic.message)?;
    }

    Ok(())
}

fn write_parse_error(
    output_format: OutputFormat,
    path: &str,
    error: ai_catalog::Error,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<()> {
    match output_format {
        OutputFormat::Text => writeln!(stderr, "{}", parse_error_message(path, &error)),
        OutputFormat::Json => write_validation_result_json(
            stdout,
            false,
            None,
            vec![json!({
                "code": null,
                "path": path,
                "message": format!("failed to parse input: {error}"),
            })],
            Vec::new(),
        ),
    }
}

fn write_validation_result_json(
    stdout: &mut impl Write,
    is_valid: bool,
    conformance_level: Option<ConformanceLevel>,
    errors: Vec<serde_json::Value>,
    warnings: Vec<serde_json::Value>,
) -> io::Result<()> {
    let payload = json!({
        "valid": is_valid,
        "conformanceLevel": conformance_level.map(conformance_level_name),
        "errors": errors,
        "warnings": warnings,
    });

    serde_json::to_writer_pretty(&mut *stdout, &payload).map_err(io::Error::other)?;
    writeln!(stdout)
}

fn diagnostics_to_json(diagnostics: &[Diagnostic]) -> Vec<serde_json::Value> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            json!({
                "code": diagnostic.code.as_str(),
                "path": diagnostic.path,
                "message": diagnostic.message,
            })
        })
        .collect()
}

fn parse_error_message(path: &str, error: &ai_catalog::Error) -> String {
    if path == "-" {
        format!("failed to parse stdin: {error}")
    } else {
        format!("failed to parse '{path}': {error}")
    }
}

fn input_label(path: &str) -> String {
    if path == "-" {
        "stdin".to_owned()
    } else {
        format!("'{path}'")
    }
}

fn conformance_level_name(level: ConformanceLevel) -> &'static str {
    match level {
        ConformanceLevel::Minimal => "minimal",
        ConformanceLevel::Discoverable => "discoverable",
        ConformanceLevel::Trusted => "trusted",
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use ai_catalog_oci::{
        COSIGN_PUBLIC_KEY_ARTIFACT_TYPE, COSIGN_SIGNATURE_ARTIFACT_TYPE,
        TRUST_MANIFEST_ARTIFACT_TYPE, import_layout,
    };

    use serde_json::Value;
    use std::env;
    use std::fs;
    use std::io::Write as _;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn prints_help_without_arguments() {
        let (exit_code, stdout, stderr) = run_command(["ai-catalog"], "");

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("Usage:"));
        for command in [
            "validate", "format", "oci", "catalog", "search", "show", "pull",
        ] {
            assert!(stdout.contains(command), "help should list {command}");
        }
        assert!(stderr.is_empty());
    }

    #[test]
    fn prints_version() {
        let (exit_code, stdout, stderr) = run_command(["ai-catalog", "--version"], "");

        assert_eq!(exit_code, 0);
        assert_eq!(
            stdout,
            format!("ai-catalog {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn validates_canonical_fixture() {
        let fixture = canonical_fixture_path();
        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "validate",
                fixture.to_str().expect("path should be utf-8"),
            ],
            "",
        );

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("catalog is valid"));
        assert!(stdout.contains("conformance: discoverable"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn formats_canonical_fixture() {
        let fixture = canonical_fixture_path();
        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "format",
                fixture.to_str().expect("path should be utf-8"),
            ],
            "",
        );

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("\"entries\": ["));
        assert!(stdout.ends_with('\n'));
        assert!(stderr.is_empty());
    }

    #[test]
    fn validates_canonical_fixture_as_json() {
        let fixture = canonical_fixture_path();
        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "validate",
                "--json",
                fixture.to_str().expect("path should be utf-8"),
            ],
            "",
        );
        let payload: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

        assert_eq!(exit_code, 0);
        assert_eq!(payload["valid"], true);
        assert_eq!(payload["conformanceLevel"], "discoverable");
        assert_eq!(payload["errors"], Value::Array(vec![]));
        assert!(stderr.is_empty());
    }

    #[test]
    fn validates_from_stdin() {
        let fixture = canonical_fixture_text();
        let (exit_code, stdout, stderr) =
            run_command(["ai-catalog-cli", "validate", "-"], &fixture);

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("catalog is valid"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn formats_from_stdin() {
        let fixture = canonical_fixture_text();
        let (exit_code, stdout, stderr) = run_command(["ai-catalog-cli", "format", "-"], &fixture);

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("\"specVersion\": \"1.0\""));
        assert!(stdout.ends_with('\n'));
        assert!(stderr.is_empty());
    }

    #[test]
    fn reports_validation_errors() {
        let invalid_fixture = r#"{
    "specVersion": "1.0",
    "entries": [
        {
            "identifier": "urn:example:model",
            "displayName": "Example Model",
            "type": "application/json",
            "url": "https://example.com/model.json",
            "data": {}
        }
    ]
}"#;
        let (exit_code, stdout, stderr) =
            run_command(["ai-catalog-cli", "validate", "-"], invalid_fixture);

        assert_eq!(exit_code, 1);
        assert!(stdout.is_empty());
        assert!(stderr.contains("catalog is invalid"));
        assert!(stderr.contains("exactly one of 'url' or 'data', found both"));
    }

    #[test]
    fn reports_validation_errors_as_json() {
        let invalid_fixture = r#"{
    "specVersion": "1.0",
    "entries": [
        {
            "identifier": "urn:example:model",
            "displayName": "Example Model",
            "type": "application/json",
            "url": "https://example.com/model.json",
            "data": {}
        }
    ]
}"#;
        let (exit_code, stdout, stderr) = run_command(
            ["ai-catalog-cli", "validate", "--json", "-"],
            invalid_fixture,
        );
        let payload: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

        assert_eq!(exit_code, 1);
        assert_eq!(payload["valid"], false);
        assert_eq!(payload["conformanceLevel"], "minimal");
        assert!(
            payload["errors"]
                .as_array()
                .expect("errors should be an array")
                .iter()
                .any(|diagnostic| diagnostic["code"] == "entry-artifact-source"
                    && diagnostic["message"]
                        .as_str()
                        .expect("message should be a string")
                        .contains("exactly one of 'url' or 'data', found both"))
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn reports_parse_errors() {
        let (exit_code, stdout, stderr) =
            run_command(["ai-catalog-cli", "validate", "-"], "not json");

        assert_eq!(exit_code, 1);
        assert!(stdout.is_empty());
        assert!(stderr.contains("failed to parse stdin"));
    }

    #[test]
    fn reports_parse_errors_as_json() {
        let (exit_code, stdout, stderr) =
            run_command(["ai-catalog-cli", "validate", "--json", "-"], "not json");
        let payload: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

        assert_eq!(exit_code, 1);
        assert_eq!(payload["valid"], false);
        assert_eq!(payload["conformanceLevel"], Value::Null);
        assert!(
            payload["errors"]
                .as_array()
                .expect("errors should be an array")
                .iter()
                .any(|diagnostic| diagnostic["path"] == "-" && diagnostic["code"].is_null())
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn rejects_invalid_usage() {
        for args in [
            vec!["ai-catalog", "validate", "--yaml", "-"],
            vec!["ai-catalog", "format"],
            vec!["ai-catalog", "bogus"],
        ] {
            let (exit_code, stdout, stderr) = run_command(args.clone(), "");

            assert_eq!(exit_code, 2, "{args:?}");
            assert!(stdout.is_empty(), "{args:?}");
            assert!(stderr.contains("Usage:"), "{args:?}");
        }
    }

    #[test]
    fn oci_pack_outputs_artifact_set_json() {
        let fixture = canonical_fixture_text();
        let (exit_code, stdout, stderr) =
            run_command(["ai-catalog-cli", "oci", "pack", "-"], &fixture);
        let payload: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

        assert_eq!(exit_code, 0);
        assert_eq!(
            payload["index"]["artifactType"],
            "application/ai-catalog+json"
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn oci_unpack_round_trips_packed_catalog() {
        let fixture = canonical_fixture_text();
        let (_, packed_stdout, packed_stderr) =
            run_command(["ai-catalog-cli", "oci", "pack", "-"], &fixture);

        assert!(packed_stderr.is_empty());

        let (exit_code, stdout, stderr) =
            run_command(["ai-catalog-cli", "oci", "unpack", "-"], &packed_stdout);

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("\"specVersion\": \"1.0\""));
        assert!(stdout.contains("\"entries\": ["));
        assert!(stderr.is_empty());
    }

    #[test]
    fn oci_export_layout_writes_standard_layout_directory() {
        let fixture = canonical_fixture_text();
        let layout_dir = unique_temp_dir("ai-catalog-cli-layout");
        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "oci",
                "export-layout",
                "--tag",
                "demo",
                "-",
                layout_dir.to_str().expect("path should be utf-8"),
            ],
            &fixture,
        );

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("exported OCI image layout"));
        assert!(layout_dir.join("oci-layout").exists());
        assert!(layout_dir.join("index.json").exists());
        assert!(stderr.is_empty());

        fs::remove_dir_all(layout_dir).expect("layout dir should be removable");
    }

    #[test]
    fn rejects_cosign_public_key_without_cosign_key() {
        let layout_dir = unique_temp_dir("ai-catalog-cli-layout-cosign-option");
        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "oci",
                "export-layout",
                "--cosign-public-key",
                "cosign.pub",
                "-",
                layout_dir.to_str().expect("path should be utf-8"),
            ],
            &canonical_fixture_text(),
        );

        assert_eq!(exit_code, 2);
        assert!(stdout.is_empty());
        assert!(stderr.contains("--cosign-key"));
    }

    #[test]
    #[cfg(unix)]
    fn oci_export_layout_with_cosign_writes_signature_and_key_referrers() {
        let fixture = r#"{
    "specVersion": "1.0",
    "entries": [
        {
            "identifier": "urn:example:model",
            "displayName": "Example Model",
            "type": "application/json",
            "data": { "name": "example" },
            "trustManifests": [{
                "contributor": "urn:example:model",
                "trustSchema": {
                    "identifier": "urn:trust:example",
                    "version": "1.0"
                }
            }]
        }
    ]
}"#;
        let layout_dir = unique_temp_dir("ai-catalog-cli-layout-cosign");
        let tool_dir = unique_temp_dir("ai-catalog-cli-fake-cosign");

        fs::create_dir_all(&tool_dir).expect("tool dir should exist");
        let script_path = write_fake_cosign_script(&tool_dir);

        let (exit_code, stdout, stderr) =
            with_env_var(super::COSIGN_BIN_ENV, script_path.as_os_str(), || {
                run_command(
                    [
                        "ai-catalog-cli",
                        "oci",
                        "export-layout",
                        "--cosign-key",
                        "fake.key",
                        "--tag",
                        "demo",
                        "-",
                        layout_dir.to_str().expect("path should be utf-8"),
                    ],
                    fixture,
                )
            });

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("attached Cosign verification artifacts for 1 trust manifest(s)"));
        assert!(stderr.is_empty());

        let imported = import_layout(&layout_dir, Some("demo")).expect("layout should import");
        let entry_digest = imported.index.manifests[0].digest.clone();
        let referrer_types = imported.referrers[&entry_digest]
            .iter()
            .filter_map(|manifest| manifest.artifact_type.as_deref())
            .collect::<Vec<_>>();

        assert!(referrer_types.contains(&TRUST_MANIFEST_ARTIFACT_TYPE));
        assert!(referrer_types.contains(&COSIGN_SIGNATURE_ARTIFACT_TYPE));
        assert!(referrer_types.contains(&COSIGN_PUBLIC_KEY_ARTIFACT_TYPE));

        fs::remove_dir_all(layout_dir).expect("layout dir should be removable");
        fs::remove_dir_all(tool_dir).expect("tool dir should be removable");
    }

    #[test]
    fn oci_unpack_layout_round_trips_exported_layout() {
        let fixture = canonical_fixture_text();
        let layout_dir = unique_temp_dir("ai-catalog-cli-unpack-layout");
        let (_, export_stdout, export_stderr) = run_command(
            [
                "ai-catalog-cli",
                "oci",
                "export-layout",
                "--tag",
                "demo",
                "-",
                layout_dir.to_str().expect("path should be utf-8"),
            ],
            &fixture,
        );

        assert!(export_stdout.contains("exported OCI image layout"));
        assert!(export_stderr.is_empty());

        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "oci",
                "unpack-layout",
                "--ref-name",
                "demo",
                layout_dir.to_str().expect("path should be utf-8"),
            ],
            "",
        );

        assert_eq!(exit_code, 0);
        assert!(stdout.contains("\"specVersion\": \"1.0\""));
        assert!(stdout.contains("\"entries\": ["));
        assert!(stderr.is_empty());

        fs::remove_dir_all(layout_dir).expect("layout dir should be removable");
    }

    #[test]
    fn builds_oras_copy_arguments() {
        let args = super::build_oras_cp_args(
            Path::new("/tmp/source-layout"),
            "demo",
            "example.com/catalog:demo",
            Some("/tmp/dest-layout"),
            true,
            true,
        );

        assert_eq!(
            args,
            vec![
                "cp",
                "-r",
                "--from-oci-layout",
                "--to-oci-layout-path",
                "/tmp/dest-layout",
                "--to-plain-http",
                "--to-insecure",
                "/tmp/source-layout:demo",
                "example.com/catalog:demo",
            ]
        );
    }

    #[test]
    fn builds_oras_copy_arguments_for_entry_referrers() {
        let args = super::build_oras_cp_referrer_args(
            Path::new("/tmp/source-layout"),
            "sha256:deadbeef",
            "example.com/catalog",
            Some("/tmp/dest-layout"),
            true,
            false,
        );

        assert_eq!(
            args,
            vec![
                "cp",
                "-r",
                "--from-oci-layout",
                "--to-oci-layout-path",
                "/tmp/dest-layout",
                "--to-plain-http",
                "/tmp/source-layout@sha256:deadbeef",
                "example.com/catalog",
            ]
        );
    }

    #[test]
    fn builds_cosign_sign_blob_arguments() {
        let args = super::build_cosign_sign_blob_args(
            "cosign.key",
            Path::new("/tmp/trust-manifest.json"),
            Path::new("/tmp/trust-manifest.sig"),
            None,
        );

        assert_eq!(
            args,
            vec![
                "sign-blob",
                "--yes",
                "--key",
                "cosign.key",
                "--bundle",
                "/tmp/trust-manifest.sig",
                "/tmp/trust-manifest.json",
            ]
        );
    }

    #[test]
    fn builds_offline_cosign_sign_blob_arguments() {
        let args = super::build_cosign_sign_blob_args(
            "cosign.key",
            Path::new("/tmp/trust-manifest.json"),
            Path::new("/tmp/trust-manifest.sig"),
            Some(Path::new("/tmp/signing-config.json")),
        );

        assert_eq!(
            args,
            vec![
                "sign-blob",
                "--yes",
                "--key",
                "cosign.key",
                "--bundle",
                "/tmp/trust-manifest.sig",
                "--signing-config",
                "/tmp/signing-config.json",
                "/tmp/trust-manifest.json",
            ]
        );
    }

    fn service_urls(services: &[super::SigstoreService]) -> [Vec<String>; 4] {
        let config: serde_json::Value =
            serde_json::from_str(&super::sigstore_signing_config(services))
                .expect("config should be JSON");
        ["caUrls", "oidcUrls", "rekorTlogUrls", "tsaUrls"].map(|key| {
            config[key]
                .as_array()
                .expect("list")
                .iter()
                .map(|service| service["url"].as_str().expect("url").to_owned())
                .collect()
        })
    }

    #[test]
    fn signing_config_without_services_lists_none() {
        assert_eq!(
            service_urls(&[]),
            [vec![], vec![], vec![], vec![]] as [Vec<String>; 4]
        );
    }

    #[test]
    fn signing_config_lists_only_selected_services() {
        use super::SigstoreService::{Tlog, Tsa};

        let [ca, oidc, rekor, tsa] = service_urls(&[Tlog]);
        assert!(ca.is_empty() && oidc.is_empty() && tsa.is_empty());
        assert_eq!(rekor, [super::SIGSTORE_REKOR_URL]);

        let [ca, oidc, rekor, tsa] = service_urls(&[Tsa]);
        assert!(ca.is_empty() && oidc.is_empty() && rekor.is_empty());
        assert_eq!(tsa, [super::SIGSTORE_TSA_URL]);

        let [ca, oidc, rekor, tsa] = service_urls(&[Tlog, Tsa]);
        assert!(ca.is_empty() && oidc.is_empty());
        assert_eq!(rekor, [super::SIGSTORE_REKOR_URL]);
        assert_eq!(tsa, [super::SIGSTORE_TSA_URL]);
    }

    #[test]
    fn rejects_sigstore_services_with_signing_config() {
        let layout_dir = unique_temp_dir("ai-catalog-cli-layout-sigstore-options");
        let (exit_code, stdout, stderr) = run_command(
            [
                "ai-catalog-cli",
                "oci",
                "export-layout",
                "--cosign-key",
                "cosign.key",
                "--sigstore-services",
                "tlog,tsa",
                "--signing-config",
                "config.json",
                "-",
                layout_dir.to_str().expect("path should be utf-8"),
            ],
            &canonical_fixture_text(),
        );

        assert_eq!(exit_code, 2);
        assert!(stdout.is_empty());
        assert!(stderr.contains("--signing-config"));
    }

    #[test]
    fn rejects_unknown_sigstore_services() {
        let layout_dir = unique_temp_dir("ai-catalog-cli-layout-sigstore-unknown");
        let (exit_code, _, stderr) = run_command(
            [
                "ai-catalog-cli",
                "oci",
                "export-layout",
                "--cosign-key",
                "cosign.key",
                "--sigstore-services",
                "fulcio",
                "-",
                layout_dir.to_str().expect("path should be utf-8"),
            ],
            &canonical_fixture_text(),
        );

        assert_eq!(exit_code, 2);
        assert!(stderr.contains("tlog") && stderr.contains("tsa"));
    }

    #[test]
    fn builds_cosign_public_key_arguments() {
        let args = super::build_cosign_public_key_args("cosign.key");

        assert_eq!(args, vec!["public-key", "--key", "cosign.key"]);
    }

    #[test]
    fn strips_tags_from_target_repositories() {
        assert_eq!(
            super::target_repository("example.com/catalog:demo"),
            "example.com/catalog"
        );
        assert_eq!(
            super::target_repository("localhost:5000/example/catalog:demo"),
            "localhost:5000/example/catalog"
        );
        assert_eq!(
            super::target_repository("example.com/catalog@sha256:deadbeef"),
            "example.com/catalog"
        );
        assert_eq!(
            super::target_repository("example.com/catalog"),
            "example.com/catalog"
        );
    }

    fn canonical_fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/spec-example.json")
    }

    fn canonical_fixture_text() -> String {
        fs::read_to_string(canonical_fixture_path()).expect("fixture should be readable")
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();

        std::env::temp_dir().join(format!("{prefix}-{unique}"))
    }

    fn run_command<'a>(
        args: impl IntoIterator<Item = &'a str>,
        stdin_text: &str,
    ) -> (i32, String, String) {
        let mut stdin = stdin_text.as_bytes();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit_code =
            run(args, &mut stdin, &mut stdout, &mut stderr).expect("run should succeed");

        (
            exit_code,
            String::from_utf8(stdout).expect("stdout should be utf-8"),
            String::from_utf8(stderr).expect("stderr should be utf-8"),
        )
    }

    fn with_env_var<T>(key: &str, value: &std::ffi::OsStr, f: impl FnOnce() -> T) -> T {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock should be available");
        let previous = env::var_os(key);
        unsafe {
            env::set_var(key, value);
        }
        let result = f();

        if let Some(previous) = previous {
            unsafe {
                env::set_var(key, previous);
            }
        } else {
            unsafe {
                env::remove_var(key);
            }
        }

        result
    }

    #[cfg(unix)]
    const VERIFY_SIGNER: &str = "did:web:apexretail.com";
    const VERIFY_KID: &str = "did:web:apexretail.com#release-key";

    /// A catalog whose single entry carries inline data and a publisher manifest
    /// signed under did-web-v1, plus the DID document for the signing key.
    fn signed_verify_fixture(edit: impl FnOnce(&mut serde_json::Value)) -> (String, String) {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use p256::ecdsa::signature::Signer as _;
        use p256::ecdsa::{Signature, SigningKey};
        use sha2::Digest as _;

        let key = SigningKey::from_slice(&[7u8; 32]).expect("valid scalar");
        let data = serde_json::json!({ "name": "refund-bot", "version": "1.2.0" });
        let jcs = ai_catalog_trust::canonicalize(data.to_string()).expect("canonicalizes");
        let digest = format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(jcs.as_bytes()))
        );
        let subject = serde_json::json!({
            "identifier": "urn:air:apexretail.com:claims:refund-bot",
            "version": "1.2.0",
            "type": "application/a2a-agent-card+json",
            "digest": digest,
        });
        let mut manifest = serde_json::json!({
            "contributor": VERIFY_SIGNER,
            "subject": subject,
            "attestations": [{ "type": "SOC2-Type2", "uri": "https://example.com/soc2.pdf" }],
            "signature": {
                "signer": VERIFY_SIGNER,
                "profile": "did-web-v1",
                "issuedAt": "2026-01-01T00:00:00Z",
                "expiresAt": "2099-01-01T00:00:00Z",
            },
        });
        let header = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "alg": "ES256",
                "kid": VERIFY_KID,
                "typ": "ai-catalog-trust-manifest+jws",
            })
            .to_string(),
        );
        let payload = ai_catalog_trust::canonicalize_for_signature(manifest.to_string())
            .expect("canonicalizes");
        let signing_input = format!("{header}.{}", URL_SAFE_NO_PAD.encode(payload));
        let signature: Signature = key.sign(signing_input.as_bytes());
        manifest["signature"]["jws"] = serde_json::json!(format!(
            "{header}..{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ));

        let mut catalog = serde_json::json!({
            "specVersion": "1.0",
            "host": { "displayName": "Apex Retail", "identifier": VERIFY_SIGNER },
            "entries": [{
                "identifier": "urn:air:apexretail.com:claims:refund-bot",
                "version": "1.2.0",
                "type": "application/a2a-agent-card+json",
                "data": data,
                "digest": digest,
                "trustManifests": [manifest],
            }],
        });
        edit(&mut catalog);

        let point = key.verifying_key().to_sec1_point(false);
        let (x, y) = point.as_bytes()[1..].split_at(32);
        let did_document = serde_json::json!({
            "id": VERIFY_SIGNER,
            "verificationMethod": [{
                "id": VERIFY_KID,
                "type": "JsonWebKey",
                "controller": VERIFY_SIGNER,
                "publicKeyJwk": {
                    "kty": "EC",
                    "crv": "P-256",
                    "x": URL_SAFE_NO_PAD.encode(x),
                    "y": URL_SAFE_NO_PAD.encode(y),
                },
            }],
            "assertionMethod": [VERIFY_KID],
        });
        (catalog.to_string(), did_document.to_string())
    }

    fn run_verify(
        catalog: &str,
        did_document: Option<&str>,
        extra: &[&str],
    ) -> (i32, String, String) {
        // A fresh directory per call: tests run in parallel, and time-based names can collide.
        let dir = tempfile::tempdir().expect("temp dir");
        let document_path = dir.path().join("did.json");
        let mut args = vec![
            "ai-catalog-cli".to_owned(),
            "verify".to_owned(),
            "--offline".to_owned(),
        ];
        if let Some(document) = did_document {
            fs::write(&document_path, document).expect("write did document");
            args.push("--did-document".to_owned());
            args.push(format!("{VERIFY_SIGNER}={}", document_path.display()));
        }
        args.extend(extra.iter().map(|arg| (*arg).to_owned()));
        args.push("-".to_owned());
        run_command(args.iter().map(String::as_str), catalog)
    }

    #[test]
    fn verify_accepts_a_valid_publisher_signature() {
        let (catalog, document) = signed_verify_fixture(|_| {});
        let (exit_code, stdout, stderr) = run_verify(&catalog, Some(&document), &[]);

        assert_eq!(exit_code, 0, "stdout: {stdout}\nstderr: {stderr}");
        assert!(stdout.contains("conformance: trusted"), "{stdout}");
        assert!(stdout.contains("[ok] signature"), "{stdout}");
        assert!(stdout.contains("[ok] artifact digest"), "{stdout}");
    }

    #[test]
    fn verify_fails_on_a_tampered_claim() {
        let (catalog, document) = signed_verify_fixture(|catalog| {
            catalog["entries"][0]["trustManifests"][0]["attestations"][0]["type"] =
                serde_json::json!("ISO27001");
        });
        let (exit_code, stdout, _) = run_verify(&catalog, Some(&document), &[]);

        assert_eq!(exit_code, 1);
        assert!(stdout.contains("[FAIL] signature"), "{stdout}");
    }

    #[test]
    fn verify_reports_unverifiable_offline_and_fails_with_strict() {
        let (catalog, _) = signed_verify_fixture(|_| {});

        let (exit_code, stdout, _) = run_verify(&catalog, None, &[]);
        assert_eq!(exit_code, 0);
        assert!(stdout.contains("[??] signature"), "{stdout}");

        let (exit_code, _, _) = run_verify(&catalog, None, &["--strict"]);
        assert_eq!(exit_code, 1);
    }

    #[test]
    fn verify_prints_a_json_report() {
        let (catalog, document) = signed_verify_fixture(|_| {});
        let (exit_code, stdout, _) = run_verify(&catalog, Some(&document), &["--json"]);
        let report: serde_json::Value = serde_json::from_str(&stdout).expect("json report");

        assert_eq!(exit_code, 0);
        assert_eq!(report["conformance"], "trusted");
        assert_eq!(
            report["entries"][0]["manifests"][0]["signature"]["result"],
            "verified"
        );
        assert_eq!(report["entries"][0]["manifests"][0]["publisher"], true);
    }

    #[test]
    fn verify_rejects_a_non_root_did_document_override() {
        let (catalog, document) = signed_verify_fixture(|_| {});
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("did.json");
        fs::write(&path, document).expect("write");
        let spec = format!("did:web:example.com:users:alice={}", path.display());
        let (exit_code, _, stderr) = run_command(
            ["ai-catalog-cli", "verify", "--did-document", &spec, "-"],
            &catalog,
        );

        assert_eq!(exit_code, 2);
        assert!(stderr.contains("root did:web"), "{stderr}");
    }

    fn write_fake_cosign_script(dir: &Path) -> PathBuf {
        let script_path = dir.join("fake-cosign.sh");
        let mut file = fs::File::create(&script_path).expect("script should be creatable");

        file.write_all(
                        br#"#!/bin/sh
set -eu
cmd="$1"
shift
case "$cmd" in
    public-key)
        echo '-----BEGIN PUBLIC KEY-----'
        echo 'RkFLRUNPU0lHTlBVQkxJQ0tFWQ=='
        echo '-----END PUBLIC KEY-----'
        ;;
    sign-blob)
        bundle=''
        while [ $# -gt 0 ]; do
            case "$1" in
                --bundle)
                    bundle="$2"
                    shift 2
                    ;;
                --key)
                    shift 2
                    ;;
                --yes)
                    shift 1
                    ;;
                *)
                    shift 1
                    ;;
            esac
        done
        printf '{"mediaType":"application/vnd.dev.sigstore.bundle.v0.3+json","messageSignature":{"signature":"fake-cosign-signature"}}' > "$bundle"
        ;;
    *)
        echo "unexpected cosign command: $cmd" >&2
        exit 1
        ;;
esac"#,
                )
                .expect("script should be writable");

        let mut permissions = fs::metadata(&script_path)
            .expect("script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("script should be executable");

        script_path
    }
}
