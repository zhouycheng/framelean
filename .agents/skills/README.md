# FrameLean Skills 使用说明

本目录保存只服务 FrameLean 仓库的项目级 skills。

## 项目规则

- Skill 位于 `.agents/skills/framelean-*`，使用小写连字符命名。
- 新增、删除、合并或重命名后，同步更新本文件和 `framelean-workflow/SKILL.md`。
- 临时计划和 release 草稿放入 ignored 的 `.workspace/`；`docs/` 只保存确认后的当前事实、版本事实、决策和经验。
- 不创建 skill 级 README、CHANGELOG、安装说明或过程记录，也不复制用户级 Skill。

## 共享预读协议

按需递增读取，避免每次加载全部文档。

1. 默认读取 `CONTEXT.md`、`docs/README.md` 和 `docs/work/active.md`。如果仓库指令尚未应用，再读取 `AGENTS.md`。
2. 按任务读取 `docs/develop/restart-baseline.md`、相关 `docs/develop/` 或 `docs/reference/`。旧版本事实位于归档 Git 历史，不把它们当作当前功能承诺。
3. 分析、实现或验证具体功能时，用 `rg` 定位相关源码、测试、脚本和配置，不全量读取目录。
4. 只有验证、交付、发布或用户明确要求 Git 操作时，读取 Git 状态、diff、历史和标签。

## 临时工作区

```text
.workspace/
  plans/YYMMDD-feature-slug.md
  release-drafts/vX.Y.Z.md
```

临时文件不进入版本库；稳定事实由交付或发布流程写入正式文档。

## Skill 路由

| 用户意图 | Skill |
| --- | --- |
| 不确定使用哪个 Skill，或要求跨阶段完整流程 | `framelean-workflow` |
| 分析需求、现有功能、交互、依赖和边界 | `framelean-feature-analysis` |
| 比较方案、划定范围、拆分任务和验证边界 | `framelean-feature-plan` |
| 实现已确认的代码、测试、脚本、文档或 Skill 变更 | `framelean-implementation` |
| 制定验证范围、审查 diff、运行检查和解释失败 | `framelean-validation` |
| 校准受影响事实，准备 commit 信息和精简 PR 描述 | `framelean-delivery` |
| 为指定版本产出面向用户的精简 release 文档 | `framelean-release` |
| 为当前或指定版本生成友好版本日志并保存到下载目录 | `framelean-user-changelog` |
| 创建、合并、删除或重构项目级 Skills | `framelean-skill-create` |

常规完整流程为：分析 → 计划 → 实现 → 验证 → 交付。正式发布文档独立使用 `framelean-release`，软件内版本日志使用 `framelean-user-changelog`；Skill 维护使用 `framelean-skill-create` 后进入实现和验证。

## 共同底线

- 先核对真实项目状态，再更新文档。
- `develop/next` 是当前开发集成线；功能工作通过短生命周期分支合入。`main` 与 Alpha 保持冻结。
- 不处理无关用户改动，不擅自 stage、commit、push、revert 或发布。
- 不创建 `docs/archive/`、`docs/features/`、`docs/plans/` 或 `docs/product/roadmap.md`。
