import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'engine_gateway.dart';

class ProcessingModel extends ChangeNotifier {
  ProcessingModel(this.gateway);
  final EngineGateway gateway;
  StreamSubscription<Map<String, dynamic>>? _events;
  Map<String, dynamic>? task;
  Map<String, dynamic>? media;
  String? analysisId;
  String input = '';
  String outputDirectory = '';
  String? error;
  bool connected = false;
  bool submitting = false;
  bool _disposed = false;
  int _sequence = -1;
  String? _instance;
  bool get busy =>
      submitting || ['running', 'validating'].contains(task?['state']);
  double? get progress => (task?['progress'] as num?)?.toDouble();
  String get state => task?['state'] as String? ?? 'idle';
  void start() {
    _events = gateway.events().listen((snapshot) {
      if (_disposed) return;
      if (snapshot.containsKey('connection_error')) {
        connected = false;
        error = snapshot['connection_error'] as String;
      } else {
        final instance = snapshot['instance_id'] as String?;
        if (instance != _instance) {
          _sequence = -1;
          _instance = instance;
          analysisId = null;
          media = null;
        }
        final sequence = snapshot['seq'] as int? ?? 0;
        if (sequence < _sequence) return;
        _sequence = sequence;
        connected = true;
        error = snapshot['health']?['available'] == false
            ? '${snapshot['health']?['error'] ?? '编码组件不可用'}'
            : null;
        task = snapshot['task'] as Map<String, dynamic>?;
        if (task != null) {
          input = task!['input'] as String? ?? input;
          if (outputDirectory.isEmpty && input.isNotEmpty) {
            outputDirectory = File(input).parent.path;
          }
          if (task!['analysis_id'] is String) {
            analysisId = task!['analysis_id'] as String;
          }
          if (task!['state'] == 'succeeded' && task!['kind'] == 'analyze') {
            media = task!['result'] as Map<String, dynamic>;
            analysisId = task!['analysis_id'] as String;
          } else if (task!['analysis'] is Map &&
              (task!['analysis'] as Map).isNotEmpty) {
            media = task!['analysis']['media'] as Map<String, dynamic>?;
          }
          if (task!['error'] != null) error = '${task!['error']['message']}';
        }
      }
      notifyListeners();
    });
  }

  Future<void> _command(String name, Map<String, dynamic> body) async {
    submitting = true;
    error = null;
    notifyListeners();
    try {
      final result = await gateway.call(name, body);
      if (result['task_id'] != null &&
          result['instance_id'] == _instance &&
          (result['seq'] as int? ?? -1) >= _sequence) {
        task = result;
        _sequence = result['seq'] as int;
      }
    } catch (e) {
      error = e.toString();
    } finally {
      submitting = false;
      if (!_disposed) notifyListeners();
    }
  }

  Future<void> analyze(String path) async {
    input = path;
    media = null;
    analysisId = null;
    if (outputDirectory.isEmpty) outputDirectory = File(path).parent.path;
    await _command('analyze', {'input': path});
  }

  void setOutput(String path) {
    outputDirectory = path;
    notifyListeners();
  }

  Future<void> execute() => _command('execute', {
    'analysis_id': analysisId,
    'output_dir': outputDirectory,
  });
  Future<void> cancel() => _command('cancel', {'task_id': task?['task_id']});
  Future<void> shutdown() async {
    gateway.stopReconnect();
    await gateway.call('shutdown', {});
    final endpoint = File('${gateway.runtimeDirectory}/endpoint.json');
    for (var n = 0; n < 60; n++) {
      if (!await endpoint.exists()) return;
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
    throw StateError('后台退出超时，请查看诊断日志');
  }

  @override
  void dispose() {
    _disposed = true;
    gateway.close();
    _events?.cancel();
    super.dispose();
  }
}
