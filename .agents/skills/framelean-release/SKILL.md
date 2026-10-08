---
name: framelean-release
description: "Use to summarize and discuss FrameLean changes for a user-specified version, then produce a concise user-facing release document. Scans Git history and current packaging facts, drafts release notes, asks for confirmation, and writes docs/releases/version/release.md when approved. Use only inside the FrameLean repository."
---

# FrameLean Release

Create a release document for a user-specified version. This skill is for version narrative and release notes, not commit or PR copy.

## Shared Context

Read `.agents/skills/README.md` first and follow the shared pre-read protocol.

## Version Rule

The user must specify the target version, such as `v1.1.6`. If no version is specified, ask for it before scanning or writing.

## Scan Scope

Use targeted Git and docs reads:

```text
git tag --sort=-creatordate
git log previous-tag..HEAD
git diff --stat previous-tag..HEAD
CHANGELOG.md
docs/develop/restart-baseline.md
docs/develop/
docs/work/
```

`develop/next` 当前有 Flutter → Work → FLL 的最小 smoke 链路，但只有在本机能够构建目标平台的静态 libav SDK 时，完整引擎构建和打包才可验证。不要仅凭 CI 配置或归档分支发布记录声称当前版本可发布。

Before claiming release readiness, inspect the current packaging and update surfaces that actually exist:

```text
desktop-client/pubspec.yaml
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

Use additional server/Admin files only when the release touches self-hosted update metadata, release package requirements, appcast generation, upload flows, or download-ticket behavior.

## Packaging Freshness Gate

Before drafting or writing the release document, compare the current packaging source files against the release claims. Do not rely on older release docs as proof that packaging still works.

Check at minimum:

- A current macOS and Windows packaging path exists and its CI/local entry points pass the required configuration.
- Artifact names, contents, platform support, signing, update behavior, and upload paths match the active scripts and generated artifacts.
- Missing release configuration fails clearly rather than producing an artifact that appears ready but is incomplete.
- Optional mechanisms such as FFmpeg bundling or Sparkle are included only if the current app and verified packaging path actually use them.

If a current packaging path is absent or the gate finds stale wiring, do not claim release readiness. Ask whether to reconnect and verify the path first or record the limitation in the release document. Keep routine gate results out of the public release narrative; record only unresolved, user-relevant compatibility or packaging risks.

## Workflow

1. Identify the previous relevant tag and state the comparison range.
2. Summarize user-facing features, important fixes, compatibility changes, and known risks. Keep workflow, documentation, and routine validation details in their source records.
3. Ask focused questions when version scope, included changes, platform support, release artifacts, or known risks are unclear.
4. Draft release copy in the conversation or `.workspace/release-drafts/vX.Y.Z.md` during discussion.
5. After user confirmation, write:

```text
docs/releases/vX.Y.Z/release.md
```

## Release Document Shape

Use Chinese unless the user asks otherwise.

```markdown
# vX.Y.Z Release

## 版本摘要

## 主要变更

## 重要修复

## 兼容性与已知问题

## 完整记录
```

Keep the summary to one short paragraph, then use 3～6 user-facing bullets per relevant section. Omit empty sections instead of writing `无` or `不适用`. Do not paste test logs, internal workflow changes, file inventories, packaging checklists, artifact lists already visible on the release page, or generic rollback instructions. Link to the full changelog and detailed migration documentation when needed. Do not tag, push, publish, or create release artifacts unless the user explicitly asks.
