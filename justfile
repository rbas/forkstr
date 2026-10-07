set shell := ["bash", "-cu"]

default:
    @just --list

# Run the same gates as CI, in lock-friendly order.
check:
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo nextest run --locked
    cargo build --locked --release

# Print the current tagged version.
version:
    @convco version

# Infer the next version, or force patch/minor/major.
next-version kind="auto":
    @case "{{kind}}" in \
      auto)  convco version --bump ;; \
      patch) convco version --bump --patch ;; \
      minor) convco version --bump --minor ;; \
      major) convco version --bump --major ;; \
      *) echo "kind must be auto, patch, minor, or major" >&2; exit 2 ;; \
    esac

# Update Cargo.toml, Cargo.lock, and CHANGELOG.md without committing.
bump kind="auto":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{kind}}" in
      auto)  next="$(convco version --bump)" ;;
      patch) next="$(convco version --bump --patch)" ;;
      minor) next="$(convco version --bump --minor)" ;;
      major) next="$(convco version --bump --major)" ;;
      *) echo "kind must be auto, patch, minor, or major" >&2; exit 2 ;;
    esac
    current="$(sed -nE '/^\[package\]$/,/^\[/{s/^version = "([^"]+)"/\1/p;}' Cargo.toml)"
    if [[ -z "$current" ]]; then
      echo "could not read package version from Cargo.toml" >&2
      exit 2
    fi
    if [[ "$current" == "$next" ]]; then
      echo "Cargo.toml is already at $next" >&2
      exit 2
    fi
    NEXT_VERSION="$next" perl -0pi -e \
      's/(\[package\][\s\S]*?\nversion = ")[^"]+("\n)/$1$ENV{NEXT_VERSION}$2/' Cargo.toml
    cargo check
    convco changelog --unreleased "$next" --output CHANGELOG.md
    echo "Bumped $current -> $next"

# Bump, commit the release files, and create an annotated vX.Y.Z tag.
release kind="auto":
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -n "$(git status --porcelain)" ]]; then
      echo "release requires a clean working tree" >&2
      exit 2
    fi
    just bump "{{kind}}"
    version="$(sed -nE '/^\[package\]$/,/^\[/{s/^version = "([^"]+)"/\1/p;}' Cargo.toml)"
    git add Cargo.toml Cargo.lock CHANGELOG.md
    git commit -m "chore(release): v$version"
    git tag -a "v$version" -m "v$version"
    echo "Created v$version. Publish with: git push origin HEAD --follow-tags"
