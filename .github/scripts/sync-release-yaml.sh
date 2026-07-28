#!/usr/bin/env bash
set -euo pipefail

RELEASE_YAML="${1:-release.yaml}"

CRATE_VERSION=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name=="limitador") | .version')
SERVER_VERSION=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name=="limitador-server") | .version')

if [[ -z "$CRATE_VERSION" || "$CRATE_VERSION" == "null" ]]; then
  echo "::error::Could not read limitador version from cargo metadata"
  exit 1
fi

if [[ -z "$SERVER_VERSION" || "$SERVER_VERSION" == "null" ]]; then
  echo "::error::Could not read limitador-server version from cargo metadata"
  exit 1
fi

# On main, Cargo.toml has -dev versions but release.yaml uses 0.0.0 sentinel.
# Strip -dev suffix: if present, write 0.0.0 instead.
if [[ "$SERVER_VERSION" == *-dev* ]]; then
  SERVER_VERSION="0.0.0"
fi
if [[ "$CRATE_VERSION" == *-dev* ]]; then
  CRATE_VERSION="0.0.0"
fi

yq --inplace ".limitador.version = \"${SERVER_VERSION}\"" "$RELEASE_YAML"
yq --inplace ".limitador.\"crate-version\" = \"${CRATE_VERSION}\"" "$RELEASE_YAML"

echo "release.yaml synced: version=${SERVER_VERSION}, crate-version=${CRATE_VERSION}"
