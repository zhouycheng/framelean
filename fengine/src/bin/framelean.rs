use clap::{Parser, Subcommand};
use framelean_engine::{Error, Result, client::Client};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[derive(Parser)]
#[command(version, about = "FrameLean 本地媒体处理")]
struct Args {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    request_id: Option<String>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Doctor,
    Analyze {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        wait: bool,
    },
    Options {
        #[arg(required = true)]
        analysis_ids: Vec<String>,
    },
    Plan {
        #[arg(required = true)]
        analysis_ids: Vec<String>,
        #[arg(long)]
        preset: Option<String>,
    },
    Compress {
        files: Vec<PathBuf>,
        #[arg(long = "analysis-id", conflicts_with = "files")]
        analysis_ids: Vec<String>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    Tasks {
        #[arg(long, default_value_t = 50)]
        limit: u16,
        #[arg(long)]
        cursor: Option<i64>,
    },
    Task {
        id: String,
        #[arg(long)]
        reconcile: bool,
        #[arg(long)]
        cursor: Option<i64>,
    },
    Watch {
        #[arg(required = true)]
        task_ids: Vec<String>,
    },
    Cancel {
        #[arg(
            long,
            conflicts_with = "analysis_id",
            required_unless_present = "analysis_id"
        )]
        attempt_id: Option<String>,
        #[arg(long, required_unless_present = "attempt_id")]
        analysis_id: Option<String>,
    },
    Retry {
        task_id: String,
        #[arg(long)]
        wait: bool,
    },
    Queue {
        #[command(subcommand)]
        command: Queue,
    },
    Worker {
        #[command(subcommand)]
        command: Worker,
    },
}
#[derive(Subcommand)]
enum Queue {
    Pause,
    Resume,
    Status,
}
#[derive(Subcommand)]
enum Worker {
    Start,
    Stop,
    Status,
}
fn absolute(path: &Path) -> Result<String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(std::fs::canonicalize(&path)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned())
}
fn write(client: &Client, name: &str, mut body: Value, root: &str) -> Result<Value> {
    body["request_id"] = json!(format!("{root}:{name}"));
    client.call(name, Some(body))
}
fn ids(value: &Value, key: &str) -> Result<Vec<String>> {
    value[key]
        .as_array()
        .ok_or_else(|| Error::new("response_invalid", "cli", format!("缺少 {key}")))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::new("response_invalid", "cli", "ID 无效"))
        })
        .collect()
}
fn wait_analysis(client: &Client, analysis_ids: &[String]) -> Result<Value> {
    let mut last = String::new();
    loop {
        let mut items = vec![];
        let mut pending = false;
        for id in analysis_ids {
            let a = client.call(&format!("analyses/{id}"), None)?;
            pending |= matches!(a["status"].as_str(), Some("queued" | "running"));
            items.push(a);
        }
        if !pending {
            return Ok(json!({"analysis_ids":analysis_ids,"items":items}));
        }
        let snapshot = client.call("snapshot", None)?;
        let message = if snapshot["paused"] == true {
            "分析等待中：队列已暂停，可执行 framelean queue resume"
        } else {
            "正在分析媒体"
        };
        if message != last {
            eprintln!("{message}");
            last = message.into();
        }
        thread::sleep(Duration::from_millis(300));
    }
}
fn prepare(client: &Client, analysis_ids: &[String], preset: Option<&str>) -> Result<Value> {
    let options = client.call("options", Some(json!({"analysis_ids":analysis_ids})))?;
    let choices = options["available_choices"]
        .as_array()
        .ok_or_else(|| Error::new("response_invalid", "cli", "缺少可用选项"))?;
    let selection = choices
        .iter()
        .find(|c| preset.is_none_or(|p| c["preset"].as_str() == Some(p)))
        .cloned()
        .ok_or_else(|| {
            Error::new("no_viable_plan", "planning", "所选素材没有共同可执行方案")
                .details(options.clone())
        })?;
    let plans = client.call(
        "plan",
        Some(json!({"analysis_ids":analysis_ids,"selection":selection})),
    )?;
    if plans["items"]
        .as_array()
        .is_none_or(|items| items.iter().any(|i| !i["error"].is_null()))
    {
        return Err(
            Error::new("no_viable_plan", "planning", "部分素材无法生成方案").details(plans),
        );
    }
    Ok(plans)
}
fn watch(client: &Client, task_ids: &[String]) -> Result<Value> {
    let mut previous = String::new();
    loop {
        let mut tasks = vec![];
        let mut pending = false;
        for id in task_ids {
            let t = client.call(&format!("tasks/{id}"), None)?;
            pending |= matches!(
                t["latest_attempt"]["status"].as_str(),
                Some("queued" | "running" | "validating" | "publishing")
            );
            tasks.push(t);
        }
        let states = tasks
            .iter()
            .map(|t| {
                format!(
                    "{}: {}",
                    t["task"]["id"].as_str().unwrap_or(""),
                    t["latest_attempt"]["status"].as_str().unwrap_or("unknown")
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let snapshot = client.call("snapshot", None)?;
        let progress = snapshot["active"]["event"]["progress"]
            .as_f64()
            .map(|p| format!(" {:.0}%", p * 100.0))
            .unwrap_or_default();
        let line = format!("{states}{progress}");
        if line != previous {
            eprintln!("{line}");
            previous = line;
        }
        if !pending {
            return Ok(json!({"tasks":tasks}));
        }
        thread::sleep(Duration::from_secs(1));
    }
}
fn run(args: &Args) -> Result<Value> {
    let start = !matches!(
        args.command,
        Action::Worker {
            command: Worker::Stop | Worker::Status
        }
    );
    let client = Client::connect(start)?;
    if matches!(
        args.command,
        Action::Doctor
            | Action::Worker {
                command: Worker::Status
            }
    ) {
        return client.call("health", None);
    }
    if !matches!(
        args.command,
        Action::Worker {
            command: Worker::Stop
        }
    ) {
        client.wait_ready()?;
    }
    let root = args
        .request_id
        .clone()
        .unwrap_or_else(framelean_engine::uid);
    if root.is_empty() || root.len() > 90 {
        return Err(Error::new("request_id", "cli", "request-id 需要 1–90 字符"));
    }
    match &args.command {
        Action::Doctor => unreachable!(),
        Action::Analyze { files, wait } => {
            let inputs = files
                .iter()
                .map(|p| absolute(p))
                .collect::<Result<Vec<_>>>()?;
            let accepted = write(&client, "analyze", json!({"inputs":inputs}), &root)?;
            if *wait {
                wait_analysis(&client, &ids(&accepted, "analysis_ids")?)
            } else {
                Ok(accepted)
            }
        }
        Action::Options { analysis_ids } => {
            client.call("options", Some(json!({"analysis_ids":analysis_ids})))
        }
        Action::Plan {
            analysis_ids,
            preset,
        } => prepare(&client, analysis_ids, preset.as_deref()),
        Action::Compress {
            files,
            analysis_ids,
            output,
            preset,
            wait,
        } => {
            let analysis_ids = if !files.is_empty() {
                let inputs = files
                    .iter()
                    .map(|p| absolute(p))
                    .collect::<Result<Vec<_>>>()?;
                let accepted = write(&client, "analyze", json!({"inputs":inputs}), &root)?;
                let ids = ids(&accepted, "analysis_ids")?;
                eprintln!("分析 ID：{}", ids.join(", "));
                wait_analysis(&client, &ids)?;
                ids
            } else {
                analysis_ids.clone()
            };
            if analysis_ids.is_empty() {
                return Err(Error::new(
                    "invalid_arguments",
                    "cli",
                    "提供文件或 --analysis-id",
                ));
            }
            let plans = prepare(&client, &analysis_ids, preset.as_deref())?;
            let items=plans["items"].as_array().unwrap().iter().map(|p|json!({"analysis_id":p["analysis_id"],"intent":p["result"]["intent"],"selection":p["result"]["selection"],"expected_plan_digest":p["result"]["plan_digest"]})).collect::<Vec<_>>();
            let accepted = write(
                &client,
                "execute",
                json!({"items":items,"output_directory":absolute(output)?}),
                &root,
            )?;
            if *wait {
                eprintln!("任务已入队：{}", accepted["task_ids"]);
                watch(&client, &ids(&accepted, "task_ids")?)
            } else {
                Ok(accepted)
            }
        }
        Action::Tasks { limit, cursor } => client.call(
            &format!(
                "tasks?limit={limit}{}",
                cursor.map(|c| format!("&cursor={c}")).unwrap_or_default()
            ),
            None,
        ),
        Action::Task {
            id,
            reconcile,
            cursor,
        } => {
            if *reconcile {
                let t = client.call(&format!("tasks/{id}"), None)?;
                write(
                    &client,
                    "reconcile",
                    json!({"attempt_id":t["latest_attempt"]["id"]}),
                    &root,
                )
            } else {
                client.call(
                    &format!(
                        "tasks/{id}{}",
                        cursor.map(|c| format!("?cursor={c}")).unwrap_or_default()
                    ),
                    None,
                )
            }
        }
        Action::Watch { task_ids } => watch(&client, task_ids),
        Action::Cancel {
            attempt_id,
            analysis_id,
        } => write(
            &client,
            "cancel",
            json!({"attempt_id":attempt_id,"analysis_id":analysis_id}),
            &root,
        ),
        Action::Retry { task_id, wait } => {
            let accepted = write(&client, "retry", json!({"task_id":task_id}), &root)?;
            if *wait {
                watch(&client, &ids(&accepted, "task_ids")?)
            } else {
                Ok(accepted)
            }
        }
        Action::Queue { command } => match command {
            Queue::Pause => write(&client, "queue/pause", json!({}), &root),
            Queue::Resume => write(&client, "queue/resume", json!({}), &root),
            Queue::Status => client.call("snapshot", None),
        },
        Action::Worker { command } => match command {
            Worker::Start | Worker::Status => client.call("health", None),
            Worker::Stop => {
                let response = write(&client, "shutdown", json!({}), &root)?;
                for _ in 0..150 {
                    if !framelean_engine::runtime_dir()
                        .join("endpoint.json")
                        .exists()
                    {
                        return Ok(response);
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(Error::new(
                    "shutdown_pending",
                    "shutdown",
                    "后台仍在收尾，可用 worker status 查看",
                ))
            }
        },
    }
}
fn exit_code(value: &Value) -> i32 {
    if value["lifecycle"] == "degraded" {
        return 3;
    }
    if let Some(tasks) = value["tasks"].as_array() {
        if tasks.iter().any(|t| {
            matches!(
                t["latest_attempt"]["status"].as_str(),
                Some("failed" | "interrupted" | "reconciling")
            )
        }) {
            return 4;
        }
        if tasks
            .iter()
            .any(|t| t["latest_attempt"]["status"] == "canceled")
        {
            return 5;
        }
    }
    if value["items"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|i| i["status"] == "failed" || i["status"] == "interrupted")
    }) {
        return 4;
    }
    if value["items"]
        .as_array()
        .is_some_and(|items| items.iter().any(|i| i["status"] == "canceled"))
    {
        return 5;
    }
    0
}
fn main() {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let help = matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            if !help && std::env::args().any(|a| a == "--json") {
                println!(
                    "{}",
                    json!({"error":Error::new("invalid_arguments","cli",&e)})
                );
            } else {
                let _ = e.print();
            }
            std::process::exit(if help { 0 } else { 2 });
        }
    };
    match run(&args) {
        Ok(value) => {
            let code = exit_code(&value);
            if args.json {
                println!("{value}");
            } else {
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            }
            std::process::exit(code);
        }
        Err(e) => {
            let code = if matches!(
                e.code.as_str(),
                "worker_unavailable"
                    | "component_unavailable"
                    | "protocol_mismatch"
                    | "endpoint_invalid"
                    | "recovery_pending"
            ) {
                3
            } else {
                2
            };
            if args.json {
                println!("{}", json!({"error":e}));
            } else {
                eprintln!("{e}");
                if !e.details.is_null() {
                    eprintln!("{}", serde_json::to_string_pretty(&e.details).unwrap());
                }
            }
            std::process::exit(code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_target_and_terminal_exit_codes() {
        assert!(Args::try_parse_from(["framelean", "cancel"]).is_err());
        assert!(
            Args::try_parse_from([
                "framelean",
                "cancel",
                "--attempt-id",
                "e",
                "--analysis-id",
                "a"
            ])
            .is_err()
        );
        assert_eq!(
            exit_code(&json!({"tasks":[{"latest_attempt":{"status":"canceled"}}]})),
            5
        );
        assert_eq!(
            exit_code(&json!({"tasks":[{"latest_attempt":{"status":"reconciling"}}]})),
            4
        );
        assert_eq!(
            exit_code(&json!({"tasks":[{"latest_attempt":{"status":"succeeded"}}]})),
            0
        );
    }
}
