#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
if ! git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "error: expected a Git worktree at $ROOT" >&2
  exit 1
fi

for marker in .gitignore desktop-client/pubspec.yaml fengine/Cargo.toml fll/Cargo.toml; do
  if [[ ! -f "$ROOT/$marker" ]]; then
    echo "error: repository marker is missing: $ROOT/$marker" >&2
    exit 1
  fi
done

targets=(
  build
  desktop-client/build
  desktop-client/.dart_tool
  desktop-client/.pub
  desktop-client/.pub-cache
  desktop-client/.flutter-plugins
  desktop-client/.flutter-plugins-dependencies
  desktop-client/coverage
  desktop-client/macos/Flutter/ephemeral
  desktop-client/macos/Pods
  desktop-client/windows/flutter/ephemeral
  fengine/target
  fll/target
)

for relative_path in "${targets[@]}"; do
  path="$ROOT/$relative_path"
  case "$path" in
    "$ROOT"/*) ;;
    *) echo "error: refusing to clean a path outside the repository: $path" >&2; exit 1 ;;
  esac
  if [[ -e "$path" || -L "$path" ]]; then
    printf 'Removing %s\n' "$relative_path"
    rm -rf -- "$path"
  fi
done

if [[ -d "$ROOT/.workspace" ]]; then
  while IFS= read -r -d '' log_path; do
    printf 'Removing %s\n' "${log_path#"$ROOT/"}"
    rm -f -- "$log_path"
  done < <(find "$ROOT/.workspace" -type f -name '*.log' -print0)
fi
