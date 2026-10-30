import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:framelean/engine_gateway.dart';
import 'package:framelean/processing_model.dart';

Future<void> until(bool Function() condition) async {
  for (var n = 0; n < 600; n++) {
    if (condition()) return;
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  fail('Timed out waiting for Worker state');
}

void main() {
  final sample = Platform.environment['FRAMELEAN_TEST_SAMPLE'];
  test(
    'Flutter gateway completes real compression and reconnects to its task',
    () async {
      final output = await Directory.systemTemp.createTemp(
        'framelean-flutter-',
      );
      ProcessingModel? model;
      try {
        model = ProcessingModel(EngineGateway())..start();
        await until(() => model!.connected);
        await model.analyze(sample!);
        await until(
          () => model!.state == 'succeeded' || model.state == 'failed',
        );
        expect(model.error, isNull);
        expect(model.analysisId, isNotNull);
        model.setOutput(output.path);
        await model.execute();
        await until(() => model!.task?['kind'] == 'execute');
        final taskId = model.task!['task_id'];
        model.dispose();
        model = ProcessingModel(EngineGateway())..start();
        await until(() => model!.task?['task_id'] == taskId);
        await until(
          () => model!.state == 'succeeded' || model.state == 'failed',
        );
        expect(model.error, isNull);
        expect(await File(model.task!['output'] as String).exists(), true);
        expect(model.media, isNotNull);
        expect(model.analysisId, isNotNull);
        await model.shutdown();
      } finally {
        model?.dispose();
        await output.delete(recursive: true);
      }
    },
    skip: sample == null
        ? 'Set FRAMELEAN_TEST_SAMPLE and an isolated FRAMELEAN_RUNTIME_DIR'
        : false,
  );
}
