use crate::{Error, Result, events::Events, host, publisher, queries, repository as db, storage};
use fll_contracts::{
    ExecutionPlan, FllCommand, FllRequest, JSON_VERSION, MediaFacts, ResourceGrant,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
pub struct Config {
    pub engine: PathBuf,
    pub library: PathBuf,
    pub runtime: PathBuf,
    pub database: PathBuf,
}
pub struct Service {
    pub config: Config,
    pub events: Arc<Events>,
    pub token: String,
    pub ready: AtomicBool,
    pub stopped: AtomicBool,
    pub stopping: AtomicBool,
    tx: mpsc::SyncSender<Message>,
    pub query_slot: Mutex<()>,
}
#[derive(Clone)]
pub struct PreparedTask {
    pub analysis_id: String,
    pub plan: ExecutionPlan,
    pub output: String,
}
pub enum Preparation {
    None,
    Inputs(Vec<String>),
    Tasks(Vec<PreparedTask>),
}
enum Message {
    Command {
        name: String,
        body: Value,
        prepared: Preparation,
        reply: mpsc::Sender<Result<Value>>,
    },
    Internal(Internal, mpsc::Sender<Result<Value>>),
}
pub enum Internal {
    Temp {
        id: String,
        identity: Value,
    },
    Phase {
        id: String,
    },
    Validation {
        id: String,
        report: Value,
    },
    Publishing {
        id: String,
        path: String,
        size: u64,
        hash: String,
        identity: Value,
    },
    Finish {
        id: String,
        kind: String,
        result: Result<Value>,
    },
    Cleanup {
        id: String,
        error: Option<Error>,
    },
    Recovered(Vec<Value>),
}
impl Service {
    pub fn start(config: Config) -> (Arc<Self>, thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::sync_channel(64);
        let service = Arc::new(Self {
            config,
            events: Arc::new(Events::new()),
            token: crate::uid(),
            ready: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            tx,
            query_slot: Mutex::new(()),
        });
        let copy = service.clone();
        let handle = thread::spawn(move || run(copy, rx));
        (service, handle)
    }
    pub fn command(&self, name: &str, body: Value, prepared: Preparation) -> Result<Value> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Message::Command {
                name: name.into(),
                body,
                prepared,
                reply: tx,
            })
            .map_err(|_| Error::new("worker_stopped", "gateway", "后台已停止"))?;
        rx.recv_timeout(Duration::from_secs(15)).map_err(|_| {
            Error::new(
                "command_timeout",
                "gateway",
                "命令回执超时，可使用相同 request_id 重试",
            )
        })?
    }
    pub fn internal(&self, operation: Internal) -> Result<Value> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Message::Internal(operation, tx))
            .map_err(|_| Error::new("worker_stopped", "runtime", "后台已停止"))?;
        rx.recv_timeout(Duration::from_secs(30))
            .map_err(|_| Error::new("coordinator_timeout", "runtime", "状态提交超时"))?
    }
    pub fn query_fll(&self, operation: FllCommand) -> Result<Value> {
        let _guard = self
            .query_slot
            .try_lock()
            .map_err(|_| Error::new("query_busy", "planning", "方案查询忙，请稍后重试"))?;
        host::run(
            &self.config.engine,
            &self.config.library,
            &self.config.runtime.join("logs"),
            &FllRequest {
                schema_version: JSON_VERSION,
                operation_id: crate::uid(),
                operation,
            },
            &self.stopping,
            Some(Duration::from_secs(10)),
            |_, _| {},
        )
    }
}
struct Coordinator {
    db: Connection,
    service: Arc<Service>,
    active: Option<(String, Arc<AtomicBool>)>,
    recovering: bool,
    cleaning: HashSet<String>,
    health: Value,
}
fn run(service: Arc<Service>, rx: mpsc::Receiver<Message>) {
    let started = (|| -> Result<Coordinator> {
        let db = db::open(&service.config.database)?;
        let health = match service.query_fll(FllCommand::Capabilities {}) {
            Ok(v) => json!({"available":true,"capabilities":v}),
            Err(e) => json!({"available":false,"error":e}),
        };
        Ok(Coordinator {
            db,
            service: service.clone(),
            active: None,
            recovering: false,
            cleaning: HashSet::new(),
            health,
        })
    })();
    let mut coordinator = match started {
        Ok(c) => c,
        Err(e) => {
            service
                .events
                .lifecycle("degraded", json!({"available":false,"error":e}));
            while let Ok(message) = rx.recv() {
                match message {
                    Message::Command {
                        name, body, reply, ..
                    } if name == "shutdown" => {
                        let _ = reply
                            .send(Ok(json!({"request_id":body["request_id"],"accepted":true})));
                        break;
                    }
                    Message::Command { reply, .. } | Message::Internal(_, reply) => {
                        let _ = reply.send(Err(Error::new(
                            "database_unavailable",
                            "database",
                            "数据库无法打开，任务写入已停止",
                        )));
                    }
                }
            }
            service.stopped.store(true, Ordering::SeqCst);
            return;
        }
    };
    if let Err(e) = coordinator.recover(None) {
        coordinator.degrade(&e);
    }
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(message) => {
                let (result, reply) = match message {
                    Message::Command {
                        name,
                        body,
                        prepared,
                        reply,
                    } => (coordinator.command(&name, &body, prepared), reply),
                    Message::Internal(command, reply) => (coordinator.internal(command), reply),
                };
                if let Err(e) = &result
                    && e.code == "database_error"
                {
                    coordinator.degrade(e);
                }
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if service.stopping.load(Ordering::SeqCst)
            && coordinator.active.is_none()
            && !coordinator.recovering
            && coordinator.cleaning.is_empty()
        {
            break;
        }
        if service.ready.load(Ordering::SeqCst)
            && !service.stopping.load(Ordering::SeqCst)
            && coordinator.active.is_none()
            && let Err(e) = coordinator.schedule()
        {
            coordinator.degrade(&e);
        }
    }
    service.events.lifecycle("stopped", coordinator.health);
    service.stopped.store(true, Ordering::SeqCst);
}
impl Coordinator {
    fn degrade(&mut self, e: &Error) {
        self.service.ready.store(false, Ordering::SeqCst);
        self.service
            .events
            .lifecycle("degraded", json!({"available":false,"error":e}));
        let _ = self.db.execute("UPDATE worker_state SET paused=1", []);
    }
    fn notify(&self) -> Result<()> {
        let revision: i64 =
            self.db
                .query_row("SELECT revision FROM worker_state WHERE id=1", [], |r| {
                    r.get(0)
                })?;
        self.service.events.state_changed(revision);
        Ok(())
    }
    fn recover(&mut self, only: Option<&str>) -> Result<()> {
        if self.recovering {
            return Err(Error::new("reconcile_busy", "recovery", "已有结果正在核对"));
        }
        let rows = db::rows(
            &self.db,
            "SELECT e.*,o.final_path,o.size_bytes,o.sha256,o.file_identity_json FROM execution_attempt e LEFT JOIN output_artifact o ON o.attempt_id=e.id WHERE e.status IN ('running','validating','publishing','reconciling') AND (?1 IS NULL OR e.id=?1)",
            [only],
        )?;
        if only.is_none() {
            let tx = self.db.transaction()?;
            let queued = db::rows(
                &tx,
                "SELECT e.id,t.plan_json FROM execution_attempt e JOIN processing_task t ON t.id=e.task_id WHERE e.status='queued'",
                [],
            )?;
            for row in queued {
                if serde_json::from_value::<ExecutionPlan>(row["plan_json"].clone()).is_err()
                    || row["plan_json"]["schema_version"] != fll_contracts::PLAN_VERSION
                {
                    tx.execute("UPDATE execution_attempt SET status='interrupted',error_json=?1,finished_at=unixepoch() WHERE id=?2 AND status='queued'",
                        params![json!(Error::new("plan_required", "upgrade", "处理方案版本已变化，请重新分析并创建任务")).to_string(),row["id"].as_str()])?;
                }
            }
            tx.execute("UPDATE analysis SET status=CASE WHEN cancel_requested_at IS NULL THEN 'interrupted' ELSE 'canceled' END,error_json='{\"code\":\"interrupted\",\"message\":\"后台中断，请重新分析\"}' WHERE status='running'",[])?;
            tx.execute("UPDATE worker_state SET paused=1 WHERE EXISTS(SELECT 1 FROM analysis WHERE status='queued') OR EXISTS(SELECT 1 FROM execution_attempt WHERE status IN ('queued','running','validating','publishing','reconciling'))",[])?;
            db::bump(&tx)?;
            tx.commit()?;
            self.service
                .events
                .lifecycle("recovering", self.health.clone());
        }
        self.recovering = true;
        let service = self.service.clone();
        thread::spawn(move || {
            let outcomes = rows
                .iter()
                .map(|r| publisher::reconcile(r, &service.stopping))
                .collect();
            let _ = service.internal(Internal::Recovered(outcomes));
        });
        Ok(())
    }
    fn command(&mut self, name: &str, body: &Value, prepared: Preparation) -> Result<Value> {
        if name == "snapshot" {
            let mut value = self.service.events.snapshot();
            for (k, v) in queries::summary(&self.db)?.as_object().unwrap() {
                value[k] = v.clone();
            }
            return Ok(value);
        }
        if name == "receipt" {
            return Ok(queries::receipt(
                &self.db,
                body["operation"].as_str().unwrap_or(""),
                &body["body"],
            )?
            .unwrap_or(Value::Null));
        }
        if let Some(value) = queries::receipt(&self.db, name, body)? {
            return Ok(value);
        }
        if self.service.stopping.load(Ordering::SeqCst) {
            return Err(Error::new("worker_stopping", "command", "后台正在退出"));
        }
        if !self.service.ready.load(Ordering::SeqCst) && !matches!(name, "shutdown" | "reconcile") {
            return Err(Error::new(
                "worker_not_ready",
                "command",
                "后台正在恢复或组件不可用",
            ));
        }
        let mut signal = None;
        let mut reconcile_id = None;
        let tx = self.db.transaction()?;
        let mut response = json!({"request_id":body["request_id"],"accepted":true});
        match name {
            "analyze" => {
                let Preparation::Inputs(inputs) = prepared else {
                    return Err(Error::new("invalid_request", "command", "缺少预检查输入"));
                };
                let mut ids = vec![];
                for input in inputs {
                    let source = crate::uid();
                    let id = crate::uid();
                    tx.execute(
                        "INSERT INTO media_source VALUES(?1,?2,unixepoch())",
                        params![source, input],
                    )?;
                    let order = db::order(&tx)?;
                    tx.execute("INSERT INTO analysis(id,source_id,source_fingerprint,status,queue_order,created_at) VALUES(?1,?2,'null','queued',?3,unixepoch())",params![id,source,order])?;
                    ids.push(id);
                }
                response["analysis_ids"] = json!(ids);
            }
            "execute" => {
                let Preparation::Tasks(tasks) = prepared else {
                    return Err(Error::new("invalid_request", "command", "缺少处理方案"));
                };
                let mut tids = vec![];
                let mut aids = vec![];
                for p in tasks {
                    let a = queries::analysis(&tx, &p.analysis_id)?;
                    if a["status"] != "succeeded" || a["facts_digest"] != p.plan.facts_digest {
                        return Err(Error::new("analysis_stale", "command", "分析结果失效"));
                    }
                    let task = crate::uid();
                    tx.execute("INSERT INTO processing_task(id,analysis_id,preset_json,output_directory,created_at,intent_json,plan_json,plan_digest) VALUES(?1,?2,?3,?4,unixepoch(),?5,?6,?7)",params![task,p.analysis_id,serde_json::to_string(&p.plan.selection)?,p.output,serde_json::to_string(&p.plan.intent)?,serde_json::to_string(&p.plan)?,p.plan.plan_digest])?;
                    aids.push(enqueue(&tx, &task)?);
                    tids.push(task);
                }
                response["task_ids"] = json!(tids);
                response["attempt_ids"] = json!(aids);
            }
            "retry" => {
                let task = text(body, "task_id")?;
                let Preparation::Tasks(plans) = prepared else {
                    return Err(Error::new("plan_required", "command", "缺少重试核验"));
                };
                let saved: String = tx.query_row(
                    "SELECT COALESCE(plan_digest,'') FROM processing_task WHERE id=?1",
                    [task],
                    |r| r.get(0),
                )?;
                if plans.first().is_none_or(|p| p.plan.plan_digest != saved) {
                    return Err(Error::new("plan_stale", "command", "原任务需要重新规划"));
                }
                let status:String=tx.query_row("SELECT status FROM execution_attempt WHERE task_id=?1 ORDER BY attempt_number DESC LIMIT 1",[task],|r|r.get(0))?;
                if !matches!(status.as_str(), "failed" | "canceled" | "interrupted") {
                    return Err(Error::new(
                        "task_active",
                        "command",
                        "仅失败、取消或中断任务可重试",
                    ));
                }
                response["task_ids"] = json!([task]);
                response["attempt_ids"] = json!([enqueue(&tx, task)?]);
            }
            "cancel" => {
                let analysis = body["analysis_id"].as_str();
                let attempt = body["attempt_id"].as_str();
                if analysis.is_some() == attempt.is_some() {
                    return Err(Error::new(
                        "cancel_target",
                        "command",
                        "指定一个 analysis_id 或 attempt_id",
                    ));
                }
                let id = analysis.or(attempt).unwrap();
                let table = if analysis.is_some() {
                    "analysis"
                } else {
                    "execution_attempt"
                };
                let status: Option<String> = tx
                    .query_row(
                        &format!("SELECT status FROM {table} WHERE id=?1"),
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let status =
                    status.ok_or_else(|| Error::new("not_found", "command", "取消目标不存在"))?;
                if matches!(status.as_str(), "publishing" | "reconciling") {
                    response["accepted"] = json!(false);
                    response["result"] = json!(status);
                } else if matches!(status.as_str(), "queued" | "running" | "validating") {
                    tx.execute(&format!("UPDATE {table} SET cancel_requested_at=unixepoch(),status=CASE WHEN status='queued' THEN 'canceled' ELSE status END WHERE id=?1"),[id])?;
                    if attempt.is_some() && status == "queued" {
                        tx.execute(
                            "UPDATE execution_attempt SET finished_at=unixepoch() WHERE id=?1",
                            [id],
                        )?;
                    }
                    signal = Some(id.to_string());
                    response["result"] = json!("accepted");
                } else {
                    response["accepted"] = json!(false);
                    response["result"] = json!("already_terminal");
                }
            }
            "queue/pause" | "queue/resume" => {
                tx.execute("UPDATE worker_state SET paused=?1", [name == "queue/pause"])?;
            }
            "reconcile" => {
                let id = text(body, "attempt_id")?;
                let status: String = tx.query_row(
                    "SELECT status FROM execution_attempt WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )?;
                if status != "reconciling" || self.recovering {
                    return Err(Error::new(
                        "reconcile_state",
                        "command",
                        "该执行当前无法重新核对",
                    ));
                }
                response["reconcile_id"] = json!(crate::uid());
                reconcile_id = Some(id.to_owned());
            }
            "shutdown" => {
                tx.execute("UPDATE worker_state SET paused=1", [])?;
                if let Some((id, _)) = &self.active {
                    let state: Option<String> = tx
                        .query_row(
                            "SELECT status FROM execution_attempt WHERE id=?1",
                            [id],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if state.as_deref() != Some("publishing") {
                        tx.execute("UPDATE execution_attempt SET cancel_requested_at=unixepoch() WHERE id=?1",[id])?;
                        tx.execute(
                            "UPDATE analysis SET cancel_requested_at=unixepoch() WHERE id=?1",
                            [id],
                        )?;
                        signal = Some(id.clone());
                    }
                }
            }
            _ => return Err(Error::new("unknown_command", "command", "未知命令")),
        }
        tx.execute(
            "INSERT INTO request_receipts VALUES(?1,?2,?3)",
            params![
                text(body, "request_id")?,
                queries::request_digest(name, body),
                response.to_string()
            ],
        )?;
        db::bump(&tx)?;
        tx.commit()?;
        if signal.is_some() {
            publisher::fault("cancel_committed");
        }
        if let Some(id) = signal
            && let Some((active, cancel)) = &self.active
            && *active == id
        {
            cancel.store(true, Ordering::SeqCst);
        }
        if name == "shutdown" {
            self.service.stopping.store(true, Ordering::SeqCst);
            self.service
                .events
                .lifecycle("stopping", self.health.clone());
        }
        if let Some(id) = reconcile_id {
            self.recover(Some(&id))?;
        }
        self.notify()?;
        Ok(response)
    }
    fn internal(&mut self, message: Internal) -> Result<Value> {
        match message {
            Internal::Temp { id, identity } => {
                let tx = self.db.transaction()?;
                tx.execute("UPDATE execution_attempt SET temp_identity_json=?1,cleanup_state='pending' WHERE id=?2 AND status='running'",params![identity.to_string(),id])?;
                let canceled: bool = tx.query_row(
                    "SELECT cancel_requested_at IS NOT NULL FROM execution_attempt WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )?;
                tx.commit()?;
                if canceled {
                    return Err(Error::new("canceled", "execute", "任务已取消"));
                }
            }
            Internal::Phase { id } => {
                let tx = self.db.transaction()?;
                if tx.execute("UPDATE execution_attempt SET status='validating' WHERE id=?1 AND status='running'",[id])?>0{db::bump(&tx)?;}
                tx.commit()?;
                self.notify()?;
            }
            Internal::Validation { id, report } => {
                let tx = self.db.transaction()?;
                tx.execute("UPDATE execution_attempt SET status='validating',validation_json=?1 WHERE id=?2 AND status IN ('running','validating')",params![report.to_string(),id])?;
                db::bump(&tx)?;
                tx.commit()?;
                self.notify()?;
            }
            Internal::Publishing {
                id,
                path,
                size,
                hash,
                identity,
            } => {
                let tx = self.db.transaction()?;
                let(status,canceled):(String,bool)=tx.query_row("SELECT status,cancel_requested_at IS NOT NULL FROM execution_attempt WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
                if status != "publishing" && (status != "validating" || canceled) {
                    return Err(Error::new(
                        "canceled",
                        "publishing",
                        "发布前任务已取消或失效",
                    ));
                }
                tx.execute("INSERT INTO output_artifact(id,attempt_id,final_path,status,size_bytes,sha256,file_identity_json) VALUES(?1,?2,?3,'publishing',?4,?5,?6) ON CONFLICT(attempt_id) DO UPDATE SET final_path=excluded.final_path",params![crate::uid(),id,path,size as i64,hash,identity.to_string()])?;
                tx.execute(
                    "UPDATE execution_attempt SET status='publishing' WHERE id=?1",
                    [&id],
                )?;
                db::bump(&tx)?;
                tx.commit()?;
                self.notify()?;
            }
            Internal::Finish { id, kind, result } => {
                let tx = self.db.transaction()?;
                let table = if kind == "analyze" {
                    "analysis"
                } else {
                    "execution_attempt"
                };
                let (old, cancel): (String, bool) = tx.query_row(
                    &format!(
                        "SELECT status,cancel_requested_at IS NOT NULL FROM {table} WHERE id=?1"
                    ),
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let mut status = if result.is_ok() {
                    "succeeded"
                } else {
                    "failed"
                };
                let mut error = result.as_ref().err().cloned();
                if cancel && old != "publishing" {
                    status = "canceled";
                    error = Some(Error::new("canceled", "operation", "任务已取消"));
                } else if result.is_err() && old == "publishing" {
                    status = "reconciling";
                }
                if kind == "analyze" {
                    let facts = if status == "succeeded" {
                        result.as_ref().ok()
                    } else {
                        None
                    };
                    tx.execute("UPDATE analysis SET status=?1,media_json=?2,error_json=?3,source_fingerprint=?4,facts_schema=?5,analyzer_version=?6,facts_digest=?7 WHERE id=?8",params![status,facts.map(Value::to_string),error.as_ref().map(|e|json!(e).to_string()),facts.map(|f|f["source_stamp"].to_string()).unwrap_or("null".into()),facts.and_then(|f|f["schema_version"].as_i64()),facts.and_then(|f|f["analyzer_version"].as_str()),facts.and_then(|f|f["facts_digest"].as_str()),id])?;
                } else {
                    tx.execute("UPDATE execution_attempt SET status=?1,error_json=?2,finished_at=CASE WHEN ?1='reconciling' THEN NULL ELSE unixepoch() END WHERE id=?3",params![status,error.as_ref().map(|e|json!(e).to_string()),id])?;
                    tx.execute(
                        "UPDATE output_artifact SET status=?1 WHERE attempt_id=?2",
                        params![
                            if status == "succeeded" {
                                "succeeded"
                            } else if status == "reconciling" {
                                "publishing"
                            } else {
                                "failed"
                            },
                            id
                        ],
                    )?;
                }
                db::bump(&tx)?;
                tx.commit()?;
                publisher::fault("terminal_committed");
                self.active = None;
                if kind != "analyze" && status != "reconciling" {
                    self.cleaning.insert(id);
                }
                self.service.events.clear();
                self.notify()?;
                return Ok(json!({"status":status}));
            }
            Internal::Cleanup { id, error } => {
                self.cleaning.remove(&id);
                self.db.execute(
                    "UPDATE execution_attempt SET cleanup_state=?1 WHERE id=?2",
                    params![if error.is_some() { "error" } else { "done" }, id],
                )?;
                if let Some(error) = error {
                    eprintln!("cleanup {id}: {error}");
                }
            }
            Internal::Recovered(outcomes) => {
                let tx = self.db.transaction()?;
                for r in outcomes {
                    let status = r["status"].as_str().unwrap();
                    tx.execute("UPDATE execution_attempt SET status=?1,error_json=?2,finished_at=CASE WHEN ?1='reconciling' THEN NULL ELSE unixepoch() END WHERE id=?3 AND status IN ('running','validating','publishing','reconciling')",params![status,if r["error"].is_null(){None}else{Some(r["error"].to_string())},r["id"].as_str()])?;
                    tx.execute(
                        "UPDATE output_artifact SET status=?1 WHERE attempt_id=?2",
                        params![
                            if status == "succeeded" {
                                "succeeded"
                            } else if status == "reconciling" {
                                "publishing"
                            } else {
                                "failed"
                            },
                            r["id"].as_str()
                        ],
                    )?;
                }
                db::bump(&tx)?;
                tx.commit()?;
                self.recovering = false;
                let ready = self.health["available"] == true;
                self.service.ready.store(ready, Ordering::SeqCst);
                self.service.events.lifecycle(
                    if self.service.stopping.load(Ordering::SeqCst) {
                        "stopping"
                    } else if ready {
                        "ready"
                    } else {
                        "degraded"
                    },
                    self.health.clone(),
                );
                self.notify()?;
                let mut cleanup = db::rows(
                    &self.db,
                    "SELECT id,temp_path,temp_identity_json FROM execution_attempt WHERE status IN ('succeeded','failed','canceled','interrupted') AND temp_path IS NOT NULL AND cleanup_state!='done'",
                    [],
                )?;
                cleanup.retain(|row| self.cleaning.insert(row["id"].as_str().unwrap().to_owned()));
                let service = self.service.clone();
                thread::spawn(move || {
                    for row in cleanup {
                        let Some(path) = row["temp_path"].as_str() else {
                            continue;
                        };
                        let id = row["id"].as_str().unwrap();
                        let error = storage::cleanup(
                            id,
                            std::path::Path::new(path),
                            &row["temp_identity_json"],
                        )
                        .err();
                        let _ = service.internal(Internal::Cleanup {
                            id: id.into(),
                            error,
                        });
                    }
                });
            }
        }
        Ok(json!({"accepted":true}))
    }
    fn schedule(&mut self) -> Result<()> {
        let paused: bool = self
            .db
            .query_row("SELECT paused FROM worker_state", [], |r| r.get(0))?;
        if paused {
            return Ok(());
        }
        let next:Option<(String,String)>=self.db.query_row("SELECT kind,id FROM (SELECT 'analyze' AS kind,id,queue_order FROM analysis WHERE status='queued' UNION ALL SELECT 'execute',id,queue_order FROM execution_attempt WHERE status='queued') ORDER BY queue_order LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((kind, id)) = next else {
            return Ok(());
        };
        let job = if kind == "analyze" {
            queries::analysis(&self.db, &id)?
        } else {
            db::rows(&self.db,"SELECT e.*,t.plan_json,t.output_directory,a.media_json,s.canonical_path AS input FROM execution_attempt e JOIN processing_task t ON t.id=e.task_id JOIN analysis a ON a.id=t.analysis_id JOIN media_source s ON s.id=a.source_id WHERE e.id=?1",[&id])?.remove(0)
        };
        let cached = if kind == "analyze" {
            db::rows(&self.db,"SELECT a.media_json FROM analysis a JOIN media_source s ON s.id=a.source_id WHERE s.canonical_path=?1 AND a.status='succeeded' AND a.facts_schema=?2 ORDER BY a.queue_order DESC LIMIT 1",params![job["input"].as_str(),fll_contracts::FACTS_VERSION])?.pop().map(|v|v["media_json"].clone())
        } else {
            None
        };
        let temp = if kind == "execute" {
            let plan: ExecutionPlan = serde_json::from_value(job["plan_json"].clone())?;
            let extension = &plan.output.extension;
            if extension.is_empty() || !extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(Error::new("invalid_output", "execute", "输出扩展名无效"));
            }
            Some(
                PathBuf::from(job["output_directory"].as_str().unwrap())
                    .join(format!(".framelean-{id}.partial.{extension}")),
            )
        } else {
            None
        };
        let tx = self.db.transaction()?;
        if kind == "analyze" {
            tx.execute(
                "UPDATE analysis SET status='running' WHERE id=?1 AND status='queued'",
                [&id],
            )?;
        } else {
            tx.execute("UPDATE execution_attempt SET status='running',temp_path=?1,cleanup_state='pending',started_at=unixepoch() WHERE id=?2 AND status='queued'",params![temp.as_ref().and_then(|p|p.to_str()),id])?;
        }
        db::bump(&tx)?;
        tx.commit()?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.active = Some((id.clone(), cancel.clone()));
        self.notify()?;
        let service = self.service.clone();
        thread::spawn(move || execute_job(service, kind, id, job, cached, temp, cancel));
        Ok(())
    }
}
pub fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && !s.contains('\0'))
        .ok_or_else(|| Error::new("invalid_request", "request", format!("缺少有效 {key}")))
}
fn enqueue(db: &Connection, task: &str) -> Result<String> {
    let id = crate::uid();
    let order = db::order(db)?;
    db.execute("INSERT INTO execution_attempt(id,task_id,attempt_number,queue_order,status) SELECT ?1,?2,COALESCE(MAX(attempt_number),0)+1,?3,'queued' FROM execution_attempt WHERE task_id=?2",params![id,task,order])?;
    Ok(id)
}
fn execute_job(
    service: Arc<Service>,
    kind: String,
    id: String,
    job: Value,
    cached: Option<Value>,
    temp: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
) {
    let mut identity = Value::Null;
    let result = (|| -> Result<Value> {
        let input = text(&job, "input")?.to_owned();
        let operation = if kind == "analyze" {
            FllCommand::Analyze {
                input: input.clone(),
                cached: cached.and_then(|v| serde_json::from_value(v).ok()),
            }
        } else {
            let path = temp.as_ref().unwrap();
            storage::directory(path.parent().unwrap())?;
            let file = OpenOptions::new().write(true).create_new(true).open(path)?;
            identity = storage::identity(&file)?;
            drop(file);
            service.internal(Internal::Temp {
                id: id.clone(),
                identity: identity.clone(),
            })?;
            FllCommand::Execute {
                input: input.clone(),
                temp: path.to_string_lossy().into_owned(),
                media: serde_json::from_value::<MediaFacts>(job["media_json"].clone())?,
                plan: serde_json::from_value::<ExecutionPlan>(job["plan_json"].clone())
                    .map_err(|_| Error::new("plan_required", "execute", "该任务需要重新规划"))?,
                grant: ResourceGrant::default(),
            }
        };
        let mut validating = false;
        let result = host::run(
            &service.config.engine,
            &service.config.library,
            &service.config.runtime.join("logs"),
            &FllRequest {
                schema_version: JSON_VERSION,
                operation_id: id.clone(),
                operation,
            },
            &cancel,
            None,
            |pid, event| {
                service.events.progress(&id, pid, json!(event));
                if event.state == "validating" && !validating {
                    validating = true;
                    let _ = service.internal(Internal::Phase { id: id.clone() });
                    publisher::fault("validating");
                }
            },
        )?;
        if let Some(path) = &temp {
            service.internal(Internal::Validation {
                id: id.clone(),
                report: result["validation"].clone(),
            })?;
            publisher::fault("validating");
            publisher::publish(
                &service,
                &id,
                path,
                &input,
                &serde_json::from_value(job["plan_json"].clone())?,
                &cancel,
            )
        } else {
            Ok(result)
        }
    })();
    let finished = service.internal(Internal::Finish {
        id: id.clone(),
        kind,
        result,
    });
    if let Ok(response) = finished
        && response["status"] != "reconciling"
        && let Some(path) = temp
    {
        let error = storage::cleanup(&id, &path, &identity).err();
        let _ = service.internal(Internal::Cleanup { id, error });
    }
}
