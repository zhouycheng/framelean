ALTER TABLE analysis ADD COLUMN facts_schema INTEGER;
ALTER TABLE analysis ADD COLUMN analyzer_version TEXT;
ALTER TABLE analysis ADD COLUMN facts_digest TEXT;
ALTER TABLE analysis ADD COLUMN cancel_requested_at INTEGER;
ALTER TABLE processing_task ADD COLUMN intent_json TEXT;
ALTER TABLE processing_task ADD COLUMN plan_json TEXT;
ALTER TABLE processing_task ADD COLUMN plan_digest TEXT;
DROP INDEX one_active_attempt;
CREATE TABLE execution_attempt_new(
 id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES processing_task(id),
 attempt_number INTEGER NOT NULL, queue_order INTEGER NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('queued','running','validating','publishing','reconciling','succeeded','failed','canceled','interrupted')),
 temp_path TEXT, error_json TEXT, started_at INTEGER, finished_at INTEGER,
 temp_identity_json TEXT, cleanup_state TEXT NOT NULL DEFAULT 'done' CHECK(cleanup_state IN ('pending','done','error')),
 cancel_requested_at INTEGER, validation_json TEXT,
 UNIQUE(task_id,attempt_number)
);
INSERT INTO execution_attempt_new(id,task_id,attempt_number,queue_order,status,temp_path,error_json,started_at,finished_at)
 SELECT id,task_id,attempt_number,queue_order,status,temp_path,error_json,started_at,finished_at FROM execution_attempt;
DROP TABLE execution_attempt;
ALTER TABLE execution_attempt_new RENAME TO execution_attempt;
CREATE UNIQUE INDEX one_active_attempt ON execution_attempt(task_id) WHERE status IN ('queued','running','validating','publishing','reconciling');
ALTER TABLE output_artifact ADD COLUMN file_identity_json TEXT;
CREATE INDEX analysis_order ON analysis(queue_order);
CREATE INDEX attempt_order ON execution_attempt(queue_order);
CREATE INDEX task_order ON processing_task(created_at,id);
UPDATE execution_attempt SET status='interrupted',error_json='{"code":"plan_required","stage":"migration","message":"旧任务需要重新分析和规划","retryable":false}',finished_at=unixepoch() WHERE status='queued';
PRAGMA user_version=2;
