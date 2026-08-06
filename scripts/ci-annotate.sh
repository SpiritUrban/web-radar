#!/usr/bin/env bash
# Surface the tail of a log as a GitHub annotation.
#
# Run logs are unreadable without authentication even in a public repository
# ("Sign in to view logs"), while annotations are readable through the API:
#   /repos/<OWNER>/<REPO>/check-runs/<JOB_ID>/annotations
#
# IMPORTANT: GitHub truncates an annotation at 4096 characters and drops the
# TAIL — that is, the actual error. So this feeds at most ~2500 characters,
# strips ANSI colours (they eat half the budget invisibly) and drops cargo/npm
# progress lines (there are hundreds and they are never the cause).
#
#   bash scripts/ci-annotate.sh "Build output" build.log [limit]

set -uo pipefail

title="${1:?a title is required}"
file="${2:?a log file is required}"
limit="${3:-2500}"

if [ ! -s "$file" ]; then
  echo "::error title=${title}::${file} is missing or empty"
  exit 0
fi

clean=$(sed -e 's/\x1b\[[0-9;]*m//g' "$file")

trimmed=$(printf '%s\n' "$clean" \
  | grep -vE '^[[:space:]]*(Compiling|Checking|Downloaded|Downloading|Fresh|Updating|Adding|Locking|Installing|Removing|Unpacking|Preparing|Selecting|Get:|Reading|Building) ' \
  || true)
# If filtering left nothing, show the log as-is rather than going silent.
[ -n "$trimmed" ] && clean="$trimmed"

log=$(printf '%s' "$clean" | tail -c "$limit")

log="${log//'%'/'%25'}"
log="${log//$'\r'/'%0D'}"
log="${log//$'\n'/'%0A'}"

echo "::error title=${title}::${log}"
