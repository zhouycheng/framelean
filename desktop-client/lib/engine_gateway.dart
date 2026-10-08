import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

const _protocolVersion = 1;
const _maxFrameBytes = 16 * 1024 * 1024;

class EngineGatewayException implements Exception {
  const EngineGatewayException(this.message);

  final String message;

  @override
  String toString() => message;
}

class EngineGateway {
  EngineGateway({String? engineBinary, String? stateDirectory})
    : _random = Random.secure(),
      _engineBinaryOverride = engineBinary,
      _stateDirectoryOverride = stateDirectory;

  final Random _random;
  final String? _engineBinaryOverride;
  final String? _stateDirectoryOverride;
  _Endpoint? _endpoint;

  Future<Map<String, dynamic>> ping() async {
    final endpoint = await _ensureService();
    return _exchange(endpoint, 'ping', const {});
  }

  Future<Map<String, dynamic>> analyzeAudio(String path) async {
    final endpoint = await _ensureService();
    return _exchange(endpoint, 'analyze_media', {
      'task_mode': 'audio_compress',
      'media_request': {
        'source': {'kind': 'local_file', 'value': path},
        'request_id': _requestId(),
        'expected_source': null,
      },
      'context': const <String, Object?>{},
    });
  }

  Future<void> stopService() async {
    final endpoint = await _ensureService();
    await _exchange(endpoint, 'shutdown', const {});
    final endpointFile = File(
      '${_stateDirectory()}${Platform.pathSeparator}endpoint.json',
    );
    final deadline = DateTime.now().add(const Duration(seconds: 5));
    while (await endpointFile.exists() && DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    _endpoint = null;
  }

  String _requestId() =>
      '${DateTime.now().microsecondsSinceEpoch}-${_random.nextInt(1 << 32)}';

  Future<_Endpoint> _ensureService() async {
    final cached = _endpoint;
    if (cached != null) {
      try {
        await _exchange(cached, 'ping', const {});
        return cached;
      } on Object {
        _endpoint = null;
      }
    }

    final stateDirectory = Directory(_stateDirectory());
    await stateDirectory.create(recursive: true);
    final endpointFile = File(
      '${stateDirectory.path}${Platform.pathSeparator}endpoint.json',
    );
    final existing = await _readEndpoint(endpointFile);
    if (existing?.state == 'ready') {
      try {
        await _exchange(existing!, 'ping', const {});
        _endpoint = existing;
        return existing;
      } on Object {
        // A stale or incompatible endpoint is replaced by a freshly started service.
      }
    }

    final binary = _engineBinary();
    if (!await File(binary).exists()) {
      throw EngineGatewayException('找不到 FrameLean Work 程序：$binary');
    }
    try {
      await Process.start(binary, [
        'serve',
        '--state-dir',
        stateDirectory.path,
      ], mode: ProcessStartMode.detached);
    } on Object catch (error) {
      throw EngineGatewayException('无法启动 FrameLean Work：$error');
    }

    final deadline = DateTime.now().add(const Duration(seconds: 20));
    Object? lastConnectionError;
    while (DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 120));
      final endpoint = await _readEndpoint(endpointFile);
      if (endpoint?.state == 'failed') {
        throw EngineGatewayException(
          'FrameLean Work 启动失败：${endpoint!.error ?? '未知错误'}',
        );
      }
      if (endpoint?.state == 'ready') {
        try {
          await _exchange(endpoint!, 'ping', const {});
          _endpoint = endpoint;
          return endpoint;
        } on Object catch (error) {
          lastConnectionError = error;
        }
      }
    }
    throw EngineGatewayException(
      '等待 FrameLean Work 启动超时${lastConnectionError == null ? '' : '：$lastConnectionError'}',
    );
  }

  Future<_Endpoint?> _readEndpoint(File file) async {
    if (!await file.exists()) {
      return null;
    }
    try {
      final value =
          jsonDecode(await file.readAsString()) as Map<String, dynamic>;
      if (value['schema_version'] != 1 || value['state'] is! String) {
        return null;
      }
      return _Endpoint(
        state: value['state'] as String,
        pid: value['pid'] as int? ?? 0,
        port: value['port'] as int?,
        token: value['token'] as String?,
        error: value['error'] as String?,
      );
    } on Object {
      return null;
    }
  }

  Future<Map<String, dynamic>> _exchange(
    _Endpoint endpoint,
    String command,
    Map<String, Object?> payload,
  ) async {
    final port = endpoint.port;
    final token = endpoint.token;
    if (endpoint.state != 'ready' || port == null || token == null) {
      throw const EngineGatewayException('FrameLean Work endpoint 尚未就绪');
    }

    Socket? socket;
    StreamIterator<Uint8List>? iterator;
    try {
      socket = await Socket.connect(
        InternetAddress.loopbackIPv4,
        port,
        timeout: const Duration(seconds: 3),
      );
      socket.setOption(SocketOption.tcpNoDelay, true);
      iterator = StreamIterator<Uint8List>(socket);
      final helloId = _requestId();
      await _writeFrame(socket, {
        'protocol_version': _protocolVersion,
        'request_id': helloId,
        'command': 'hello',
        'payload': {'token': token},
      });
      final hello = await _readFrame(iterator);
      _checkResponse(hello, requestId: helloId);
      final operationId = _requestId();
      await _writeFrame(socket, {
        'protocol_version': _protocolVersion,
        'request_id': operationId,
        'command': command,
        'payload': payload,
      });
      return _checkResponse(await _readFrame(iterator), requestId: operationId);
    } on EngineGatewayException {
      rethrow;
    } on Object catch (error) {
      throw EngineGatewayException('连接 FrameLean Work 失败：$error');
    } finally {
      await iterator?.cancel();
      socket?.destroy();
    }
  }

  Map<String, dynamic> _checkResponse(
    Map<String, dynamic> response, {
    required String requestId,
  }) {
    if (response['protocol_version'] != _protocolVersion) {
      throw const EngineGatewayException('FrameLean Work 返回了不兼容的协议版本');
    }
    if (response['request_id'] != requestId) {
      throw const EngineGatewayException('FrameLean Work 响应与请求不匹配');
    }
    if (response['ok'] != true) {
      final error = response['error'];
      if (error is Map<String, dynamic>) {
        throw EngineGatewayException(
          error['message'] as String? ?? 'FrameLean Work 请求失败',
        );
      }
      throw const EngineGatewayException('FrameLean Work 请求失败');
    }
    final result = response['result'];
    if (result is Map<String, dynamic>) return result;
    throw const EngineGatewayException('FrameLean Work 返回了无效结果');
  }

  String _engineBinary() {
    final override =
        _engineBinaryOverride ??
        Platform.environment['FRAMELEAN_FENGINE_BINARY'];
    if (override != null && override.isNotEmpty) return override;

    final executableDirectory = File(Platform.resolvedExecutable).parent.path;
    if (Platform.isMacOS) {
      return '$executableDirectory/../Resources/framelean-engine';
    }
    if (Platform.isWindows) {
      return '$executableDirectory/framelean-engine.exe';
    }
    throw const EngineGatewayException('FrameLean Work 目前只支持 macOS 和 Windows');
  }

  String _stateDirectory() {
    final override =
        _stateDirectoryOverride ??
        Platform.environment['FRAMELEAN_ENGINE_STATE_DIR'];
    if (override != null && override.isNotEmpty) return override;

    if (Platform.isMacOS) {
      final home = Platform.environment['HOME'];
      if (home == null || home.isEmpty) {
        throw const EngineGatewayException('无法确定 macOS 用户目录');
      }
      return '$home/Library/Application Support/FrameLean/engine';
    }
    if (Platform.isWindows) {
      final appData = Platform.environment['APPDATA'];
      if (appData == null || appData.isEmpty) {
        throw const EngineGatewayException('无法确定 Windows 应用数据目录');
      }
      return '$appData${Platform.pathSeparator}FrameLean${Platform.pathSeparator}engine';
    }
    throw const EngineGatewayException('FrameLean Work 目前只支持 macOS 和 Windows');
  }
}

