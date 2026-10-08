# Runtime JSON Schemas

These schemas are generated from the aggregate DTOs owned by
`framelean-runtime`. Regenerate them with:

```bash
cargo run -p framelean-runtime --example export_schemas
```

The internal `AnalyzeTaskRequest` is intentionally excluded because FEngine
maps its public analysis command into that Runtime-only request. Execution
submission is different: `ExecutionSubmissionRequest` and
`ExecutionSubmissionResult` are explicit FLL Runtime boundary DTOs and their
schemas are checked in alongside the analysis and configuration payloads.

The checked-in v1 schemas are updated in place during the pre-release 0.1
phase. The current revision includes the persisted analysis snapshot,
atomic execution-chain capabilities, structured error codes, product preset
policies, resolved configuration, and execution submission. A successful
submission only means that FLL created and queued a real execution; progress,
pause/resume and terminal events remain runtime/protocol state rather than a
claim that submission completed media processing.
