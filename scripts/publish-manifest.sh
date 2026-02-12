#!/usr/bin/env bash
set -euo pipefail

output="${1:-installers.toml}"
key_file="${MANIFEST_SIGNING_KEY_FILE:-}"

if [[ -z "${key_file}" ]]; then
  if [[ -z "${MANIFEST_SIGNING_KEY:-}" ]]; then
    echo "MANIFEST_SIGNING_KEY or MANIFEST_SIGNING_KEY_FILE must be set" >&2
    exit 1
  fi
  key_file="$(mktemp)"
  printf "%s" "$MANIFEST_SIGNING_KEY" > "$key_file"
  chmod 600 "$key_file"
  cleanup_key=1
else
  cleanup_key=0
fi

cargo run --quiet --bin manifest-merge -- --output "$output" assets/manifest/*.toml

openssl pkeyutl -sign -inkey "$key_file" -rawin -in "$output" -out "$output.sig.bin"
xxd -p -c 256 "$output.sig.bin" > "$output.sig"

rm -f "$output.sig.bin"

if [[ "$cleanup_key" -eq 1 ]]; then
  rm -f "$key_file"
fi

echo "Wrote $output and $output.sig"
