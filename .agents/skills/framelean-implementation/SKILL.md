---
name: framelean-implementation
description: "Use to implement confirmed FrameLean changes in production code, tests, scripts, documentation, or project-level skills while preserving scope, architecture, user changes, and project style. Use after requirements are accepted or when the user explicitly requests execution. Use only inside the FrameLean repository."
---

# FrameLean Implementation

Read `.agents/skills/README.md` and the accepted requirement or plan. Inspect related implementation and tests before editing.

## Rules

- Make the smallest coherent change that completes the confirmed scope.
- Preserve unrelated user changes and avoid formatting churn, duplicate abstractions, speculative features, and unnecessary dependencies.
- Prefer existing helpers, providers, use cases, mappers, widgets, scripts, and documentation patterns.
- Follow the architecture currently documented in `docs/develop/architecture.md`; do not restore old layers or media runtime abstractions without a confirmed requirement.
- Update only documentation whose current facts changed. Keep temporary plans in `.workspace/`.
- If evidence invalidates the agreed scope or exposes a material risk, stop and explain before expanding the task.

After editing, inspect the diff and run proportionate checks for the changed surface. Use `framelean-validation` for deep review, broad suites, or validation-specific work.

Report what changed, the boundary it satisfies, relevant behavior deliberately left unchanged, checks run, and remaining risks.
