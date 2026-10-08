---
name: framelean-validation
description: "Use for FrameLean validation planning, diff review, targeted or broad check execution, failure diagnosis, and re-validation after implementation. Reviews scope, architecture, tests, docs, packaging, and user-change preservation. Use only inside the FrameLean repository."
---

# FrameLean Validation

Read `.agents/skills/README.md`. Inspect Git status and the relevant diff before selecting checks.

Review for scope creep, architecture violations, missing behavior coverage, stale facts, broken runtime or platform assumptions, and unrelated user changes. Run targeted checks first, then broaden according to risk.

For changed Dart files, check formatting without modifying files:

```bash
cd desktop-client
dart format --output=none --set-exit-if-changed <changed-dart-files>
flutter analyze
flutter test <targeted-tests>
```

Run the full Flutter suite only when the changed surface or requested confidence justifies it.

For Rust Work changes, from the repository root:

```bash
cargo check --manifest-path fengine/Cargo.toml --all-targets --locked
cargo test --manifest-path fengine/Cargo.toml --locked
```

FLL checks require the platform's bundled static libav SDK. Use the runtime build script when that SDK is available.

For docs or project Skills:

```bash
git diff --check
python3 <current-skill-creator>/scripts/quick_validate.py .agents/skills/framelean-skill-name
```

Resolve the available system `skill-creator` from the current environment; do not embed a user-specific absolute path. If its Python validator cannot import YAML, use another structured YAML parser and report the fallback. Scan for stale paths and deleted names with `rg`.

Report the checked scope, commands and concrete results, actionable findings, and risks not covered. Never claim a check passed unless it ran successfully.
