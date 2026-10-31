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
  Map<String, dynamic>? submitted;
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
      expect(body?['format'], 'mp4');
      final choices = [
        {'preset': 'balanced', 'container': 'mp4'},
        {'preset': 'quality', 'container': 'mp4'},
        {'preset': 'small', 'container': 'mp4'},
      ];
      final presets = [
        {'id': 'balanced', 'label': '均衡'},
        {'id': 'quality', 'label': '画质优先'},
        {'id': 'small', 'label': '体积优先'},
      ];
      return {
        'available_choices': choices,
        'default_selection': choices.first,
        'presets': presets,
        'items': [
          {
            'analysis_id': 'supported',
            'result': {'available_choices': choices, 'presets': presets},
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
    if (name == 'plan') {
      return {
        'items': [
          {
            'analysis_id': 'supported',
            'result': {
              'selection': body!['selection'],
              'intent': {'operation': 'compress', 'preservation': 'preserve'},
              'plan_digest': 'digest',
            },
          },
        ],
      };
    }
    if (name == 'execute') {
      submitted = body;
      return {
        'task_ids': ['t'],
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
  test(
    'valid preset selection is submitted and unavailable choice is blocked',
    () async {
      final gateway = FakeGateway();
      final model = ProcessingModel(gateway);
      await model.loadAnalyses();
      expect(model.presetsFor(['supported']).length, 3);
      expect(model.presetsFor(['supported', 'hdr']), isEmpty);
      await model.enqueue(['supported'], preset: 'quality');
      expect(gateway.submitted!['items'][0]['selection']['preset'], 'quality');
      gateway.submitted = null;
      await model.enqueue(['supported'], preset: 'unknown');
      expect(gateway.submitted, isNull);
      expect(model.error, contains('预设已不可用'));
      model.dispose();
    },
  );
  test('v4 reconnect, stale progress, hidden unsupported choices and restart projection', () async {
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
