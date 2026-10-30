import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'engine_gateway.dart';

class ProcessingModel extends ChangeNotifier {
  ProcessingModel(this.gateway);
  final EngineGateway gateway;
  StreamSubscription<Map<String, dynamic>>? _events;
  Map<String, dynamic> snapshot = {};
  final Map<String, Map<String, dynamic>> choices = {};
  String? error;
  String outputDirectory = '';
  bool connected = false, submitting = false, _disposed = false;
  bool _refreshing = false, _refreshPending = false;
  dynamic analysisCursor, taskCursor;
  int _seq = -1;
  String? _instance, _capability;
  Timer? _retryOptions;
  List<Map<String, dynamic>> rows(String key) =>
      (snapshot[key] as List? ?? []).cast<Map<String, dynamic>>();
  bool get paused => snapshot['paused'] == true;
  bool get busy => snapshot['active'] != null;
  bool canProcess(String id) =>
      (choices[id]?['available_choices'] as List? ?? []).isNotEmpty;
  String unavailableReason(String id) {
    final diagnostics = choices[id]?['diagnostics'] as List? ?? [];
    return diagnostics.isEmpty ? '正在查询可用方案' : '${diagnostics.first['message']}';
  }

  void start() {
    _events = gateway.events().listen((event) {
      if (_disposed) return;
      if (event.containsKey('connection_error')) {
        connected = false;
        error = '${event['connection_error']}';
      } else {
        connected = true;
        final data = event['data'];
        if (event['type'] == 'snapshot' || event['type'] == 'reset') {
          _applySnapshot((data as Map).cast<String, dynamic>());
          unawaited(refresh());
        } else {
          final instance = event['instance_id'] as String?;
          if (instance != _instance) {
            _instance = instance;
            _seq = -1;
          }
          final seq = event['seq'] as int? ?? 0;
          if (seq <= _seq) return;
          _seq = seq;
          if (event['type'] == 'progress') {
            snapshot['active'] = data;
          } else {
            unawaited(refresh());
          }
        }
      }
      notifyListeners();
    });
  }

  void _applySnapshot(Map<String, dynamic> value) {
    final instance = value['instance_id'] as String?;
    if (instance == _instance && (value['seq'] as int? ?? 0) < _seq) return;
    if (instance != _instance) _seq = -1;
    _instance = instance;
    _seq = value['seq'] as int? ?? 0;
    snapshot.addAll(value);
    final capability =
        value['health']?['capabilities']?['capability_digest'] as String?;
    if (capability != _capability) {
      choices.clear();
      _capability = capability;
    }
    if (value['lifecycle'] == 'degraded') {
      error = '${value['health']?['error']?['message'] ?? '后台组件不可用'}';
    }
  }

  Future<void> refresh() async {
    if (_refreshing) {
      _refreshPending = true;
      return;
    }
    _refreshing = true;
    try {
      do {
        _refreshPending = false;
        final state = await gateway.call('snapshot');
        if (_disposed) return;
        _applySnapshot(state);
        if (state['lifecycle'] != 'ready') break;
        await loadAnalyses();
        await loadTasks();
      } while (_refreshPending && !_disposed);
    } catch (e) {
      if (!_disposed) error = '$e';
    } finally {
      _refreshing = false;
      if (!_disposed) notifyListeners();
    }
  }

  Future<void> loadAnalyses({bool more = false}) async {
    final value = await gateway.call(
      'analyses?limit=50${more && analysisCursor != null ? '&cursor=$analysisCursor' : ''}',
    );
    if (_disposed) return;
    final items = (value['items'] as List).cast<Map<String, dynamic>>();
    snapshot['analyses'] = more ? [...rows('analyses'), ...items] : items;
    analysisCursor = value['next_cursor'];
    final missing = items
        .where(
          (a) => a['status'] == 'succeeded' && !choices.containsKey(a['id']),
        )
        .map((a) => a['id'])
        .toList();
    if (missing.isNotEmpty) {
      final result = await gateway.call('options', {'analysis_ids': missing});
      if (_disposed) return;
      for (final item in result['items'] as List) {
        if (item['error']?['code'] == 'query_busy') {
          _retryOptions?.cancel();
          _retryOptions = Timer(const Duration(seconds: 1), refresh);
          continue;
        }
        choices[item['analysis_id'] as String] = item['result'] != null
            ? (item['result'] as Map).cast<String, dynamic>()
            : {
                'available_choices': [],
                'diagnostics': [item['error']],
              };
      }
    }
    if (!_disposed) notifyListeners();
  }

  Future<void> loadTasks({bool more = false}) async {
    final value = await gateway.call(
      'tasks?limit=50${more && taskCursor != null ? '&cursor=$taskCursor' : ''}',
    );
    if (_disposed) return;
    for (final key in ['tasks', 'attempts', 'outputs']) {
      final source = key == 'tasks' ? 'items' : key;
      final items = (value[source] as List).cast<Map<String, dynamic>>();
      snapshot[key] = more ? [...rows(key), ...items] : items;
    }
    taskCursor = value['next_cursor'];
    notifyListeners();
  }

  Future<void> loadMore(String kind) async {
    try {
      if (kind == 'analyses') {
        await loadAnalyses(more: true);
      } else {
        await loadTasks(more: true);
      }
    } catch (e) {
      error = '$e';
      if (!_disposed) notifyListeners();
    }
  }

  Future<void> command(String name, Map<String, dynamic> body) async {
    submitting = true;
    error = null;
    notifyListeners();
    try {
      await gateway.call(name, body);
      await refresh();
    } catch (e) {
      error = '$e';
    } finally {
      submitting = false;
      if (!_disposed) notifyListeners();
    }
  }

  Future<void> analyze(List<String> paths) async {
    if (paths.isEmpty) return;
    if (outputDirectory.isEmpty) {
      outputDirectory = File(paths.first).parent.path;
    }
    await command('analyze', {'inputs': paths});
  }

  Future<void> enqueue(List<String> ids) async {
    submitting = true;
    error = null;
    notifyListeners();
    try {
      final options = await gateway.call('options', {'analysis_ids': ids});
      final available = options['available_choices'] as List;
      if (available.isEmpty) throw StateError('所选媒体没有共同可用方案');
      final planned = await gateway.call('plan', {
        'analysis_ids': ids,
        'selection': available.first,
      });
      final items = <Map<String, dynamic>>[];
      for (final item in planned['items'] as List) {
        if (item['error'] != null) {
          throw StateError('${item['error']['message']}');
        }
        final plan = item['result'];
        items.add({
          'analysis_id': item['analysis_id'],
          'intent': plan['intent'],
          'selection': plan['selection'],
          'expected_plan_digest': plan['plan_digest'],
        });
      }
      await gateway.call('execute', {
        'items': items,
        'output_directory': outputDirectory,
      });
      await refresh();
    } catch (e) {
      error = '$e';
    } finally {
      submitting = false;
      if (!_disposed) notifyListeners();
    }
  }

  void setOutput(String path) {
    outputDirectory = path;
    notifyListeners();
  }

  Future<void> shutdown() async {
    await gateway.call('shutdown', {});
    gateway.stopReconnect();
    for (var n = 0; n < 150; n++) {
      if (!await File('${gateway.runtimeDirectory}/endpoint.json').exists()) {
        return;
      }
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
    throw StateError('后台仍在收尾，请稍后查看状态');
  }

  @override
  void dispose() {
    _disposed = true;
    _retryOptions?.cancel();
    gateway.close();
    _events?.cancel();
    super.dispose();
  }
}
