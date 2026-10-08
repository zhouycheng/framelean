# Client ↔ Work Protocol v1

## Transport

- TCP 仅绑定 `127.0.0.1` 的随机端口；端口和随机访问 token 保存在当前用户的应用数据目录 `endpoint.json`。
- 每个请求连接先发送 `hello` 帧，token 校验成功后发送一个操作帧；操作结束即关闭连接。新连接可以重新认证并接入同一个仍运行的 Work 进程。
- 每帧格式为 `4-byte big-endian payload length + UTF-8 JSON`；帧长度必须在 `1..=16 MiB`。
- Work 只监听 loopback。token 不写日志；Unix endpoint 文件权限为 `0600`。

## Envelope

请求：

```json
{
  "protocol_version": 1,
  "request_id": "client-generated-id",
  "command": "ping",
  "payload": {}
}
```

连接认证使用 `command: "hello"`，token 位于 `payload.token`。认证成功后发送实际操作帧。

成功响应：

```json
{
  "protocol_version": 1,
  "request_id": "client-generated-id",
  "output": "response",
  "ok": true,
  "result": {}
}
```

失败响应 `ok` 为 `false`，错误位于 `error`，可携带稳定 `code`、FLL 的 `engine_code`、消息和 `retryable`。

## 当前操作

- `ping`：转发到 FLL `ping`，用于验证已加载的动态库与 Runtime。
- `analyze_media`：转发给 FLL `analyze_media`，payload 由 `framelean-analysis::MediaAnalyzeRequest`、`framelean-decision::TaskMode` 和 `framelean-runtime::RequestContext` 定义；模型事实由 FLL 拥有。
- `shutdown`：认证后的本地受控关闭，仅用于开发和验证；桌面端断线不会自动停止 Work。

这里实现的是可验证的首个真实调用闭环，不代表 Alpha 的任务队列、执行控制、持久化、快照续接和进度事件已经迁入。
