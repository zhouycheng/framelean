---
name: framelean-delivery
description: "Use for FrameLean delivery closeout after implementation or validation. Calibrates current project facts, updates necessary documentation, and returns Markdown commit information plus a concise reviewer-facing PR description. Use only inside the FrameLean repository."
---

# FrameLean Delivery

Prepare a change for handoff by making project facts current and producing commit and PR copy. This skill does not generate formal release documents; use `framelean-release` for that.

## Shared Context

Read `.agents/skills/README.md` first and follow the shared pre-read protocol. Always inspect Git status, current branch, changed files, and relevant diffs.

## Documentation Calibration

Identify which facts the changed files can affect, then inspect only those current-fact documents. Typical targets are `CONTEXT.md`, `CHANGELOG.md`, the relevant `docs/work/`, `docs/develop/`, `docs/reference/`, or `.agents/skills/` entries. Release documents may be created under `docs/releases/` when a version is actually ready. Use `rg` for stale names and paths instead of opening the entire documentation tree.

## Packaging Freshness Check

Run a packaging and update-config freshness check only when the change affects release, CI, installer, update, signing, notarization, packaging, or related workflow facts.

Inspect the current source of truth instead of trusting existing docs:

```text
.github/workflows/*.yml
scripts/README.md
scripts/build/package_macos.sh
scripts/build/package_windows.ps1
desktop-client/macos/Runner/Info.plist
desktop-client/macos/Runner/Configs/*.xcconfig
desktop-client/windows/CMakeLists.txt
fengine/Cargo.toml
fll/Cargo.toml
legal/
```

Use targeted `rg` searches for update/signing variables such as `FRAMELEAN_UPDATE`, `FRAMELEAN_SPARKLE`, `SUPublicEDKey`, `SUFeedURL`, `FRAMELEAN_RELEASE`, `notarization`, `sign_update`, and artifact paths.

Check at minimum:

- Active CI and local packaging entry points pass the required configuration to the current release scripts.
- Artifact names, contents, signing, update behavior, and output paths match the active scripts and generated artifacts.
- Missing release configuration fails clearly rather than producing an incomplete artifact that appears ready.
- Optional mechanisms such as FFmpeg bundling or Sparkle are checked only if the current app and verified packaging path actually use them.

The new Work/FLL packaging path requires a platform-specific static libav SDK and an observed package smoke; configuration files alone are not evidence that the app can be released.

If the check finds drift, update the stale script, workflow, YAML, or docs before final delivery, or explicitly report the blocker. Do not copy routine packaging checks into the PR description; include only a material unresolved risk under optional notes.

## Update Rules

- Update `CHANGELOG.md` when the change is worth version-level recall.
- Update `docs/releases/` when a stable shipped design or workflow fact changed.
- Update `docs/work/decisions.md` and the affected engineering fact when an important durable decision was made.
- Add reusable lessons to the engineering fact they affect; create a separate lessons document only when multiple current lessons justify it.
- Update `docs/work/active.md` when current task status changed.
- Update `AGENTS.md`, `CLAUDE.md`, `README.md`, or `.agents/skills/` when agent workflow or developer workflow changed.
- Do not create `docs/archive/`, `docs/features/`, `docs/plans/`, daily logs, or one-bug-one-file notes.

## Delivery Copy

Always return both sections in Markdown:

````markdown
## Commit 信息

```text
type(scope): 中文摘要
```

提交范围：

- ...

提交正文：

- 需要或不需要
- 原因：...

## PR Description

```markdown
## Summary

- 用一段短文或 2～5 个 bullet 说明改了什么，以及不明显时为什么要改。
- 需要时直接附 `Closes #...` 或相关设计文档链接。

## Notes

- 仅保留评审者必须知道的迁移、兼容性、风险、截图或特殊决策；没有就删除整个章节。
```
````

Keep ordinary PR bodies around 5～15 lines. Scale detail with review risk, not diff size or a fixed section count. Do not include a `Verification` section, routine test commands, file lists, commit summaries, documentation inventories, empty headings, or generic rollback boilerplate. Link to issues, decisions, changelogs, or CI instead of copying their content.

Do not run `git add`, `git commit`, `git push`, create tags, or open PRs unless the user explicitly asks.
