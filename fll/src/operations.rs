use crate::{Operation, analysis, backends::libav, planning, source, validation};
use fll_contracts::*;
use std::path::Path;
pub(crate) fn execute(
    input: &str,
    temp: &str,
    media: &MediaFacts,
    plan: &ExecutionPlan,
    grant: &ResourceGrant,
    op: &Operation,
) -> Result<ExecutionResult> {
    if grant.threads == 0 || grant.threads > plan.resource_hint.max_threads {
        return Err(MediaError::new(
            "resource_grant_invalid",
            "execute",
            "线程额度不在方案允许范围内",
        ));
    }
    let actual = analysis::analyze(input, None, op)?;
    if actual.source_stamp != media.source_stamp {
        return Err(MediaError::new(
            "source_changed",
            "execute",
            "源文件已变化，请重新分析",
        ));
    }
    let expected = planning::plan(&actual, &plan.intent, &plan.selection)?;
    if expected != *plan {
        return Err(MediaError::new(
            "plan_stale",
            "execute",
            "处理方案或当前能力已变化，请重新规划",
        ));
    }
    if std::fs::canonicalize(input).ok() == std::fs::canonicalize(temp).ok()
        || source::stamp(Path::new(temp)).is_ok_and(|s| s.identity == actual.source_stamp.identity)
    {
        return Err(MediaError::new(
            "output_conflict",
            "execute",
            "输出不能覆盖输入",
        ));
    }
    let opened = libav::Input::open(input, op)
        .map_err(|e| MediaError::new("media_unreadable", "execute", e))?;
    match &plan.parameters {
        ProcessingParameters::Transcode { .. } => {
            libav::transcode(&opened, &actual, plan, grant, temp, op)
        }
        ProcessingParameters::Remux {} => libav::remux(&opened, &actual, plan, grant, temp, op),
    }
    .map_err(|e| MediaError::new("processing_failed", "execute", e))?;
    drop(opened);
    op.check()?;
    op.progress("validating", None, None);
    let output = analysis::analyze(temp, None, op)?;
    let report = validation::validate(&actual, &output, plan)?;
    if source::stamp(Path::new(input))? != actual.source_stamp {
        return Err(MediaError::new(
            "source_changed",
            "validating",
            "处理期间源文件变化",
        ));
    }
    op.check()?;
    Ok(ExecutionResult {
        media: output,
        validation: report,
    })
}
