import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'api.dart';
import 'native_engine.dart';

/// Metadata only. The shared Rust worker owns source/destination files and SHA-256.
class MobileFiles extends ChangeNotifier {
  MobileFiles(
    this.engine,
    this.sessionId, {
    required this.current,
    bool start = true,
  }) {
    if (start) {
      timer = Timer.periodic(
        const Duration(milliseconds: 500),
        (_) => unawaited(refresh()),
      );
      unawaited(refresh());
    }
  }
  final NativeEngine engine;
  final String sessionId;
  final bool Function() current;
  Timer? timer;
  Json state = {};
  Json? selection;
  String error = '';
  bool busy = false, polling = false, closed = false;
  int revision = 0;
  bool get available => state['available'] == true && current();
  Json? get incoming => state['incoming'] is Map
      ? Map<String, dynamic>.from(state['incoming'] as Map)
      : null;
  Json? get outgoing => state['outgoing'] is Map
      ? Map<String, dynamic>.from(state['outgoing'] as Map)
      : null;
  bool get invitation => available && incoming?['state'] == 'offered';
  bool get uploadActive =>
      ['offered', 'sending', 'verifying'].contains(outgoing?['state']);
  Future<void> refresh() async {
    if (closed || polling || busy || !current()) return;
    polling = true;
    final version = revision;
    try {
      final next = await engine.files(sessionId, {'operation': 'status'});
      if (!closed && current() && version == revision && !busy) {
        state = next;
        notifyListeners();
      }
    } catch (_) {
      if (!closed && current() && version == revision && !busy) {
        state = {...state, 'available': false};
        notifyListeners();
      }
    } finally {
      polling = false;
    }
  }

  Future<void> pick() async {
    if (!available || busy || uploadActive) return;
    revision++;
    busy = true;
    error = '';
    notifyListeners();
    try {
      final next = await engine.pickFile(sessionId);
      if (!closed && current()) selection = next;
    } catch (e) {
      if (!closed && current()) error = message(e);
    } finally {
      if (!closed) {
        busy = false;
        notifyListeners();
      }
    }
  }

  Future<void> act(Json action) async {
    if (!available || busy) return;
    revision++;
    busy = true;
    error = '';
    notifyListeners();
    try {
      final next = await engine.files(sessionId, action);
      if (!closed && current()) {
        state = next;
        if (action['operation'] == 'offer') selection = null;
      }
    } catch (e) {
      if (!closed && current()) error = message(e);
    } finally {
      if (!closed) {
        busy = false;
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    closed = true;
    timer?.cancel();
    super.dispose();
  }

  static String message(Object error) => error is PlatformException
      ? error.message ?? 'Không thể xử lý file.'
      : error.toString();
}

String fileState(dynamic state) =>
    const {
      'offered': 'Chờ chấp nhận',
      'sending': 'Đang gửi',
      'receiving': 'Đang nhận',
      'verifying': 'Đang kiểm tra',
      'completed': 'Hoàn tất',
      'cancelled': 'Đã hủy',
    }[state] ??
    'Đang chờ';

String fileSize(dynamic bytes) {
  final value = bytes is num ? bytes.toDouble() : 0.0;
  return value < 1024
      ? '${value.toInt()} B'
      : value < 1048576
      ? '${(value / 1024).toStringAsFixed(1)} KB'
      : '${(value / 1048576).toStringAsFixed(1)} MB';
}

Future<void> showReceivedFiles(BuildContext context, NativeEngine engine) =>
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => ReceivedFiles(engine: engine),
    );

class ReceivedFiles extends StatefulWidget {
  const ReceivedFiles({super.key, required this.engine});
  final NativeEngine engine;
  @override
  State<ReceivedFiles> createState() => _ReceivedFilesState();
}

class _ReceivedFilesState extends State<ReceivedFiles> {
  List<Json> items = [];
  bool busy = false;
  String error = '';
  @override
  void initState() {
    super.initState();
    unawaited(load());
  }

  Future<void> load() async {
    if (busy) return;
    setState(() {
      busy = true;
      error = '';
    });
    try {
      final next = await widget.engine.fileInbox();
      if (mounted) setState(() => items = next);
    } catch (e) {
      if (mounted) setState(() => error = MobileFiles.message(e));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> save(Json item) async {
    if (busy) return;
    setState(() {
      busy = true;
      error = '';
    });
    try {
      await widget.engine.exportFile(item['key'] as String);
    } catch (e) {
      if (mounted) setState(() => error = MobileFiles.message(e));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) => SafeArea(
    child: SizedBox(
      height: MediaQuery.sizeOf(context).height * .75,
      child: Column(
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
            child: Row(
              children: [
                const Expanded(
                  child: Text(
                    'Tệp đã nhận',
                    style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
                  ),
                ),
                IconButton(
                  onPressed: busy ? null : load,
                  tooltip: 'Làm mới',
                  icon: const Icon(Icons.refresh),
                ),
              ],
            ),
          ),
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 20),
            child: Text(
              'File đã kiểm tra SHA-256, còn lưu trong Sanser sau khi ngắt kết nối. Bấm Lưu để chọn nơi xuất file.',
            ),
          ),
          if (busy) const LinearProgressIndicator(),
          if (error.isNotEmpty)
            Padding(padding: const EdgeInsets.all(12), child: Text(error)),
          Expanded(
            child: items.isEmpty && !busy
                ? const Center(child: Text('Chưa có file đã nhận.'))
                : ListView.builder(
                    itemCount: items.length,
                    itemBuilder: (_, index) {
                      final item = items[index];
                      return ListTile(
                        leading: const Icon(Icons.insert_drive_file_outlined),
                        title: Text(item['name'] as String? ?? 'File'),
                        subtitle: Text(fileSize(item['size'])),
                        trailing: TextButton(
                          onPressed: busy ? null : () => save(item),
                          child: const Text('Lưu'),
                        ),
                      );
                    },
                  ),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Đóng'),
          ),
        ],
      ),
    ),
  );
}

