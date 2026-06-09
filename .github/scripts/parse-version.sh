#!/usr/bin/env bash
set -euo pipefail

RELEASE_YAML="${1:-release.yaml}"

if [[ ! -f "$RELEASE_YAML" ]]; then
  echo "::error::File not found: $RELEASE_YAML"
  exit 1
fi

VERSION=$(yq '.limitador.version' "$RELEASE_YAML")
if [[ -z "$VERSION" || "$VERSION" == "null" ]]; then
  echo "::error::No version found in $RELEASE_YAML under limitador.version"
  exit 1
fi

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  echo "::error::Invalid semver for version: $VERSION"
  exit 1
fi

CRATE_VERSION=$(yq '.limitador."crate-version"' "$RELEASE_YAML")
if [[ -z "$CRATE_VERSION" || "$CRATE_VERSION" == "null" ]]; then
  echo "::error::No crate-version found in $RELEASE_YAML under limitador.crate-version"
  exit 1
fi

if ! [[ "$CRATE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  echo "::error::Invalid semver for crate-version: $CRATE_VERSION"
  exit 1
fi

MAJOR=$(echo "$VERSION" | cut --delimiter=. --fields=1)
MINOR=$(echo "$VERSION" | cut --delimiter=. --fields=2)
PATCH=$(echo "$VERSION" | cut --delimiter=. --fields=3 | cut --delimiter=- --fields=1)
RELEASE_BRANCH="release-${MAJOR}.${MINOR}"

CRATE_MAJOR=$(echo "$CRATE_VERSION" | cut --delimiter=. --fields=1)
CRATE_MINOR=$(echo "$CRATE_VERSION" | cut --delimiter=. --fields=2)
CRATE_PATCH=$(echo "$CRATE_VERSION" | cut --delimiter=. --fields=3 | cut --delimiter=- --fields=1)

echo "version=$VERSION" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "major=$MAJOR" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "minor=$MINOR" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "patch=$PATCH" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "release-branch=$RELEASE_BRANCH" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "crate-version=$CRATE_VERSION" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "crate-major=$CRATE_MAJOR" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "crate-minor=$CRATE_MINOR" >> "${GITHUB_OUTPUT:-/dev/stdout}"
echo "crate-patch=$CRATE_PATCH" >> "${GITHUB_OUTPUT:-/dev/stdout}"
