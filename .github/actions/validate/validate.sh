#!/usr/bin/env bash
# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Validate a catalog with the installed ai-catalog CLI and report the result
# as GitHub annotations, a step summary, and step outputs.
#
# Env: CATALOG_PATH or CATALOG_URL (exactly one), FAIL_ON_INVALID (true|false),
# AI_CATALOG_PATH (optional, defaults to ai-catalog on PATH), RUNNER_TEMP,
# GITHUB_OUTPUT, GITHUB_STEP_SUMMARY (the last two are optional).
set -euo pipefail

path="${CATALOG_PATH:-}"
url="${CATALOG_URL:-}"
tmp="${RUNNER_TEMP:-$(mktemp -d)}"
out="${GITHUB_OUTPUT:-/dev/null}"
summary="${GITHUB_STEP_SUMMARY:-/dev/null}"

if { [ -n "$path" ] && [ -n "$url" ]; } || { [ -z "$path" ] && [ -z "$url" ]; }; then
  echo "::error::Set exactly one of 'path' or 'url'" >&2
  exit 1
fi

bin="${AI_CATALOG_PATH:-ai-catalog}"
if ! command -v "$bin" > /dev/null 2>&1; then
  echo "::error::ai-catalog not found. Run setup-ai-catalog first or set ai_catalog_path" >&2
  exit 1
fi

if [ -n "$url" ]; then
  path="$tmp/ai-catalog-input.json"
  curl -fsSL --retry 3 --max-time 60 -o "$path" "$url" || {
    echo "::error::Could not fetch $url" >&2
    exit 1
  }
  source_label="$url"
  annotation_file=""
else
  source_label="$path"
  annotation_file="$path"
fi

# validate exits 1 for an invalid catalog; the JSON report is on stdout either
# way, so read the report rather than the exit code.
report="$tmp/ai-catalog-report.json"
set +e
"$bin" validate --json "$path" > "$report"
status=$?
set -e
if ! jq -e 'has("valid")' "$report" > /dev/null 2>&1; then
  echo "::error::ai-catalog validate failed (exit $status) without a JSON report" >&2
  exit 1
fi

valid="$(jq -r '.valid' "$report")"
level="$(jq -r '.conformanceLevel // "none"' "$report")"
{
  echo "valid=$valid"
  echo "conformance_level=$level"
  echo "report=$report"
} >> "$out"

# Workflow-command escaping for annotation data and properties.
annotate() {
  severity="$1"
  jq -r --arg sev "$severity" --arg file "$annotation_file" '
    def esc: gsub("%"; "%25") | gsub("\r"; "%0D") | gsub("\n"; "%0A");
    def prop: esc | gsub(":"; "%3A") | gsub(","; "%2C");
    (if $sev == "error" then .errors else .warnings end)[]
    | "::\($sev) "
      + (if $file != "" then "file=\($file | prop)," else "" end)
      + "title=\((.code // "invalid-input") | prop)::\(((.path // "") + ": " + .message) | esc)"
  ' "$report"
}
annotate error
annotate warning

{
  echo "### AI Catalog validation"
  echo
  echo "- Source: \`$source_label\`"
  echo "- Valid: **$valid**"
  echo "- Conformance level: **$level**"
  if [ "$(jq '(.errors | length) + (.warnings | length)' "$report")" -gt 0 ]; then
    echo
    echo "| Severity | Code | Path | Message |"
    echo "|---|---|---|---|"
    jq -r '
      def cell: tostring | gsub("\\|"; "\\|") | gsub("\n"; " ");
      (.errors[] | ["error", (.code // "-"), (.path // "-"), .message]),
      (.warnings[] | ["warning", (.code // "-"), (.path // "-"), .message])
      | "| " + (map(cell) | join(" | ")) + " |"
    ' "$report"
  fi
} >> "$summary"

if [ "$valid" != "true" ] && [ "${FAIL_ON_INVALID:-true}" = "true" ]; then
  exit 1
fi
