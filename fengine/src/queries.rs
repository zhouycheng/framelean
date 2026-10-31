use crate::{Error, Result, repository as db};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
pub fn analysis(c: &Connection, id: &str) -> Result<Value> {
    db::rows(c,"SELECT a.*,s.canonical_path AS input FROM analysis a JOIN media_source s ON s.id=a.source_id WHERE a.id=?1",[id])?.pop().ok_or_else(||Error::new("not_found","query","分析不存在"))
}
pub fn task(c: &Connection, id: &str, limit: i64, cursor: i64) -> Result<Value> {
    let task=db::rows(c,"SELECT t.*,a.source_fingerprint,a.media_json,s.canonical_path AS input FROM processing_task t JOIN analysis a ON a.id=t.analysis_id JOIN media_source s ON s.id=a.source_id WHERE t.id=?1",[id])?.pop().ok_or_else(||Error::new("not_found","query","任务不存在"))?;
    let attempts = db::rows(
        c,
        "SELECT * FROM execution_attempt WHERE task_id=?1 AND queue_order<?2 ORDER BY queue_order DESC LIMIT ?3",
        params![id, cursor, limit],
    )?;
    let mut outputs = vec![];
    for e in &attempts {
        outputs.extend(db::rows(
            c,
            "SELECT * FROM output_artifact WHERE attempt_id=?1",
            [e["id"].as_str().unwrap_or("")],
        )?);
    }
    let total: i64 = c.query_row(
        "SELECT count(*) FROM execution_attempt WHERE task_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let latest = db::rows(
        c,
        "SELECT * FROM execution_attempt WHERE task_id=?1 ORDER BY attempt_number DESC LIMIT 1",
        [id],
    )?
    .pop();
    Ok(
        json!({"task":task,"attempts":attempts,"outputs":outputs,"latest_attempt":latest,"attempt_count":total,"next_cursor":if attempts.len()==limit as usize{attempts.last().map(|e|e["queue_order"].clone())}else{None}}),
    )
}
pub fn list(c: &Connection, kind: &str, limit: i64, cursor: i64) -> Result<Value> {
    if kind == "analyses" {
        let items = db::rows(
            c,
            "SELECT a.*,s.canonical_path AS input FROM analysis a JOIN media_source s ON s.id=a.source_id WHERE a.queue_order<?1 ORDER BY a.queue_order DESC LIMIT ?2",
            params![cursor, limit],
        )?;
        return Ok(
            json!({"items":items,"next_cursor":if items.len()==limit as usize{items.last().map(|e|e["queue_order"].clone())}else{None}}),
        );
    }
    let tasks = db::rows(
        c,
        "SELECT t.*,a.source_fingerprint,s.canonical_path AS input,MIN(e.queue_order) AS list_order FROM processing_task t JOIN analysis a ON a.id=t.analysis_id JOIN media_source s ON s.id=a.source_id JOIN execution_attempt e ON e.task_id=t.id GROUP BY t.id HAVING list_order<?1 ORDER BY list_order DESC LIMIT ?2",
        params![cursor, limit],
    )?;
    let mut attempts = vec![];
    let mut outputs = vec![];
    for t in &tasks {
        let es = db::rows(
            c,
            "SELECT * FROM execution_attempt WHERE task_id=?1 ORDER BY attempt_number DESC LIMIT 1",
            [t["id"].as_str().unwrap_or("")],
        )?;
        for e in &es {
            outputs.extend(db::rows(
                c,
                "SELECT * FROM output_artifact WHERE attempt_id=?1",
                [e["id"].as_str().unwrap_or("")],
            )?);
        }
        attempts.extend(es);
    }
    Ok(
        json!({"items":tasks,"attempts":attempts,"outputs":outputs,"next_cursor":if tasks.len()==limit as usize{tasks.last().map(|e|e["list_order"].clone())}else{None}}),
    )
}
pub fn receipt(c: &Connection, name: &str, body: &Value) -> Result<Option<Value>> {
    let rid = body["request_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() < 128)
        .ok_or_else(|| Error::new("request_id", "request", "request_id 需要 1–127 字符"))?;
    let old: Option<(String, String)> = c
        .query_row(
            "SELECT request_json,response_json FROM request_receipts WHERE request_id=?1",
            [rid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((digest, response)) = old {
        if digest != request_digest(name, body) {
            return Err(Error::new(
                "request_conflict",
                "request",
                "request_id 已用于不同请求",
            ));
        }
        return Ok(Some(serde_json::from_str(&response)?));
    }
    Ok(None)
}
pub fn request_digest(name: &str, body: &Value) -> String {
    fll_contracts::digest(&(fll_contracts::HTTP_VERSION, name, body))
}
pub fn summary(c: &Connection) -> Result<Value> {
    let (paused, revision): (bool, i64) = c.query_row(
        "SELECT paused,revision FROM worker_state WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let queued:i64=c.query_row("SELECT (SELECT count(*) FROM analysis WHERE status='queued')+(SELECT count(*) FROM execution_attempt WHERE status='queued')",[],|r|r.get(0))?;
    let reconciling: i64 = c.query_row(
        "SELECT count(*) FROM execution_attempt WHERE status='reconciling'",
        [],
        |r| r.get(0),
    )?;
    Ok(
        json!({"paused":paused,"revision":revision,"queue":{"queued":queued,"reconciling":reconciling}}),
    )
}
