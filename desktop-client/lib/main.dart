import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'engine_gateway.dart';
import 'processing_model.dart';

void main() => runApp(const FrameLeanApp());

class FrameLeanApp extends StatelessWidget {
  const FrameLeanApp({super.key});
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'FrameLean',
    debugShowCheckedModeBanner: false,
    theme: ThemeData(
      colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff245cbe)),
      useMaterial3: true,
    ),
    home: const ProcessingPage(),
  );
}

class ProcessingPage extends StatefulWidget {
  const ProcessingPage({super.key});
  @override
  State<ProcessingPage> createState() => _ProcessingPageState();
}

class _ProcessingPageState extends State<ProcessingPage> {
  late final ProcessingModel model = ProcessingModel(EngineGateway())..start();
  static const lifecycle = MethodChannel('framelean/lifecycle');
  @override
  void initState() {
    super.initState();
    lifecycle.setMethodCallHandler((call) async {
      if (call.method == 'requestExit') await quit();
      return false;
    });
  }

  @override
  void dispose() {
    lifecycle.setMethodCallHandler(null);
    model.dispose();
    super.dispose();
  }

  Future<void> chooseInput() async {
    final file = await openFile(
      acceptedTypeGroups: [
        const XTypeGroup(label: '视频', extensions: ['mp4', 'mov']),
      ],
    );
    if (file != null) await model.analyze(file.path);
  }

  Future<void> quit() async {
    final accepted = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('退出 FrameLean'),
        content: Text(model.busy ? '退出会取消当前任务并停止后台。关闭窗口则继续处理。' : '退出会停止后台服务。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('返回'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('退出软件'),
          ),
        ],
      ),
    );
    if (accepted != true) return;
    try {
      await model.shutdown();
      if (mounted) exit(0);
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('退出失败：$e')));
      }
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: model,
    builder: (context, _) {
      const labels = {
        'idle': '等待选择视频',
        'running': '处理中',
        'validating': '校验输出',
        'succeeded': '已完成',
        'failed': '处理失败',
        'canceled': '已取消',
      };
      final media = model.media;
      final output = model.task?['output'] as String?;
      return Scaffold(
        backgroundColor: const Color(0xfff3f6f9),
        body: SafeArea(
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 900),
              child: ListView(
                padding: const EdgeInsets.all(32),
                children: [
                  Row(
                    children: [
                      const Expanded(
                        child: Text(
                          'FrameLean',
                          style: TextStyle(
                            fontSize: 30,
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                      ),
                      TextButton(
                        onPressed: model.connected ? quit : null,
                        child: const Text('退出软件'),
                      ),
                    ],
                  ),
                  const Text(
                    '技术贯通 · 本地视频压缩',
                    style: TextStyle(color: Colors.blueGrey),
                  ),
                  const SizedBox(height: 24),
                  Card(
                    child: Padding(
                      padding: const EdgeInsets.all(24),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
                            children: [
                              const Expanded(
                                child: Text(
                                  '1  选择视频',
                                  style: TextStyle(fontSize: 20),
                                ),
                              ),
                              FilledButton.tonal(
                                onPressed: model.busy || !model.connected
                                    ? null
                                    : chooseInput,
                                child: const Text('选择视频'),
                              ),
                            ],
                          ),
                          const SizedBox(height: 12),
                          SelectableText(
                            model.input.isEmpty
                                ? '支持 MP4/MOV · H.264 8-bit SDR · 无音轨或单 AAC 音轨'
                                : model.input,
                          ),
                          if (media != null)
                            Padding(
                              padding: const EdgeInsets.only(top: 16),
                              child: Text(
                                '${media['video']['width']} × ${media['video']['height']}  ·  ${(media['duration_us'] / 1000000).toStringAsFixed(2)} 秒  ·  H.264\n音轨：${media['audio'] == null ? '无' : 'AAC / ${media['audio']['sample_rate']} Hz / ${media['audio']['channels']} 声道'}',
                              ),
                            ),
                          const Divider(height: 32),
                          const Text(
                            '2  固定压缩预设',
                            style: TextStyle(fontSize: 20),
                          ),
                          const SizedBox(height: 8),
                          const Text(
                            'H.264 · CRF 23 · medium · 保持尺寸与时间戳\nAAC 音轨直接复制，输出 MP4。实际文件大小取决于源素材。',
                          ),
                          const SizedBox(height: 16),
                          Row(
                            children: [
                              Expanded(
                                child: SelectableText(
                                  model.outputDirectory.isEmpty
                                      ? '输出目录：默认与源文件同目录'
                                      : '输出目录：${model.outputDirectory}',
                                ),
                              ),
                              TextButton(
                                onPressed: model.busy
                                    ? null
                                    : () async {
                                        final path = await getDirectoryPath();
                                        if (path != null) model.setOutput(path);
                                      },
                                child: const Text('选择目录'),
                              ),
                            ],
                          ),
                          const SizedBox(height: 16),
                          Row(
                            children: [
                              FilledButton(
                                onPressed:
                                    model.busy ||
                                        !model.connected ||
                                        model.analysisId == null
                                    ? null
                                    : model.execute,
                                child: const Text('开始压缩'),
                              ),
                              const SizedBox(width: 12),
                              OutlinedButton(
                                onPressed: model.busy && model.connected
                                    ? model.cancel
                                    : null,
                                child: const Text('取消'),
                              ),
                            ],
                          ),
                        ],
                      ),
                    ),
                  ),
                  const SizedBox(height: 16),
                  Card(
                    child: Padding(
                      padding: const EdgeInsets.all(24),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            model.connected
                                ? '${labels[model.state] ?? model.state} · ${model.task?['kind'] == 'analyze' ? '媒体分析' : '视频压缩'}'
                                : '正在连接后台…',
                            style: const TextStyle(fontSize: 18),
                          ),
                          if (model.busy)
                            Padding(
                              padding: const EdgeInsets.symmetric(vertical: 16),
                              child: LinearProgressIndicator(
                                value: model.task?['kind'] == 'analyze'
                                    ? null
                                    : model.progress,
                              ),
                            ),
                          if (model.error != null)
                            Padding(
                              padding: const EdgeInsets.only(top: 12),
                              child: SelectableText(
                                model.error!,
                                style: TextStyle(
                                  color: Theme.of(context).colorScheme.error,
                                ),
                              ),
                            ),
                          if (output != null) ...[
                            const SizedBox(height: 12),
                            SelectableText('输出：$output'),
                            Text(
                              '输出 ${(model.task!['output_bytes'] / 1048576).toStringAsFixed(2)} MB；源文件 ${(model.task!['fingerprint']['size'] / 1048576).toStringAsFixed(2)} MB',
                            ),
                            TextButton(
                              onPressed: () async {
                                if (Platform.isMacOS) {
                                  await Process.run('/usr/bin/open', [
                                    '-R',
                                    output,
                                  ]);
                                } else {
                                  await Process.run('explorer.exe', [
                                    '/select,',
                                    output,
                                  ]);
                                }
                              },
                              child: const Text('打开结果位置'),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ),
                  const Padding(
                    padding: EdgeInsets.only(top: 16),
                    child: Text(
                      '原文件始终保留。关闭窗口后后台继续处理，重新打开可恢复进度。\n当前阶段一次处理一个视频；后台重启后需重新分析。',
                      style: TextStyle(color: Colors.blueGrey),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
    },
  );
}
