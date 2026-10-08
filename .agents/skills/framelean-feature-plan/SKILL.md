---
name: framelean-feature-plan
description: "Use for FrameLean feature planning after requirement discussion or analysis. Compare meaningful options, define scope and architecture boundaries, break work into implementation tasks, and specify proportionate validation. Persist temporary plans under .workspace/plans when useful. Use only inside the FrameLean repository."
---

# FrameLean Feature Plan

Read `.agents/skills/README.md`, the accepted requirement or analysis, and only the related current facts and code patterns.

Write a practical Chinese plan adapted to the task. Include:

- Recommended outcome and why.
- In-scope and out-of-scope boundaries.
- Meaningful alternatives only when a real decision exists.
- Ordered implementation tasks with likely files or modules.
- Validation coverage and commands or inspection methods.
- Material compatibility, migration, platform, or rollback risks.

Follow the architecture currently documented in `docs/develop/architecture.md`. The restarted branch is a minimal Flutter shell; do not assume the former `features/application/domain/infrastructure` structure until a new architecture is explicitly chosen.

Default to inline output. Save a plan that must survive the current conversation to `.workspace/plans/YYMMDD-feature-slug.md`; do not place temporary plans in `docs/`. Suggest a branch only when the user requests one or the workflow needs it. Do not implement code in this Skill.
