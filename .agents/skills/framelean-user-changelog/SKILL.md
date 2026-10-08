---
name: framelean-user-changelog
description: "Use to generate friendly, app-facing FrameLean version logs in Markdown and save them to the current user's Downloads directory. Trigger when the user asks for a version log, update log, changelog, what's new copy, or content for the software's full release-notes page. Default to the current version in desktop-client/pubspec.yaml when no version is specified. Use framelean-release instead for formal release documents and packaging readiness."
---

# FrameLean User Changelog

Create concise Markdown that tells software users what changed and why it matters. Keep formal release documentation and packaging checks in `framelean-release`.

## Shared Context

Read `.agents/skills/README.md` first and follow its shared pre-read protocol.

## Choose the Version

- Use an explicitly requested version and normalize it to `vX.Y.Z`.
- Otherwise, read `version:` from `desktop-client/pubspec.yaml`, remove the `+build` suffix, and use that semantic version. Do not ask for a version merely because the user omitted it.
- Omit the build number from the public title unless distinguishing builds is necessary and supported by project facts.

## Gather Evidence

1. Read `CHANGELOG.md` and the matching `docs/releases/vX.Y.Z/` files when they exist.
2. Inspect product release tags matching `vX.Y.Z` and choose the highest semantic version lower than the target as the previous release. If the target tag exists, compare that previous tag through the target tag. If it does not exist, compare the previous tag through `HEAD`.
3. Use targeted Git history, decisions, lessons, active work, implementation, and tests only when the version documents do not establish the user-visible facts.
4. Include only changes shipped in the target version. Do not mix in later work, uncommitted changes, speculative plans, or claims inferred from a commit title alone.

If evidence conflicts, prefer verified implementation and Git scope, then note the uncertainty outside the user-facing Markdown. Do not turn packaging readiness into a prerequisite for drafting this log.

## Write for Software Users

- Use Chinese unless the user requests another language.
- Lead with the benefit: prefer “现在可以批量处理任务夹” over internal component or architecture names.
- Translate implementation details into observable behavior. For example, write “媒体分析或处理超时后不再持续占用资源” instead of naming FFprobe / FFmpeg, and “导出完成前不再出现不完整文件” instead of describing partial files or atomic publication.
- Keep a technical product term only when users see it in the interface, need it to act, or explicitly ask for technical detail; explain it in the same sentence when necessary.
- Combine related fixes into outcomes users can recognize.
- Exclude class names, database schema numbers, architecture refactors, test details, CI changes, commit ids, file paths, and internal process notes.
- Include compatibility or upgrade information only when users need to act on it.
- Avoid unsupported superlatives, vague claims such as “全面优化”, and repetitive “新增 / 修复” wording.

## Output Shape

Use this friendly variation of the release format:

```markdown
# FrameLean vX.Y.Z 更新日志

## 这次更新

用一小段话概括最值得用户关注的变化。

## 新功能与改进

- 以用户收益为主的变化。

## 问题修复

- 用户能识别的问题与结果。

## 升级提醒

- 仅保留用户需要采取行动的兼容性或升级信息。
```

Keep the summary to one short paragraph and each relevant section to roughly 2～6 bullets. Omit empty sections instead of writing “无” or “不适用”. Add public links only when they help users; do not add an internal “完整记录” section by default.

## Save to Downloads

- Always save the finished Markdown document to the current user's system Downloads directory. Do not leave the final result only in the conversation.
- Use `FrameLean-vX.Y.Z-更新日志.md` as the default filename, with the resolved target version substituted into the name.
- Resolve the actual Downloads directory at runtime for the current operating system; do not hard-code a developer-specific absolute path.
- Keep the file in Downloads when the user specifies only a filename. Use another directory only when the user explicitly overrides the destination.
- Inspect an existing target file before replacing it and preserve unrelated user content.
- Return the saved file's absolute clickable path in the final response.

Do not publish, upload, tag, or create release artifacts without explicit authorization.
