# How to release Limitador

Limitador uses a two-phase release process.
A **pre-release** workflow prepares code changes and opens a pull request.
A **release** workflow builds artifacts and creates the GitHub Release after the PR is merged.

## Version conventions

Limitador maintains two independent version numbers:

- **Server version** (e.g. `2.5.0`) — used for container image tags and as the primary release version
- **Crate version** (e.g. `0.13.0`) — used for the `limitador` library crate on crates.io

**Source of truth: `Cargo.toml`.** The `limitador/Cargo.toml` and `limitador-server/Cargo.toml` package versions are authoritative. `release.yaml` at the repo root is a generated mirror that `kuadrant-operator` and other cross-repo tooling reads for a uniform version file regardless of language.

On `main`, `Cargo.toml` versions end in `-dev` (e.g. `0.13.0-dev`) and `release.yaml` uses `0.0.0` sentinel values. On release branches, both files must agree exactly.

### Tag conventions

Each release creates three git tags:

- `vX.Y.Z` — primary tag (server version), used by other Kuadrant components
- `server-vX.Y.Z` — server-specific tag (backward compatibility)
- `crate-vX.Y.Z` — crate-specific tag (backward compatibility)

### Branch conventions

Release branches follow the `release-X.Y` pattern (e.g. `release-2.5` for server version `2.5.0`).
Patch releases reuse the same branch (e.g. `2.5.1` releases from `release-2.5`).

## Prerequisites

### Repository secrets

The following secrets must be configured in **Settings > Secrets and variables > Actions > Repository secrets**:

| Secret | Purpose | Used by |
|--------|---------|---------|
| `CARGO_REGISTRY_TOKEN` | crates.io API token with publish permission for the `limitador` crate | Release workflow (`cargo publish`) |
| `IMG_REGISTRY_USERNAME` | quay.io robot account username (e.g. `kuadrant+ci`) | Build Image workflow (docker login) |
| `IMG_REGISTRY_TOKEN` | quay.io robot account token | Build Image workflow (docker login) |

> `GITHUB_TOKEN` is provided automatically by GitHub Actions and does not need to be configured.
> It is used for creating tags, GitHub Releases, and opening pull requests.

## Minor release

### Phase 1: Pre-release

1. Go to **Actions > Pre-release** and click **Run workflow**
2. Enter the **server version** (e.g. `2.5.0`) and **crate version** (e.g. `0.13.0`)
3. Optionally set the **source branch** (defaults to `main`).
   For patch releases, set this to the existing release branch.
4. The workflow will:
   - Create the release branch `release-X.Y` from the source branch (if it doesn't exist)
   - Create a `pre-release-vX.Y.Z` working branch
   - Set `limitador/Cargo.toml` version via `cargo set-version` and verify with `cargo metadata`
   - Set `limitador-server/Cargo.toml` version via `cargo set-version` and verify with `cargo metadata`
   - Generate `release.yaml` from Cargo.toml values
   - Update `Cargo.lock`
   - Open a PR against the release branch
   - Open a **post-release PR** to `main` bumping versions to the next `-dev` and resetting `release.yaml` to `0.0.0` (do not merge until after the release)

### Review gate

5. Review the PR. CI will run:
   - Standard CI checks (fmt, clippy, test, image build)
   - **Version gate** — validates `release.yaml` and `Cargo.toml` agree, and on release branches rejects sentinel/dev versions
6. Approve and merge the PR

### Phase 2: Release

7. Go to **Actions > Release** and click **Run workflow**
8. Enter the **release branch** (e.g. `release-2.5`)
9. The workflow will (in strict order):
   - **Read version** from `release.yaml` and verify no existing GitHub Release
   - **Smoke tests** — fmt, clippy, check, full test suite, `cargo publish --dry-run`
   - **Tag** — create and push three tags (`vX.Y.Z`, `server-vX.Y.Z`, `crate-vX.Y.Z`)
   - **Publish crate** — `cargo publish -p limitador` to crates.io
   - **Build images** — multi-arch container images (amd64, arm64, s390x) pushed to quay.io
   - **Create GitHub Release** — only after all artifacts succeed

### Verify

10. Confirm the release artifacts:
   - [GitHub Release](https://github.com/Kuadrant/limitador/releases) exists with correct tag
   - [limitador on crates.io](https://crates.io/crates/limitador) shows the new version
   - Container image `quay.io/kuadrant/limitador:vX.Y.Z` is available

## Patch release

The process is identical to a minor release, except:

- The release branch (`release-X.Y`) already exists
- Enter the patch version (e.g. `2.5.1`) and corresponding crate version
- Set the **source branch** to the existing release branch (e.g. `release-2.5`)
- Before running the pre-release workflow, backport any fixes to the release branch.
  Create a branch, cherry-pick the commits, and open a PR against the release branch.
  Branch protections prevent pushing directly to release branches.

## After the release

When the source branch is `main` (i.e. a minor release), the pre-release workflow automatically opens a **post-release PR** to `main`.
This PR bumps both `Cargo.toml` files to the next minor `-dev` version (e.g. `2.5.0` → `2.6.0-dev`, `0.13.0` → `0.14.0-dev`).
It is created alongside the release PR but should only be merged after the release is complete.

For patch releases (source branch is a release branch), the post-release job is skipped.

## Rollback

If the release workflow fails partway through:

- **Failed during smoke tests**: No artifacts created. Fix the issue and re-run.
- **Failed during tag**: No artifacts created. Fix the issue and re-run.
- **Failed during crate publish**: Tags exist but no GitHub Release.
  The crate may or may not have been published — check crates.io.
  If the crate was published, you cannot re-publish the same version.
  Delete the tags manually if needed, fix the issue, and re-run with a patch version.
- **Failed during image build**: Tags exist and crate is published, but no GitHub Release.
  Delete the tags, yank the crate version on crates.io if needed, fix the issue, and re-run with a patch version.
  Alternatively, fix the image build and re-run only the release workflow (the tag and crate steps will detect existing artifacts and fail — manual intervention may be needed).
- **Failed during GitHub Release creation**: All artifacts exist.
  Create the GitHub Release manually via `gh release create`.

## Files

| File | Purpose |
|------|---------|
| `limitador/Cargo.toml` | Crate version (source of truth) |
| `limitador-server/Cargo.toml` | Server version (source of truth) |
| `release.yaml` | Generated mirror for cross-repo tooling (`kuadrant-operator`) |
| `.github/workflows/pre-release.yaml` | Phase 1: set Cargo.toml versions, generate release.yaml, open PR |
| `.github/workflows/release.yaml` | Phase 2: test, tag, build, publish, release |
| `.github/workflows/version-gate.yaml` | CI check: release.yaml/Cargo.toml consistency (all branches) |
| `.github/workflows/build-image.yaml` | Multi-arch container image build (reusable) |
| `.github/scripts/parse-version.sh` | Parse and validate versions from release.yaml |
| `.github/scripts/sync-release-yaml.sh` | Generate release.yaml from Cargo.toml via cargo metadata |
| `.github/scripts/check-versions.sh` | Validate release.yaml and Cargo.toml agree |
| `.github/scripts/validate-release-yaml.sh` | Version gate: reject sentinel/dev versions on release branches |
