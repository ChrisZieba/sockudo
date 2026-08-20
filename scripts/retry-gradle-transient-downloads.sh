#!/usr/bin/env bash

set -uo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: retry-gradle-transient-downloads.sh <gradle command> [args...]" >&2
  exit 64
fi

gradle_retry_max_attempts="${GRADLE_TRANSIENT_MAX_ATTEMPTS:-3}"
gradle_retry_delay_seconds="${GRADLE_TRANSIENT_RETRY_DELAY_SECONDS:-15}"

if ! [[ "$gradle_retry_max_attempts" =~ ^[1-9][0-9]*$ ]]; then
  echo "GRADLE_TRANSIENT_MAX_ATTEMPTS must be a positive integer" >&2
  exit 64
fi
if ! [[ "$gradle_retry_delay_seconds" =~ ^[0-9]+$ ]]; then
  echo "GRADLE_TRANSIENT_RETRY_DELAY_SECONDS must be a non-negative integer" >&2
  exit 64
fi

gradle_retry_log="$(mktemp "${TMPDIR:-/tmp}/sockudo-gradle-retry.XXXXXX")"
gradle_retry_cleanup() {
  rm -f "$gradle_retry_log"
}
trap gradle_retry_cleanup EXIT

gradle_retry_attempt=1
while true; do
  "$@" 2>&1 | tee "$gradle_retry_log"
  gradle_retry_status=${PIPESTATUS[0]}
  if [[ $gradle_retry_status -eq 0 ]]; then
    exit 0
  fi

  if [[ $gradle_retry_attempt -ge $gradle_retry_max_attempts ]] ||
    ! grep -Eiq 'could not (get resource|GET|HEAD|resolve)|could not resolve all artifacts' "$gradle_retry_log" ||
    ! grep -Eiq 'status code (429|500|502|503|504)|HTTP response code: (429|500|502|503|504)|too many requests|read timed out|connection (reset|refused|timed out)|temporary failure in name resolution|name or service not known' "$gradle_retry_log"; then
    exit "$gradle_retry_status"
  fi

  gradle_retry_wait_seconds=$((gradle_retry_delay_seconds * gradle_retry_attempt))
  echo "::warning::Gradle dependency download failed transiently; retrying in ${gradle_retry_wait_seconds}s (attempt $((gradle_retry_attempt + 1))/${gradle_retry_max_attempts})"
  sleep "$gradle_retry_wait_seconds"
  gradle_retry_attempt=$((gradle_retry_attempt + 1))
done
