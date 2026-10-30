import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:framelean/processing_model.dart';
import 'package:framelean/engine_gateway.dart';

class FakeGateway extends EngineGateway {
  final stream = StreamController<Map<String, dynamic>>();
  final response = Completer<Map<String, dynamic>>();
  @override
  Stream<Map<String, dynamic>> events() => stream.stream;
  @override
  Future<Map<String, dynamic>> call(
    String name, [
    Map<String, dynamic>? body,
  ]) => response.future;
  @override
  void close() {
    stream.close();
  }
}

void main() {
  test('reconnection restores progress and prevents duplicate work', () async {
    final gateway = FakeGateway();
    final model = ProcessingModel(gateway)..start();
    gateway.stream.add({
      'instance_id': 'worker',
      'seq': 4,
      'task': {
        'state': 'running',
        'kind': 'execute',
        'input': 'sample.mp4',
        'progress': 0.4,
      },
    });
    await Future<void>.delayed(Duration.zero);
    expect(model.busy, true);
    expect(model.progress, 0.4);
    expect(model.input, 'sample.mp4');
    model.dispose();
  });
  test(
    'late HTTP admission response cannot overwrite terminal SSE state',
    () async {
      final gateway = FakeGateway();
      final model = ProcessingModel(gateway)..start();
      gateway.stream.add({'instance_id': 'worker', 'seq': 0, 'task': null});
      await Future<void>.delayed(Duration.zero);
      final pending = model.analyze('sample.mp4');
      gateway.stream.add({
        'instance_id': 'worker',
        'seq': 2,
        'task': {
          'state': 'succeeded',
          'kind': 'analyze',
          'input': 'sample.mp4',
          'result': <String, dynamic>{},
          'analysis_id': 'a',
        },
      });
      await Future<void>.delayed(Duration.zero);
      gateway.response.complete({
        'instance_id': 'worker',
        'seq': 1,
        'task_id': 'a',
        'state': 'running',
      });
      await pending;
      expect(model.state, 'succeeded');
      expect(model.analysisId, 'a');
      gateway.stream.add({'instance_id': 'new-worker', 'seq': 0, 'task': null});
      await Future<void>.delayed(Duration.zero);
      expect(model.analysisId, isNull);
      expect(model.media, isNull);
      model.dispose();
    },
  );
}
