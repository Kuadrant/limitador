#!/usr/bin/env bash
set -euo pipefail

RELEASE_YAML="${1:-release.yaml}"

if [[ ! -f "$RELEASE_YAML" ]]; then
  echo "::error::File not found: $RELEASE_YAML"
  exit 1
fi

YAML_VERSION=$(yq '.limitador.version' "$RELEASE_YAML")
YAML_CRATE_VERSION=$(yq '.limitador."crate-version"' "$RELEASE_YAML")

CARGO_SERVER_VERSION=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name=="limitador-server") | .version')
CARGO_CRATE_VERSION=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name=="limitador") | .version')

ERRORS=0

# Special case: release.yaml 0.0.0 is valid alongside Cargo.toml -dev versions
if [[ "$YAML_VERSION" == "0.0.0" ]]; then
  if [[ "$CARGO_SERVER_VERSION" != *-dev* ]]; then
    echo "::error::release.yaml version is 0.0.0 but limitador-server Cargo.toml version '${CARGO_SERVER_VERSION}' does not end in -dev"
    ERRORS=$((ERRORS + 1))
  fi
else
  if [[ "$YAML_VERSION" != "$CARGO_SERVER_VERSION" ]]; then
    echo "::error::Server version mismatch: release.yaml has '${YAML_VERSION}' but Cargo.toml has '${CARGO_SERVER_VERSION}'"
    ERRORS=$((ERRORS + 1))
  fi
fi

if [[ "$YAML_CRATE_VERSION" == "0.0.0" ]]; then
  if [[ "$CARGO_CRATE_VERSION" != *-dev* ]]; then
    echo "::error::release.yaml crate-version is 0.0.0 but limitador Cargo.toml version '${CARGO_CRATE_VERSION}' does not end in -dev"
    ERRORS=$((ERRORS + 1))
  fi
else
  if [[ "$YAML_CRATE_VERSION" != "$CARGO_CRATE_VERSION" ]]; then
    echo "::error::Crate version mismatch: release.yaml has '${YAML_CRATE_VERSION}' but Cargo.toml has '${CARGO_CRATE_VERSION}'"
    ERRORS=$((ERRORS + 1))
  fi
fi

if [[ "$ERRORS" -gt 0 ]]; then
  echo "::error::Version consistency check failed with ${ERRORS} error(s)"
  exit 1
fi

echo "Version consistency check passed: release.yaml and Cargo.toml agree"
echo "  Server: release.yaml=${YAML_VERSION} Cargo.toml=${CARGO_SERVER_VERSION}"
echo "  Crate:  release.yaml=${YAML_CRATE_VERSION} Cargo.toml=${CARGO_CRATE_VERSION}"
