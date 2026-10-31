use crate::{
    Error, Result,
    coordinator::{Preparation, PreparedTask, Service, text},
    queries, repository, storage,
};
use fll_contracts::{ExecutionPlan, FllCommand, Intent, MediaFacts, Selection};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{Arc, atomic::Ordering},
};
use tiny_http::{Header, Request, Response, StatusCode};
pub fn reply(req: Request, status: u16, value: Value) {
    let _ = req.respond(
        Response::from_string(value.to_string())
            .with_status_code(StatusCode(status))
            .with_header(
                Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap(),
            )
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
    );
}
fn error(req: Request, rid: &str, e: Error) {
    let code = match e.code.as_str() {
        "not_found" => 404,
        "query_busy" | "worker_not_ready" | "worker_stopping" | "database_unavailable" => 503,
        "request_conflict" | "plan_stale" | "source_changed" | "task_active" => 409,
        _ => 400,
    };
    reply(req, code, json!({"request_id":rid,"error":e}));
}
pub fn handle(service: Arc<Service>, mut req: Request) {
    if req.headers().iter().any(|h| h.field.equiv("Origin")) {
        reply(
            req,
            403,
            json!({"error":Error::new("origin_denied","gateway","仅允许本地原生客户端")}),
        );
        return;
    }
    if !req.headers().iter().any(|h| {
        h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {}", service.token)
    }) {
        reply(
            req,
            401,
            json!({"error":Error::new("unauthorized","gateway","凭证无效")}),
        );
        return;
    }
    let url = req.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let Some(name) = path.strip_prefix("/v4/") else {
        reply(
            req,
            404,
            json!({"error":Error::new("protocol_mismatch","gateway","需要 Worker 协议 v4")}),
        );
        return;
    };
    if req.method().as_str() == "GET" {
        if name == "events" {
            events(service, req, query);
            return;
        }
        let rid = crate::uid();
        let result = (|| -> Result<Value> {
            if name == "health" {
                return Ok(service.events.snapshot());
            }
            if name == "snapshot" {
                return service.command("snapshot", json!({}), Preparation::None);
            }
            let limit = parameter(query, "limit")
                .unwrap_or("50")
                .parse::<i64>()
                .map_err(|_| Error::new("pagination", "query", "limit 无效"))?;
            if !(1..=200).contains(&limit) {
                return Err(Error::new("pagination", "query", "limit 需要 1–200"));
            }
            let cursor = parameter(query, "cursor")
                .map(|v| v.parse::<i64>())
                .transpose()
                .map_err(|_| Error::new("pagination", "query", "cursor 无效"))?
                .unwrap_or(i64::MAX);
            let c = repository::read(&service.config.database)?;
            if let Some(id) = name.strip_prefix("analyses/") {
                queries::analysis(&c, id)
            } else if let Some(id) = name.strip_prefix("tasks/") {
                queries::task(&c, id, limit, cursor)
            } else if matches!(name, "tasks" | "analyses") {
                queries::list(&c, name, limit, cursor)
            } else {
                Err(Error::new("not_found", "query", "接口不存在"))
            }
        })();
        match result {
            Ok(mut value) => {
                value["request_id"] = json!(rid);
                reply(req, 200, value);
            }
            Err(e) => error(req, &rid, e),
        }
        return;
    }
    if req.method().as_str() != "POST" {
        reply(
            req,
            405,
            json!({"error":Error::new("method","gateway","需要 GET 或 POST")}),
        );
        return;
    }
    let mut bytes = vec![];
    if req.as_reader().take(65537).read_to_end(&mut bytes).is_err() || bytes.len() > 65536 {
        reply(
            req,
            413,
            json!({"error":Error::new("request_size","gateway","请求超过 64 KiB")}),
        );
        return;
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(Value::Object(v)) => Value::Object(v),
        _ => {
            error(
                req,
                "",
                Error::new("invalid_json", "gateway", "请求需要 JSON 对象"),
            );
            return;
        }
    };
    let rid = body["request_id"].as_str().unwrap_or("").to_owned();
    let result = (|| -> Result<Value> {
        if matches!(name, "options" | "plan") {
            return evaluate(&service, name, &body);
        }
        let prior = service.command(
            "receipt",
            json!({"operation":name,"body":body}),
            Preparation::None,
        )?;
        if !prior.is_null() {
            return Ok(prior);
        }
        let prepared = match name {
            "analyze" => {
                let inputs = array(&body, "inputs")?;
                let mut paths = vec![];
                for value in inputs {
                    let p = value
                        .as_str()
                        .filter(|s| !s.contains('\0'))
                        .ok_or_else(|| Error::new("path", "gateway", "输入需要文件路径"))?;
                    if !Path::new(p).is_absolute() {
                        return Err(Error::new("path", "gateway", "输入路径需要绝对路径"));
                    }
                    paths.push(
                        std::fs::canonicalize(p)
                            .unwrap_or_else(|_| p.into())
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
                Preparation::Inputs(paths)
            }
            "execute" => {
                let output = storage::directory(Path::new(text(&body, "output_directory")?))?;
                let c = repository::read(&service.config.database)?;
                let mut tasks = vec![];
                let mut errors = vec![];
                for item in array(&body, "items")? {
                    let aid = text(item, "analysis_id")?;
                    let result = (|| -> Result<PreparedTask> {
                        let a = queries::analysis(&c, aid)?;
                        let media = media(&a)?;
                        source_observation(&a, &media)?;
                        let intent = intent(item)?;
                        let selection: Selection =
                            serde_json::from_value(item["selection"].clone())?;
                        let plan: ExecutionPlan =
                            serde_json::from_value(service.query_fll(FllCommand::Plan {
                                media,
                                intent,
                                selection,
                            })?)?;
                        if text(item, "expected_plan_digest")? != plan.plan_digest {
                            return Err(Error::new(
                                "plan_stale",
                                "planning",
                                "处理方案已变化，请重新选择",
                            ));
                        }
                        Ok(PreparedTask {
                            analysis_id: aid.into(),
                            plan,
                            output: output.clone(),
                        })
                    })();
                    match result {
                        Ok(task) => tasks.push(task),
                        Err(e) => errors.push(json!({"analysis_id":aid,"error":e})),
                    }
                }
                if !errors.is_empty() {
                    return Err(Error::new(
                        "batch_invalid",
                        "planning",
                        "批次包含无效方案，未创建任务",
                    )
                    .details(json!({"items":errors})));
                }
                Preparation::Tasks(tasks)
            }
            "retry" => {
                let c = repository::read(&service.config.database)?;
                let task = queries::task(&c, text(&body, "task_id")?, 1, i64::MAX)?;
                let t = &task["task"];
                let old: ExecutionPlan =
                    serde_json::from_value(t["plan_json"].clone()).map_err(|_| {
                        Error::new("plan_required", "planning", "旧任务需要重新分析与创建方案")
                    })?;
                let media: MediaFacts = serde_json::from_value(t["media_json"].clone())?;
                source_observation(t, &media)?;
                let plan: ExecutionPlan =
                    serde_json::from_value(service.query_fll(FllCommand::Plan {
                        media,
                        intent: old.intent.clone(),
                        selection: old.selection.clone(),
                    })?)?;
                if plan != old {
                    return Err(Error::new(
                        "plan_stale",
                        "planning",
                        "原方案已失效，需要创建新任务",
                    ));
                }
                Preparation::Tasks(vec![PreparedTask {
                    analysis_id: text(t, "analysis_id")?.into(),
                    plan,
                    output: storage::directory(Path::new(text(t, "output_directory")?))?,
                }])
            }
            _ => Preparation::None,
        };
        service.command(name, body.clone(), prepared)
    })();
    match result {
        Ok(mut value) => {
            if !rid.is_empty() {
                value["request_id"] = json!(rid);
            }
            reply(req, 200, value);
        }
        Err(e) => error(req, &rid, e),
    }
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v[key]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 100)
        .ok_or_else(|| Error::new("batch_size", "request", format!("{key} 需要 1–100 项")))
}
fn intent(v: &Value) -> Result<Intent> {
    if v["intent"].is_null() {
        Ok(Intent::default())
    } else {
        Ok(serde_json::from_value(v["intent"].clone())?)
    }
}
fn media(a: &Value) -> Result<MediaFacts> {
    if a["status"] != "succeeded" {
        return Err(Error::new(
            "analysis_pending",
            "planning",
            "分析尚未成功完成",
        ));
    }
    serde_json::from_value(a["media_json"].clone())
        .map_err(|_| Error::new("analysis_stale", "planning", "分析格式需要更新"))
}
fn source_observation(a: &Value, facts: &MediaFacts) -> Result<()> {
    let m = std::fs::metadata(text(a, "input")?)?;
    let time = m
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Error::new("source_changed", "preflight", e))?
        .as_nanos()
        .to_string();
    if m.len() != facts.source_stamp.size || time != facts.source_stamp.modified_ns {
        return Err(Error::new(
            "source_changed",
            "preflight",
            "源文件发生变化，请重新分析",
        ));
    }
    Ok(())
}
fn evaluate(service: &Service, name: &str, body: &Value) -> Result<Value> {
    let format = body
        .get("format")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_str()
                .ok_or_else(|| Error::new("invalid_format", "planning", "目标格式需要字符串"))
        })
        .transpose()?;
    if !service.ready.load(Ordering::SeqCst) {
        return Err(Error::new("worker_not_ready", "planning", "后台尚未就绪"));
    }
    let c = repository::read(&service.config.database)?;
    let mut items = vec![];
    let mut common: Option<Vec<Selection>> = None;
    let mut presets = json!([]);
    for id in array(body, "analysis_ids")? {
        let aid = id
            .as_str()
            .ok_or_else(|| Error::new("analysis_id", "planning", "分析 ID 无效"))?;
        let result = (|| -> Result<Value> {
            let facts = media(&queries::analysis(&c, aid)?)?;
            let mut options = service.query_fll(FllCommand::Options {
                media: facts.clone(),
                intent: intent(body)?,
            })?;
            let mut choices: Vec<Selection> =
                serde_json::from_value(options["available_choices"].clone())?;
            if let Some(format) = format {
                choices.retain(|c| c.container == format);
            }
            options["available_choices"] = json!(choices);
            options["default_selection"] = json!(choices.first());
            presets = options["presets"].clone();
            if let Some(shared) = &mut common {
                shared.retain(|s| choices.contains(s));
            } else {
                common = Some(choices);
            }
            if name == "options" {
                Ok(options)
            } else {
                let selection: Selection = serde_json::from_value(body["selection"].clone())?;
                service.query_fll(FllCommand::Plan {
                    media: facts,
                    intent: intent(body)?,
                    selection,
                })
            }
        })();
        match result {
            Ok(v) => items.push(json!({"analysis_id":aid,"result":v})),
            Err(e) => {
                common = Some(vec![]);
                items.push(json!({"analysis_id":aid,"error":e}));
            }
        }
    }
    let common = common.unwrap_or_default();
    Ok(
        json!({"request_id":body["request_id"].as_str().map(str::to_owned).unwrap_or_else(crate::uid),"items":items,"default_selection":common.first(),"available_choices":common,"presets":presets}),
    )
}
fn parameter<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|s| s.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}
fn events(service: Arc<Service>, req: Request, query: &str) {
    let mut cursor = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Last-Event-ID"))
        .map(|h| h.value.as_str().to_owned())
        .or_else(|| parameter(query, "after").map(str::to_owned));
    let mut writer = req.into_writer();
    let _ = (|| -> std::io::Result<()> {
        writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")?;
        while !service.stopped.load(Ordering::SeqCst) {
            match service.events.after(cursor.as_deref()) {
                Some(events) => {
                    for event in events {
                        cursor = event["event_id"].as_str().map(str::to_owned);
                        chunk(
                            &mut writer,
                            &format!("id: {}\ndata: {}\n\n", cursor.as_deref().unwrap(), event),
                        )?;
                    }
                }
                None => {
                    let snapshot = service.events.snapshot();
                    cursor = snapshot["event_id"].as_str().map(str::to_owned);
                    chunk(
                        &mut writer,
                        &format!(
                            "id: {}\ndata: {}\n\n",
                            cursor.as_deref().unwrap(),
                            json!({"type":"reset","data":snapshot})
                        ),
                    )?;
                }
            }
            chunk(&mut writer, ": heartbeat\n\n")?;
            service.events.wait();
        }
        writer.write_all(b"0\r\n\r\n")?;
        Ok(())
    })();
}
fn chunk(w: &mut impl Write, s: &str) -> std::io::Result<()> {
    write!(w, "{:x}\r\n{}\r\n", s.len(), s)?;
    w.flush()
}
