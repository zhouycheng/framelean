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
  State<ProcessingPage> createState() => _Page();
}

class _Page extends State<ProcessingPage> {
  final model = ProcessingModel(EngineGateway());
  final selected = <String>{};
  String? selectedPreset;
  static const channel = MethodChannel('framelean/lifecycle');
  static const labels = {
    'queued': '等待处理',
    'running': '处理中',
    'validating': '校验输出',
    'publishing': '正在完成',
    'reconciling': '等待核对结果',
    'succeeded': '已完成',
    'failed': '失败',
    'canceled': '已取消',
    'interrupted': '已中断',
  };
  @override
  void initState() {
    super.initState();
    model.start();
    channel.setMethodCallHandler((c) async {
      if (c.method == 'requestExit') await quit();
      return false;
    });
  }

  @override
  void dispose() {
    channel.setMethodCallHandler(null);
    model.dispose();
    super.dispose();
  }

  Future<void> quit() async {
    final yes = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: const Text('退出 FrameLean'),
        content: const Text('停止当前处理并暂停队列。等待任务会保留，下次启动后可继续。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(c, false),
            child: const Text('返回'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(c, true),
            child: const Text('退出软件'),
          ),
        ],
      ),
    );
    if (yes != true) return;
    try {
      await model.shutdown();
      exit(0);
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('$e')));
      }
    }
  }

  String mediaDescription(dynamic m) {
    if (m is! Map) return '';
    final tracks = (m['tracks'] as List? ?? []).cast<Map>();
    final videos = tracks.where((t) => t['kind'] == 'video').toList();
    final audio = tracks.where((t) => t['kind'] == 'audio').toList();
    final v = videos.isEmpty ? null : videos.first['video'];
    final duration = m['duration_us']?['value'] as num?;
    return '${v?['width']?['value'] ?? '?'} × ${v?['height']?['value'] ?? '?'} · ${duration == null ? '时长未知' : '${(duration / 1000000).toStringAsFixed(2)} 秒'} · ${videos.isEmpty ? '无视频轨' : videos.first['codec']?['value'] ?? '编码未知'} · ${audio.isEmpty ? '无音轨' : '${audio.length} 条音轨'}';
  }

  Future<void> history(String id) async {
    try {
      final detail = await model.gateway.call('tasks/$id');
      if (!mounted) return;
      await showDialog<void>(
        context: context,
        builder: (c) => AlertDialog(
          title: Text('执行历史 · 共 ${detail['attempt_count']} 次'),
          content: SizedBox(
            width: 600,
            child: SingleChildScrollView(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final e in detail['attempts'] as List)
                    SelectableText(
                      '第 ${e['attempt_number']} 次 · ${labels[e['status']] ?? e['status']}\n${e['error_json']?['message'] ?? ''}',
                    ),
                  if (detail['next_cursor'] != null)
                    const Text('此处显示最近 50 次；完整历史可通过 CLI 分页查询。'),
                ],
              ),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(c),
              child: const Text('关闭'),
            ),
          ],
        ),
      );
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('$e')));
      }
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: model,
    builder: (context, _) {
      final ready = model
          .rows('analyses')
          .where(
            (a) =>
                a['status'] == 'succeeded' &&
                model.canProcess(a['id'] as String),
          )
          .map((a) => a['id'] as String)
          .toSet();
      selected.removeWhere((id) => !ready.contains(id));
      final presets = model.presetsFor(selected);
      if (!presets.any((p) => p['id'] == selectedPreset)) selectedPreset = null;
      return Scaffold(
        appBar: AppBar(
          title: const Text('FrameLean · 批量压缩'),
          actions: [TextButton(onPressed: quit, child: const Text('退出软件'))],
        ),
        body: ListView(
          padding: const EdgeInsets.all(24),
          children: [
            Text(
              model.connected
                  ? '后台已连接 · ${model.paused ? '队列已暂停' : '队列运行中'}'
                  : '正在连接后台',
            ),
            const Text('先分析媒体，再选择可用的压缩方案。关闭窗口后后台继续；重启后可恢复等待队列。'),
            if (model.error != null)
              SelectableText(
                model.error!,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            Wrap(
              spacing: 12,
              children: [
                FilledButton(
                  onPressed: !model.connected || model.submitting
                      ? null
                      : () async {
                          final files = await openFiles();
                          await model.analyze(
                            files.map((f) => f.path).toList(),
                          );
                        },
                  child: const Text('添加视频'),
                ),
                OutlinedButton(
                  onPressed: !model.connected || model.submitting
                      ? null
                      : () => model.command(
                          model.paused ? 'queue/resume' : 'queue/pause',
                          {},
                        ),
                  child: Text(model.paused ? '继续队列' : '暂停队列'),
                ),
                TextButton(
                  onPressed: () async {
                    final p = await getDirectoryPath();
                    if (p != null) model.setOutput(p);
                  },
                  child: const Text('选择输出目录'),
                ),
              ],
            ),
            SelectableText(
              '输出目录：${model.outputDirectory.isEmpty ? '选择文件后默认为同目录' : model.outputDirectory}',
            ),
            const SizedBox(height: 16),
            Text('分析列表', style: Theme.of(context).textTheme.titleLarge),
            for (final a in model.rows('analyses'))
              if (a['status'] == 'succeeded' &&
                  !model.canProcess(a['id'] as String))
                ListTile(
                  title: Text('${a['input']}'),
                  subtitle: Text(
                    '${mediaDescription(a['media_json'])}\n${model.unavailableReason(a['id'] as String)}',
                  ),
                )
              else
                CheckboxListTile(
                  value: selected.contains(a['id']),
                  onChanged: a['status'] != 'succeeded'
                      ? null
                      : (v) => setState(() {
                          if (v == true) {
                            selected.add(a['id'] as String);
                          } else {
                            selected.remove(a['id']);
                          }
                        }),
                  title: Text('${a['input']}'),
                  subtitle: Text(
                    '${labels[a['status']] ?? a['status']}\n${mediaDescription(a['media_json'])} ${a['error_json']?['message'] ?? ''}',
                  ),
                  secondary:
                      a['status'] == 'interrupted' || a['status'] == 'failed'
                      ? IconButton(
                          tooltip: '重新分析',
                          onPressed: () =>
                              model.analyze([a['input'] as String]),
                          icon: const Icon(Icons.refresh),
                        )
                      : null,
                ),
            if (model.analysisCursor != null)
              TextButton(
                onPressed: () => model.loadMore('analyses'),
                child: const Text('更多媒体'),
              ),
            if (selected.isNotEmpty)
              Row(
                children: [
                  const Text('压缩预设：'),
                  DropdownButton<String>(
                    value:
                        selectedPreset ?? presets.firstOrNull?['id'] as String?,
                    items: presets
                        .map(
                          (p) => DropdownMenuItem<String>(
                            value: p['id'] as String,
                            child: Text(p['label'] as String),
                          ),
                        )
                        .toList(),
                    onChanged: model.submitting
                        ? null
                        : (value) => setState(() => selectedPreset = value),
                  ),
                ],
              ),
            if (selected.isNotEmpty)
              FilledButton(
                onPressed: presets.isEmpty || model.submitting
                    ? null
                    : () async {
                        await model.enqueue(
                          selected.toList(),
                          preset: selectedPreset,
                        );
                        if (model.error == null && mounted) {
                          setState(selected.clear);
                        }
                      },
                child: Text('将选中 ${selected.length} 项加入压缩队列'),
              ),
            const Text('保持原尺寸和帧率。预设表示画质与体积取舍，输出可能大于原文件。'),
            const SizedBox(height: 24),
            Text('任务与执行历史', style: Theme.of(context).textTheme.titleLarge),
            for (final t in model.rows('tasks')) ...[
              Text(
                '${t['input']}',
                style: Theme.of(context).textTheme.titleMedium,
              ),
              Text(
                '操作：${t['intent_json']?['operation'] ?? 'compress'} · 方案：${t['plan_json']?['parameters']?['mode'] ?? '历史方案'} · 输出：${t['plan_json']?['output']?['container'] ?? 'mp4'}',
              ),
              for (final e
                  in model
                      .rows('attempts')
                      .where((e) => e['task_id'] == t['id']))
                Card(
                  child: Padding(
                    padding: const EdgeInsets.all(12),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '第 ${e['attempt_number']} 次 · ${e['cancel_requested_at'] != null && ['running', 'validating'].contains(e['status']) ? '取消中' : labels[e['status']] ?? e['status']}',
                        ),
                        if (model.snapshot['active']?['id'] == e['id'])
                          LinearProgressIndicator(
                            value:
                                (model.snapshot['active']?['event']?['progress']
                                        as num?)
                                    ?.toDouble(),
                          ),
                        if (e['error_json'] != null)
                          Text('${e['error_json']?['message'] ?? ''}'),
                        if ([
                              'queued',
                              'running',
                              'validating',
                            ].contains(e['status']) &&
                            e['cancel_requested_at'] == null)
                          TextButton(
                            onPressed: () => model.command('cancel', {
                              'attempt_id': e['id'],
                            }),
                            child: const Text('取消'),
                          ),
                        if (e['status'] == 'reconciling')
                          TextButton(
                            onPressed: () => model.command('reconcile', {
                              'attempt_id': e['id'],
                            }),
                            child: const Text('重新核对结果'),
                          ),
                        for (final o
                            in model
                                .rows('outputs')
                                .where((o) => o['attempt_id'] == e['id']))
                          Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              SelectableText(
                                '${o['status'] == 'succeeded' ? '结果' : '发布记录'}：${o['final_path']}\n输出 ${o['size_bytes']} 字节；源文件 ${t['source_fingerprint']?['size']} 字节',
                              ),
                              if (o['status'] == 'succeeded')
                                TextButton(
                                  onPressed: () async {
                                    try {
                                      final path = o['final_path'] as String;
                                      final result = await Process.run(
                                        Platform.isMacOS
                                            ? 'open'
                                            : 'explorer.exe',
                                        Platform.isMacOS
                                            ? ['-R', path]
                                            : ['/select,', path],
                                      );
                                      if (result.exitCode != 0) {
                                        throw StateError('无法打开结果位置');
                                      }
                                    } catch (error) {
                                      if (context.mounted) {
                                        ScaffoldMessenger.of(context)
                                            .showSnackBar(
                                              SnackBar(content: Text('$error')),
                                            );
                                      }
                                    }
                                  },
                                  child: const Text('打开结果位置'),
                                ),
                            ],
                          ),
                      ],
                    ),
                  ),
                ),
              TextButton(
                onPressed: () => history(t['id'] as String),
                child: const Text('查看执行历史'),
              ),
              if (model
                      .rows('attempts')
                      .where((e) => e['task_id'] == t['id'])
                      .isNotEmpty &&
                  ['failed', 'canceled', 'interrupted'].contains(
                    model
                        .rows('attempts')
                        .lastWhere((e) => e['task_id'] == t['id'])['status'],
                  ))
                TextButton(
                  onPressed: () => model.command('retry', {'task_id': t['id']}),
                  child: const Text('重试任务'),
                ),
            ],
            if (model.taskCursor != null)
              TextButton(
                onPressed: () => model.loadMore('tasks'),
                child: const Text('更多任务'),
              ),
          ],
        ),
      );
    },
  );
}
