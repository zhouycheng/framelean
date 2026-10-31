import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:framelean/engine_gateway.dart';
import 'package:framelean/processing_model.dart';

Future<void> until(bool Function() predicate) async {
  for (var n = 0; n < 600; n++) {
    if (predicate()) return;
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  fail('Worker timeout');
}

void main() {
  final sample = Platform.environment['FRAMELEAN_TEST_SAMPLE'];
  test(
    'v4 real execution and model reconnect',
    () async {
      final output = await Directory.systemTemp.createTemp('framelean-v4-ui-');
      var model = ProcessingModel(EngineGateway())..start();
      try {
        await until(
          () => model.connected && model.snapshot['lifecycle'] == 'ready',
        );
        await model.analyze([sample!]);
        await model.command('queue/resume', {});
        await until(
          () => model.rows('analyses').any((a) => a['status'] == 'succeeded'),
        );
        final id = model.rows('analyses').last['id'] as String;
        await until(() => model.canProcess(id));
        model.setOutput(output.path);
        await model.enqueue([id], preset: 'quality');
        model.dispose();
        model = ProcessingModel(EngineGateway())..start();
        await until(
          () => model.rows('outputs').any((o) => o['status'] == 'succeeded'),
        );
        expect(
          await File(model.rows('outputs').last['final_path'] as String)
              .exists(),
          true,
        );
        expect(
          model.rows('tasks').last['plan_json']['parameters']['video']['crf'],
          18,
        );
        await model.shutdown();
      } finally {
        model.dispose();
        await output.delete(recursive: true);
      }
    },
    skip: sample == null
        ? 'Set FRAMELEAN_TEST_SAMPLE and isolated runtime/data directories'
        : false,
  );
}