class FileTransfersSheet extends StatelessWidget {
  const FileTransfersSheet({super.key, required this.files});
  final MobileFiles files;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: files,
    builder: (context, _) => SafeArea(
      child: SingleChildScrollView(
        padding: EdgeInsets.fromLTRB(
          20,
          16,
          20,
          MediaQuery.viewInsetsOf(context).bottom + 24,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const Text(
              'Truyền file',
              style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 8),
            const Text(
              'Tối đa 25 MB mỗi file. Máy nhận phải chấp nhận; chỉ lưu file hoàn chỉnh sau khi kiểm tra SHA-256.',
            ),
            const SizedBox(height: 12),
            if (!files.available)
              const Text(
                'Cần host desktop hỗ trợ truyền file và phiên có quyền điều khiển.',
              ),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                OutlinedButton.icon(
                  onPressed:
                      files.available && !files.busy && !files.uploadActive
                      ? files.pick
                      : null,
                  icon: const Icon(Icons.attach_file),
                  label: const Text('Chọn file'),
                ),
                FilledButton.icon(
                  onPressed:
                      files.available &&
                          !files.busy &&
                          !files.uploadActive &&
                          files.selection != null
                      ? () => files.act({
                          'operation': 'offer',
                          'sourceId': files.selection!['sourceId'],
                          'name': files.selection!['name'],
                        })
                      : null,
                  icon: const Icon(Icons.upload_outlined),
                  label: const Text('Mời nhận file'),
                ),
              ],
            ),
            if (files.selection != null)
              Text(
                '${files.selection!['name']} · ${fileSize(files.selection!['size'])}',
              ),
            if (files.busy) const LinearProgressIndicator(),
            for (final row in [
              (label: 'Nhận', value: files.incoming),
              (label: 'Gửi', value: files.outgoing),
            ])
              if (row.value != null)
                Card(
                  child: Padding(
                    padding: const EdgeInsets.all(16),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Text(
                          '${row.label}: ${row.value!['name']}',
                          style: const TextStyle(fontWeight: FontWeight.w600),
                        ),
                        Text(
                          '${fileState(row.value!['state'])} · ${fileSize(row.value!['bytes'])} / ${fileSize(row.value!['size'])}',
                        ),
                        if (row.value!['state'] != 'offered')
                          LinearProgressIndicator(
                            value: row.value!['state'] == 'completed'
                                ? 1
                                : (row.value!['size'] as num? ?? 0) > 0
                                ? ((row.value!['bytes'] as num? ?? 0) /
                                          (row.value!['size'] as num))
                                      .clamp(0, 1)
                                      .toDouble()
                                : 0,
                          ),
                        if (row.value!['error'] is String)
                          Text(row.value!['error'] as String),
                        Wrap(
                          spacing: 8,
                          children: [
                            if (row.label == 'Nhận' &&
                                row.value!['state'] == 'offered')
                              FilledButton(
                                onPressed: files.available && !files.busy
                                    ? () => files.act({
                                        'operation': 'accept',
                                        'id': row.value!['id'],
                                      })
                                    : null,
                                child: const Text('Chấp nhận'),
                              ),
                            if ([
                              'offered',
                              'sending',
                              'receiving',
                              'verifying',
                            ].contains(row.value!['state']))
                              TextButton(
                                onPressed: files.available && !files.busy
                                    ? () => files.act({
                                        'operation': 'cancel',
                                        'id': row.value!['id'],
                                      })
                                    : null,
                                child: Text(
                                  row.label == 'Nhận' &&
                                          row.value!['state'] == 'offered'
                                      ? 'Từ chối'
                                      : 'Hủy truyền',
                                ),
                              ),
                          ],
                        ),
                        if (row.value!['state'] == 'completed' &&
                            row.label == 'Nhận')
                          const Text(
                            'Đã nhận và xác minh. Mở Tệp đã nhận để lưu ra thiết bị.',
                          ),
                      ],
                    ),
                  ),
                ),
            if (files.error.isNotEmpty) Text(files.error),
            const SizedBox(height: 12),
            OutlinedButton.icon(
              onPressed: () => showReceivedFiles(context, files.engine),
              icon: const Icon(Icons.folder_open),
              label: const Text('Tệp đã nhận'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Đóng'),
            ),
          ],
        ),
      ),
    ),
  );
}