class _Endpoint {
  const _Endpoint({
    required this.state,
    required this.pid,
    this.port,
    this.token,
    this.error,
  });

  final String state;
  final int pid;
  final int? port;
  final String? token;
  final String? error;
}

Future<void> _writeFrame(Socket socket, Map<String, Object?> value) async {
  final payload = utf8.encode(jsonEncode(value));
  if (payload.isEmpty || payload.length > _maxFrameBytes) {
    throw const EngineGatewayException('FrameLean Work 请求超过 16 MiB 限制');
  }
  final header = ByteData(4)..setUint32(0, payload.length, Endian.big);
  socket.add(header.buffer.asUint8List());
  socket.add(payload);
  await socket.flush();
}

Future<Map<String, dynamic>> _readFrame(
  StreamIterator<Uint8List> iterator,
) async {
  final reader = _FrameReader(iterator);
  final header = await reader.readExact(4);
  final length = ByteData.sublistView(header).getUint32(0, Endian.big);
  if (length == 0 || length > _maxFrameBytes) {
    throw const EngineGatewayException('FrameLean Work 返回了无效帧长度');
  }
  final bytes = await reader.readExact(length);
  final decoded = jsonDecode(utf8.decode(bytes));
  if (decoded is! Map<String, dynamic>) {
    throw const EngineGatewayException('FrameLean Work 返回了无效 JSON');
  }
  return decoded;
}

class _FrameReader {
  _FrameReader(this.iterator);

  final StreamIterator<Uint8List> iterator;
  final Queue<int> _pending = Queue<int>();

  Future<Uint8List> readExact(int length) async {
    while (_pending.length < length) {
      final hasData = await iterator.moveNext().timeout(
        const Duration(seconds: 20),
      );
      if (!hasData) throw const SocketException('FrameLean Work 关闭了连接');
      _pending.addAll(iterator.current);
    }
    return Uint8List.fromList(
      List<int>.generate(length, (_) => _pending.removeFirst()),
    );
  }
}
