import 'dart:convert';

import 'package:flutter/material.dart';

import 'package:framelean/engine_gateway.dart';

void main() {
  runApp(const FrameLeanApp());
}

class FrameLeanApp extends StatelessWidget {
  const FrameLeanApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'FrameLean',
      theme: ThemeData(useMaterial3: true, fontFamily: 'AlibabaPuHuiTi'),
      home: const EngineSmokePage(),
    );
  }
}

class EngineSmokePage extends StatefulWidget {
  const EngineSmokePage({super.key});

  @override
  State<EngineSmokePage> createState() => _EngineSmokePageState();
}

class _EngineSmokePageState extends State<EngineSmokePage> {
  final EngineGateway _gateway = EngineGateway();
  final TextEditingController _pathController = TextEditingController();
  bool _busy = false;
  String _status = '尚未连接 Work';
  String _result = '';

  @override
  void dispose() {
    _pathController.dispose();
    super.dispose();
  }

  Future<void> _run(Future<Map<String, dynamic>> Function() action) async {
    setState(() {
      _busy = true;
      _result = '';
    });
    try {
      final result = await action();
      if (!mounted) return;
      setState(() {
        _status = 'Work 与 FLL 已连接';
        _result = const JsonEncoder.withIndent('  ').convert(result);
      });
    } on Object catch (error) {
      if (!mounted) return;
      setState(() {
        _status = '连接失败';
        _result = error.toString();
      });
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 760),
          child: Padding(
            padding: const EdgeInsets.all(32),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'FrameLean',
                  style: Theme.of(context).textTheme.headlineMedium,
                ),
                const SizedBox(height: 8),
                const Text('重启主线：Flutter → Work → FLL'),
                const SizedBox(height: 24),
                Text('Work 状态：$_status'),
                const SizedBox(height: 16),
                Wrap(
                  spacing: 12,
                  children: [
                    FilledButton(
                      onPressed: _busy ? null : () => _run(_gateway.ping),
                      child: const Text('连接并检查引擎'),
                    ),
                  ],
                ),
                const SizedBox(height: 24),
                TextField(
                  controller: _pathController,
                  decoration: const InputDecoration(
                    labelText: '音频 fixture 的完整路径',
                    hintText: '/path/to/sample.wav',
                    border: OutlineInputBorder(),
                  ),
                  enabled: !_busy,
                  onChanged: (_) => setState(() {}),
                ),
                const SizedBox(height: 12),
                FilledButton.tonal(
                  onPressed: _busy || _pathController.text.trim().isEmpty
                      ? null
                      : () => _run(
                          () => _gateway.analyzeAudio(
                            _pathController.text.trim(),
                          ),
                        ),
                  child: const Text('通过引擎分析音频'),
                ),
                if (_result.isNotEmpty) ...[
                  const SizedBox(height: 20),
                  Flexible(
                    child: SingleChildScrollView(
                      child: SelectableText(_result),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
