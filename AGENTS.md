# FrameLean Agent Instructions

This file is the shared project entry point for AI coding agents.

## Sources of Truth

- Current project facts: `CONTEXT.md` and `docs/README.md`.
- Git, branch, validation, commit, PR, and release workflow: `docs/develop/workflow.md`.
- Project Skill routing and shared pre-read rules: `.agents/skills/README.md`.
- Agent workflow router: `.agents/skills/framelean-workflow/SKILL.md`.
- Specialized project Skills: `.agents/skills/framelean-*`.

When docs and implementation disagree, verify the real project state and update stale current-fact documentation. Follow `docs/develop/workflow.md` for Git-specific conflicts.

## Repository Boundaries

- FrameLean project Skills belong under `.agents/skills/`; tool-specific settings belong only in that tool's directory.
- `CLAUDE.md` remains a thin adapter to this shared instruction file.
- Preserve unrelated user changes. Do not stage, commit, push, revert, delete, or format them.
- `develop/next` is the active long-lived integration branch. Create short-lived `feature/*`, `fix/*`, `chore/*`, `docs/*`, `release/*`, or `hotfix/*` branches from it and merge them back through PR / MR; do not develop on the frozen `main` or `develop/v1.3.0-alpha` refs.
- Use ignored `.workspace/` for temporary plans and release drafts, and ignored `worktrees/` for local worktrees.

## Working Rules

- Discuss non-trivial requirements, alternatives, risks, and boundaries before implementation unless the user requests direct end-to-end execution.
- Inspect relevant docs, implementation, tests, scripts, and configuration before changing behavior.
- Make the smallest coherent change that satisfies the confirmed requirement and current architecture.
- Run checks proportionate to the touched files; use `docs/develop/workflow.md` for Dart / Flutter and delivery requirements.
- Update `docs/` when a change affects current architecture, data models, workflow, release facts, user-visible behavior, durable decisions, or reusable lessons.
- Do not create `docs/archive/`, `docs/features/`, `docs/plans/`, or `docs/product/roadmap.md`.
- Keep generated diagrams, screenshots, exports, local outputs, and temporary execution artifacts out of commits unless explicitly requested.
