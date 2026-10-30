import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:framelean/engine_gateway.dart';
import 'package:framelean/processing_model.dart';

class FakeGateway extends EngineGateway {
  final controller = StreamController<Map<String, dynamic>>();
  Map<String, dynamic> state = {
    'instance_id': 'a',
    'seq': 4,
    'lifecycle': 'ready',
    'paused': true,
  };
  String status = 'queued';
  @override
  Stream<Map<String, dynamic>> events() => controller.stream;
  @override
  Future<Map<String, dynamic>> call(
    String name, [
    Map<String, dynamic>? body,
  ]) async {
    if (name == 'snapshot') return state;
    if (name.startsWith('analyses?')) {
      return {
        'items': [
          {'id': 'supported', 'status': 'succeeded'},
          {'id': 'hdr', 'status': 'succeeded'},
        ],
      };
    }
    if (name == 'options') {
      return {
        'items': [
          {
            'analysis_id': 'supported',
            'result': {
              'available_choices': [
                {'preset': 'h264_crf23', 'container': 'mp4'},
              ],
            },
          },
          {
            'analysis_id': 'hdr',
            'result': {
              'available_choices': [],
              'diagnostics': [
                {'message': 'HDR 处理链尚未接通'},
              ],
            },
          },
        ],
      };
    }
    if (name.startsWith('tasks?')) {
      return {
        'items': [
          {'id': 't'},
        ],
        'attempts': [
          {'id': 'e', 'status': status},
        ],
        'outputs': [],
      };
    }
    throw StateError('Unexpected command $name');
  }

  @override
  void close() {
    controller.close();
  }
}

void main() {
  test('v3 reconnect, stale progress, hidden unsupported choices and restart projection', () async {
    final gateway = FakeGateway();
    final model = ProcessingModel(gateway)..start();
    gateway.controller.add({'type': 'snapshot', 'data': gateway.state});
    await Future<void>.delayed(Duration.zero);
    expect(model.rows('tasks').single['id'], 't');
    expect(model.paused, true);
    expect(model.canProcess('supported'), true);
    expect(model.canProcess('hdr'), false);
    expect(model.unavailableReason('hdr'), contains('HDR'));
    gateway.controller.add({
      'type': 'progress',
      'instance_id': 'a',
      'seq': 5,
      'data': {'id': 'e'},
    });
    gateway.controller.add({
      'type': 'progress',
      'instance_id': 'a',
      'seq': 3,
      'data': null,
    });
    await Future<void>.delayed(Duration.zero);
    expect(model.snapshot['active']['id'], 'e');
    gateway.status = 'interrupted';
    gateway.state = {
      'instance_id': 'b',
      'seq': 1,
      'lifecycle': 'ready',
      'paused': true,
      'active': null,
    };
    gateway.controller.add({'type': 'snapshot', 'data': gateway.state});
    await Future<void>.delayed(Duration.zero);
    expect(model.rows('attempts').single['status'], 'interrupted');
    expect(model.snapshot['active'], null);
    model.dispose();
  });
}
