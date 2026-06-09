# Release Scripts

Helper scripts used by the release workflows. Both require [yq](https://github.com/mikefarah/yq) to be installed.

## parse-version.sh

Reads `release.yaml`, validates that both `version` and `crate-version` are valid semver, and outputs version components. In CI it writes to `$GITHUB_OUTPUT`; locally it prints to stdout.

### Usage

```sh
.github/scripts/parse-version.sh [path-to-release-yaml]
```

### Examples

Using the default `release.yaml` at the repo root:

```sh
.github/scripts/parse-version.sh
```

Using a custom file:

```sh
cat > /tmp/test-release.yaml <<'EOF'
limitador:
  version: "2.5.0"
  crate-version: "0.13.0"

dependencies: {}
EOF

.github/scripts/parse-version.sh /tmp/test-release.yaml
```

Expected output:

```
version=2.5.0
major=2
minor=5
patch=0
release-branch=release-2.5
crate-version=0.13.0
crate-major=0
crate-minor=13
crate-patch=0
```

### Error cases

Invalid semver:

```sh
cat > /tmp/bad.yaml <<'EOF'
limitador:
  version: "not-a-version"
  crate-version: "0.13.0"
dependencies: {}
EOF

.github/scripts/parse-version.sh /tmp/bad.yaml
# ::error::Invalid semver for version: not-a-version
# exit code: 1
```

## validate-release-yaml.sh

Validates version gating rules for the version-gate CI check. On release branches it rejects `0.0.0` sentinel values and `-dev` suffixed versions. For any declared dependencies, it verifies the corresponding GitHub Release exists.

### Usage

```sh
.github/scripts/validate-release-yaml.sh <branch-name> [org] [path-to-release-yaml]
```

| Argument | Required | Default | Description |
|----------|----------|---------|-------------|
| `branch-name` | yes | | The branch name to validate against (e.g. `main`, `release-2.5`) |
| `org` | no | `Kuadrant` | GitHub organization for dependency release lookups |
| `path-to-release-yaml` | no | `release.yaml` | Path to the release.yaml file |

### Examples

Validating on `main` (sentinel `0.0.0` is allowed):

```sh
.github/scripts/validate-release-yaml.sh main
# release.yaml validation passed
```

Validating on a release branch with the default `release.yaml` (should fail because versions are `0.0.0` on main):

```sh
.github/scripts/validate-release-yaml.sh release-2.5
# ::error::release.yaml version is 0.0.0 on branch 'release-2.5' ...
# exit code: 1
```

Validating on a release branch with proper versions:

```sh
cat > /tmp/release-test.yaml <<'EOF'
limitador:
  version: "2.5.0"
  crate-version: "0.13.0"

dependencies: {}
EOF

.github/scripts/validate-release-yaml.sh release-2.5 Kuadrant /tmp/release-test.yaml
# release.yaml validation passed
```

### Error cases

Dev version on a release branch:

```sh
cat > /tmp/dev.yaml <<'EOF'
limitador:
  version: "2.5.0-dev"
  crate-version: "0.13.0"
dependencies: {}
EOF

.github/scripts/validate-release-yaml.sh release-2.5 Kuadrant /tmp/dev.yaml
# ::error::release.yaml version '2.5.0-dev' is a dev version on branch 'release-2.5' ...
# exit code: 1
```

Missing dependency release (requires `gh` CLI and authentication):

```sh
cat > /tmp/deps.yaml <<'EOF'
limitador:
  version: "2.5.0"
  crate-version: "0.13.0"

dependencies:
  some-repo: "99.99.99"
EOF

.github/scripts/validate-release-yaml.sh release-2.5 Kuadrant /tmp/deps.yaml
# ::error::Dependency 'some-repo' targets version '99.99.99', but release v99.99.99 does not exist in Kuadrant/some-repo
# exit code: 1
```
