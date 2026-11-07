import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'native_engine.dart';

bool validClipboardText(String value) {
  if (value.length > 8192) return false;
  for (var i = 0; i < value.length; ++i) {
    final unit = value.codeUnitAt(i);
    if ((unit < 32 && unit != 9 && unit != 10 && unit != 13) ||
        (unit >= 127 && unit <= 159)) {
      return false;
    }
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (++i >= value.length) return false;
      final next = value.codeUnitAt(i);
      if (next < 0xdc00 || next > 0xdfff) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return false;
    }
  }
  return utf8.encode(value).length <= 8192;
}

class ClipboardSheet extends StatefulWidget {
  const ClipboardSheet({
    super.key,
    required this.engine,
    required this.sessionId,
    required this.current,
  });
  final NativeEngine engine;
  final String sessionId;
  final bool Function() current;
  @override
  State<ClipboardSheet> createState() => _ClipboardSheetState();
}

class _ClipboardSheetState extends State<ClipboardSheet> {
  final text = TextEditingController();
  bool busy = false;
  String message = '';
  @override
  void dispose() {
    text.dispose();
    super.dispose();
  }

  Future<void> act(String operation) async {
    if (busy) return;
    if (!widget.current()) {
      setState(() => message = 'Phiên điều khiển đã kết thúc.');
      return;
    }
    final id = widget.sessionId;
    final snapshot = text.text;
    if ((operation == 'write' || operation == 'copy') &&
        !validClipboardText(snapshot)) {
      setState(() => message = 'Clipboard hỗ trợ tối đa 8 KB văn bản.');
      return;
    }
    setState(() {
      busy = true;
      message = '';
    });
    bool current() => mounted && id == widget.sessionId && widget.current();
    try {
      if (operation == 'paste') {
        final local = await Clipboard.getData(Clipboard.kTextPlain);
        if (!validClipboardText(local?.text ?? '')) {
          throw const FormatException('Clipboard hỗ trợ tối đa 8 KB văn bản.');
        }
        if (current()) text.text = local?.text ?? '';
      } else if (operation == 'copy') {
        await Clipboard.setData(ClipboardData(text: snapshot));
        if (current()) setState(() => message = 'Đã sao chép trên điện thoại.');
      } else {
        final received = await widget.engine.clipboard(
          id,
          write: operation == 'write',
          text: operation == 'write' ? snapshot : '',
        );
        if (current()) {
          if (operation == 'read') text.text = received;
          setState(
            () => message = operation == 'read'
                ? 'Đã nhận. Bạn có thể xem và sao chép vào điện thoại.'
                : 'Đã cập nhật clipboard trên máy tính.',
          );
        }
      }
    } catch (error) {
      if (current()) {
        setState(
          () => message = error is PlatformException
              ? error.message ?? 'Không thể truy cập clipboard.'
              : error.toString(),
        );
      }
    } finally {
      if (mounted) {
        setState(() {
          busy = false;
          if (!widget.current()) message = 'Phiên điều khiển đã kết thúc.';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) => SingleChildScrollView(
    padding: EdgeInsets.fromLTRB(
      20,
      24,
      20,
      MediaQuery.viewInsetsOf(context).bottom + 24,
    ),
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Text(
          'Clipboard',
          style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
        ),
        const SizedBox(height: 8),
        const Text(
          'Gửi và nhận văn bản thủ công. Nội dung nhận về chỉ được lưu vào clipboard điện thoại khi bạn bấm Sao chép.',
        ),
        const SizedBox(height: 16),
        TextField(
          controller: text,
          enabled: !busy,
          minLines: 3,
          maxLines: 6,
          maxLength: 8192,
          onChanged: (_) => setState(() {}),
          decoration: const InputDecoration(labelText: 'Văn bản · tối đa 8 KB'),
        ),
        const SizedBox(height: 12),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            FilledButton.icon(
              onPressed: busy ? null : () => act('read'),
              icon: const Icon(Icons.download_outlined),
              label: const Text('Nhận từ máy tính'),
            ),
            OutlinedButton.icon(
              onPressed:
                  busy || text.text.isEmpty || !validClipboardText(text.text)
                  ? null
                  : () => act('write'),
              icon: const Icon(Icons.upload_outlined),
              label: const Text('Gửi đến máy tính'),
            ),
            TextButton(
              onPressed: busy ? null : () => act('paste'),
              child: const Text('Dán từ điện thoại'),
            ),
            TextButton(
              onPressed: busy || text.text.isEmpty ? null : () => act('copy'),
              child: const Text('Sao chép vào điện thoại'),
            ),
          ],
        ),
        if (busy)
          const Padding(
            padding: EdgeInsets.only(top: 12),
            child: LinearProgressIndicator(),
          ),
        if (message.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 12),
            child: Text(message, semanticsLabel: message),
          ),
        Align(
          alignment: Alignment.centerRight,
          child: TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Đóng'),
          ),
        ),
      ],
    ),
  );
}
