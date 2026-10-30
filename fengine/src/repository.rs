use crate::{Error, Result};
use rusqlite::{Connection, OpenFlags, Params};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

pub fn rows(db: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    let mut stmt = db.prepare(sql)?;
    let names: Vec<_> = stmt.column_names().map(str::to_owned).collect();
    let rows = stmt
        .query_map(params, |r| {
            let mut out = serde_json::Map::new();
            for (i, n) in names.iter().enumerate() {
                use rusqlite::types::ValueRef;
                let v = match r.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(v) => json!(v),
                    ValueRef::Real(v) => json!(v),
                    ValueRef::Text(v) => {
                        let s = String::from_utf8_lossy(v);
                        if n.ends_with("_json") || n == "source_fingerprint" {
                            serde_json::from_str(&s).unwrap_or(json!(s))
                        } else {
                            json!(s)
                        }
                    }
                    _ => Value::Null,
                };
                out.insert(n.clone(), v);
            }
            Ok(Value::Object(out))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
pub fn bump(db: &Connection) -> Result<i64> {
    db.execute("UPDATE worker_state SET revision=revision+1 WHERE id=1", [])?;
    Ok(
        db.query_row("SELECT revision FROM worker_state WHERE id=1", [], |r| {
            r.get(0)
        })?,
    )
}
pub fn order(db: &Connection) -> Result<i64> {
    db.execute(
        "UPDATE worker_state SET next_order=next_order+1 WHERE id=1",
        [],
    )?;
    Ok(
        db.query_row("SELECT next_order FROM worker_state WHERE id=1", [], |r| {
            r.get(0)
        })?,
    )
}
pub fn read(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    c.busy_timeout(Duration::from_secs(1))?;
    Ok(c)
}

fn reference(version: i64) -> Result<Connection> {
    let db = Connection::open_in_memory()?;
    db.execute_batch(include_str!("schema_v1.sql"))?;
    if version == 2 {
        db.execute_batch(include_str!("migration_v2.sql"))?;
    }
    Ok(db)
}
fn schema(db: &Connection) -> Result<Vec<Value>> {
    let tables = rows(
        db,
        "SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        [],
    )?;
    let mut result = vec![];
    for t in tables {
        let name = t["name"].as_str().unwrap();
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::new("schema_invalid", "database", "数据库表名异常"));
        }
        let norm = |s: &str| {
            s.chars()
                .filter(|c| !c.is_whitespace() && *c != '"')
                .collect::<String>()
                .to_ascii_lowercase()
        };
        result.push(json!({"table":name,"columns":rows(db,&format!("PRAGMA table_info({name})"),[])?,"foreign_keys":rows(db,&format!("PRAGMA foreign_key_list({name})"),[])?,"definition":norm(t["sql"].as_str().unwrap_or(""))}));
    }
    let indexes = rows(
        db,
        "SELECT name,sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL ORDER BY name",
        [],
    )?;
    for mut index in indexes {
        index["sql"] = json!(
            index["sql"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .collect::<String>()
                .to_ascii_lowercase()
        );
        result.push(index);
    }
    Ok(result)
}
fn verify(db: &Connection, version: i64) -> Result<()> {
    if schema(db)? != schema(&reference(version)?)? {
        return Err(Error::new(
            "schema_invalid",
            "database",
            "数据库结构与版本不符；已保留原数据",
        ));
    }
    if !rows(db, "PRAGMA foreign_key_check", [])?.is_empty() {
        return Err(Error::new(
            "foreign_key_invalid",
            "database",
            "数据库外键校验失败",
        ));
    }
    Ok(())
}
pub fn open(path: &Path) -> Result<Connection> {
    let mut db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    match version {
        0 => {
            if db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get::<_, i64>(0),
            )? != 0
            {
                return Err(Error::new(
                    "schema_invalid",
                    "database",
                    "未版本化的数据库包含已有结构",
                ));
            }
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("schema_v1.sql"))?;
            tx.execute_batch(include_str!("migration_v2.sql"))?;
            verify(&tx, 2)?;
            tx.commit()?;
        }
        1 => {
            verify(&db, 1)?;
            let backup = path.with_extension(format!("v1-{}.backup", crate::uid()));
            db.backup("main", &backup, None::<fn(rusqlite::backup::Progress)>)?;
            db.execute_batch("PRAGMA foreign_keys=OFF;")?;
            let migrated = (|| -> Result<()> {
                let tx = db.transaction()?;
                tx.execute_batch(include_str!("migration_v2.sql"))?;
                verify(&tx, 2)?;
                tx.commit()?;
                Ok(())
            })();
            db.execute_batch("PRAGMA foreign_keys=ON;")?;
            migrated?;
        }
        2 => verify(&db, 2)?,
        _ => {
            return Err(Error::new(
                "schema_version",
                "database",
                "不支持的数据库版本",
            ));
        }
    }
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Connection) {
        let path = std::env::temp_dir().join(format!("framelean-db-{}.sqlite", crate::uid()));
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("schema_v1.sql")).unwrap();
        (path, db)
    }
    #[test]
    fn migrate_real_history_and_preserve_relations() {
        let (p, db) = fixture();
        db.execute_batch("INSERT INTO media_source VALUES('s','sample.mp4',0); INSERT INTO analysis VALUES('a','s','{}','succeeded','{}',NULL,1,0); INSERT INTO processing_task VALUES('t','a','{}','out',0); INSERT INTO execution_attempt VALUES('e','t',1,2,'queued',NULL,NULL,NULL,NULL);").unwrap();
        drop(db);
        let db = open(&p).unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM execution_attempt", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "interrupted"
        );
        assert!(db.execute("DELETE FROM analysis WHERE id='a'", []).is_err());
        db.execute("INSERT INTO execution_attempt(id,task_id,attempt_number,queue_order,status) VALUES('e2','t',2,3,'queued')",[]).unwrap();
        assert!(db.execute("INSERT INTO execution_attempt(id,task_id,attempt_number,queue_order,status) VALUES('e3','t',3,4,'reconciling')",[]).is_err());
        drop(db);
        open(&p).unwrap();
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn migration_failure_rolls_back_and_schema_damage_is_rejected() {
        let (p, mut db) = fixture();
        {
            let tx = db.transaction().unwrap();
            tx.execute_batch(include_str!("migration_v2.sql")).unwrap();
        }
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute("ALTER TABLE analysis ADD COLUMN unexpected TEXT", [])
            .unwrap();
        drop(db);
        assert!(open(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn empty_creation_and_unknown_version() {
        let p = std::env::temp_dir().join(format!("framelean-db-{}.sqlite", crate::uid()));
        let db = open(&p).unwrap();
        db.execute_batch("PRAGMA user_version=99").unwrap();
        drop(db);
        assert!(open(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }
}
