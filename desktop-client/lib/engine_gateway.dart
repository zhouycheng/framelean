import 'dart:async';
import 'dart:convert';
import 'dart:io';

/// The Worker owns media operations. This class only transports commands/events.
class EngineGateway {
  final HttpClient _http = HttpClient()
    ..connectionTimeout = const Duration(seconds: 3);
  Map<String, dynamic>? _endpoint;
  bool _closed = false;
  String get runtimeDirectory =>
      Platform.environment['FRAMELEAN_RUNTIME_DIR'] ??
      (Platform.isWindows
          ? '${Platform.environment['LOCALAPPDATA']}/FrameLean/runtime'
          : '${Platform.environment['HOME']}/Library/Application Support/FrameLean/runtime');

  Future<void> connect() async {
    Future<bool> discover() async {
      try {
        _endpoint = jsonDecode(
          await File('$runtimeDirectory/endpoint.json').readAsString(),
        ) as Map<String, dynamic>;
        if (_endpoint!['protocol_version'] != 4) {
          throw const FormatException('Worker 协议不兼容，请先退出旧版本后台');
        }
        final url = Uri.parse(_endpoint!['url'] as String);
        if (url.scheme != 'http' || url.host != '127.0.0.1') {
          throw const FormatException('Worker 地址无效');
        }
        final health = await call('health');
        if (health['protocol_version'] != 4) {
          throw const FormatException('Worker 协议不兼容');
        }
        return true;
      } on FormatException {
        rethrow;
      } catch (_) {
        _endpoint = null;
        return false;
      }
    }

    if (await discover()) return;
    final binary =
        Platform.environment['FRAMELEAN_ENGINE'] ??
        (Platform.isMacOS
            ? '${File(Platform.resolvedExecutable).parent.parent.path}/Resources/runtime/framelean-engine'
            : '${File(Platform.resolvedExecutable).parent.path}/runtime/framelean-engine.exe');
    if (!await File(binary).exists()) {
      throw FileSystemException(
        '缺少 Worker，请运行打包脚本或配置 FRAMELEAN_ENGINE',
        binary,
      );
    }
    await Process.start(binary, [
      'worker',
      if (Platform.environment['FRAMELEAN_FLL'] != null)
        Platform.environment['FRAMELEAN_FLL']!,
    ], mode: ProcessStartMode.detached);
    for (var n = 0; n < 60 && !_closed; n++) {
      if (await discover()) return;
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    throw const SocketException('Worker 启动超时，请检查运行组件');
  }

  Future<Map<String, dynamic>> call(
    String name, [
    Map<String, dynamic>? body,
  ]) async {
    final endpoint = _endpoint;
    if (endpoint == null) throw const SocketException('尚未连接 Worker');
    final request = await _http.openUrl(
      body == null ? 'GET' : 'POST',
      Uri.parse('${endpoint['url']}/v4/$name'),
    );
    request.headers.set(
      HttpHeaders.authorizationHeader,
      'Bearer ${endpoint['token']}',
    );
    if (body != null) {
      request.headers.contentType = ContentType.json;
      request.write(
        jsonEncode({
          ...body,
          'request_id':
              body['request_id'] ??
              '${DateTime.now().microsecondsSinceEpoch}-$name',
        }),
      );
    }
    final response = await request.close().timeout(const Duration(seconds: 60));
    final text = await utf8.decoder
        .bind(response)
        .join()
        .timeout(const Duration(seconds: 60));
    final data = jsonDecode(text) as Map<String, dynamic>;
    if (response.statusCode >= 400) {
      throw StateError('${data['error']?['message'] ?? '请求失败'}');
    }
    return data;
  }

  Stream<Map<String, dynamic>> events() async* {
    while (!_closed) {
      try {
        await connect();
        final snapshot = await call('snapshot');
        yield {'type': 'snapshot', 'data': snapshot};
        final request = await _http.getUrl(
          Uri.parse('${_endpoint!['url']}/v4/events'),
        );
        request.headers.set('Last-Event-ID', snapshot['event_id'] as String);
        request.headers.set(
          HttpHeaders.authorizationHeader,
          'Bearer ${_endpoint!['token']}',
        );
        final response = await request.close().timeout(
          const Duration(seconds: 60),
        );
        if (response.statusCode != 200) throw const SocketException('事件连接失败');
        await for (final line
            in response
                .transform(utf8.decoder)
                .transform(const LineSplitter())
                .timeout(const Duration(seconds: 10))) {
          if (_closed) return;
          if (line.startsWith('data: ')) {
            yield jsonDecode(line.substring(6)) as Map<String, dynamic>;
          }
        }
      } catch (error) {
        if (_closed) return;
        yield {'connection_error': error.toString()};
      }
      await Future<void>.delayed(const Duration(seconds: 1));
    }
  }

  void stopReconnect() {
    _closed = true;
  }

  void close() {
    _closed = true;
    _http.close(force: true);
  }
}
