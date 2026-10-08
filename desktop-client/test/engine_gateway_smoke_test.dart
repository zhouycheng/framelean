import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:framelean/engine_gateway.dart';

void main() {
  final engineBinary = Platform.environment['FRAMELEAN_FENGINE_BINARY'];

  test(
    'Flutter gateway reconnects to Work and analyzes a WAV through the FLL library',
    () async {
      final temporaryDirectory = await Directory.systemTemp.createTemp(
        'framelean-engine-smoke-',
      );
      final stateDirectory = Directory('${temporaryDirectory.path}/work');
      final fixture = File('${temporaryDirectory.path}/fixture.wav');
      await fixture.writeAsBytes(_wavFixture());
      final gateway = EngineGateway(
        engineBinary: engineBinary,
        stateDirectory: stateDirectory.path,
      );
      try {
        final firstPing = await gateway.ping();
        expect(firstPing['service'], 'framelean-fll-runtime');
        expect(firstPing['abi_major'], 1);
        final endpointFile = File('${stateDirectory.path}/endpoint.json');
        final firstEndpoint =
            jsonDecode(await endpointFile.readAsString())
                as Map<String, dynamic>;
        expect(firstEndpoint['pid'], isPositive);

        final analysis = await gateway.analyzeAudio(fixture.path);
        expect(analysis['media_analysis_status'], 'complete');
        expect(analysis['analysis_id'], isA<String>());
        final media = analysis['media'] as Map<String, dynamic>;
        expect(media['kind'], 'audio');
        expect(media['provider'], 'framelean-ffmpeg');
        final descriptor = media['descriptor'] as Map<String, dynamic>;
        final audio = (descriptor['streams'] as List).single as Map;
        expect(audio['info']['codec'], 'pcm_s16le');
        expect(audio['info']['sample_rate_hz']['value'], 8000);
        expect(audio['info']['channel_count']['value'], 1);

        final reconnectedGateway = EngineGateway(
          engineBinary: engineBinary,
          stateDirectory: stateDirectory.path,
        );
        final reconnectedPing = await reconnectedGateway.ping();
        expect(reconnectedPing['service'], firstPing['service']);
        final reconnectedEndpoint =
            jsonDecode(await endpointFile.readAsString())
                as Map<String, dynamic>;
        expect(reconnectedEndpoint['pid'], firstEndpoint['pid']);
      } finally {
        try {
          await gateway.stopService();
        } on Object {
          // Preserve the original failure if the engine exited before cleanup.
        }
        await temporaryDirectory.delete(recursive: true);
      }
    },
    skip: engineBinary == null
        ? 'Set FRAMELEAN_FENGINE_BINARY after building FEngine and FLL side by side.'
        : false,
  );
}

Uint8List _wavFixture() {
  const sampleRate = 8000;
  const sampleCount = 8000;
  const dataBytes = sampleCount * 2;
  final bytes = BytesBuilder(copy: false)
    ..add(ascii.encode('RIFF'))
    ..add(_u32(36 + dataBytes))
    ..add(ascii.encode('WAVEfmt '))
    ..add(_u32(16))
    ..add(_u16(1))
    ..add(_u16(1))
    ..add(_u32(sampleRate))
    ..add(_u32(sampleRate * 2))
    ..add(_u16(2))
    ..add(_u16(16))
    ..add(ascii.encode('data'))
    ..add(_u32(dataBytes))
    ..add(Uint8List(dataBytes));
  return bytes.takeBytes();
}

Uint8List _u16(int value) =>
    (ByteData(2)..setUint16(0, value, Endian.little)).buffer.asUint8List();

Uint8List _u32(int value) =>
    (ByteData(4)..setUint32(0, value, Endian.little)).buffer.asUint8List();
