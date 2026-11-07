import 'dart:async';
import 'package:flutter/material.dart';
import 'mobile_input.dart';
import 'session_actions.dart';
import 'design_tokens.dart';

enum ShortcutHost { windows, macos }

/// Match declared platform values, never the phone OS or a device name.
ShortcutHost? shortcutHost(String? platform) =>
    switch (platform?.trim().toLowerCase()) {
      'windows' || 'win32' || 'win64' => ShortcutHost.windows,
      'macos' || 'darwin' || 'mac' || 'osx' => ShortcutHost.macos,
      _ => null,
    };

class HostShortcut {
  final String id, label, keys, group;
  final List<int> codes;
  final IconData icon;
  const HostShortcut(
    this.id,
    this.label,
    this.keys,
    this.group,
    this.codes,
    this.icon,
  );
}

/// Physical HID shortcuts on the existing authenticated keyboard lane. These
/// are requests to the foreground OS/app, not window-management API results.
List<HostShortcut> hostShortcuts(ShortcutHost host) {
  final mac = host == ShortcutHost.macos, modifier = mac ? 227 : 224;
  final name = mac ? 'Cmd+' : 'Ctrl+';
  return List.unmodifiable([
    HostShortcut(
      'switch',
      'Đổi ứng dụng',
      mac ? 'Cmd+Tab' : 'Alt+Tab',
      'Cửa sổ',
      [mac ? 227 : 226, 43],
      Icons.swap_horiz,
    ),
    HostShortcut(
      'previous',
      'Ứng dụng trước',
      mac ? 'Cmd+Shift+Tab' : 'Alt+Shift+Tab',
      'Cửa sổ',
      [mac ? 227 : 226, 225, 43],
      Icons.keyboard_double_arrow_left,
    ),
    if (mac) ...[
      const HostShortcut('minimize', 'Thu nhỏ cửa sổ', 'Cmd+M', 'Cửa sổ', [
        227,
        16,
      ], Icons.minimize),
      const HostShortcut(
        'minimize-app',
        'Thu nhỏ các cửa sổ app',
        'Option+Cmd+M',
        'Cửa sổ',
        [226, 227, 16],
        Icons.filter_none,
      ),
      const HostShortcut(
        'fullscreen',
        'Toàn màn hình',
        'Ctrl+Cmd+F',
        'Cửa sổ',
        [224, 227, 9],
        Icons.fullscreen,
      ),
      const HostShortcut('hide', 'Ẩn ứng dụng', 'Cmd+H', 'Cửa sổ', [
        227,
        11,
      ], Icons.visibility_off_outlined),
      const HostShortcut('search', 'Spotlight', 'Cmd+Space', 'Cửa sổ', [
        227,
        44,
      ], Icons.search),
    ] else ...[
      const HostShortcut('desktop', 'Hiện desktop', 'Win+D', 'Cửa sổ', [
        227,
        7,
      ], Icons.desktop_windows_outlined),
      const HostShortcut('overview', 'Task View', 'Win+Tab', 'Cửa sổ', [
        227,
        43,
      ], Icons.view_carousel_outlined),
      const HostShortcut('snap-left', 'Gắn cửa sổ trái', 'Win+←', 'Cửa sổ', [
        227,
        80,
      ], Icons.align_horizontal_left),
      const HostShortcut('snap-right', 'Gắn cửa sổ phải', 'Win+→', 'Cửa sổ', [
        227,
        79,
      ], Icons.align_horizontal_right),
      const HostShortcut('minimize', 'Thu nhỏ tất cả', 'Win+M', 'Cửa sổ', [
        227,
        16,
      ], Icons.minimize),
      const HostShortcut('explorer', 'File Explorer', 'Win+E', 'Cửa sổ', [
        227,
        8,
      ], Icons.folder_open_outlined),
    ],
    HostShortcut(
      'close',
      'Đóng cửa sổ',
      mac ? 'Cmd+W' : 'Alt+F4',
      'Cửa sổ',
      mac ? const [227, 26] : const [226, 61],
      Icons.close,
    ),
    HostShortcut('copy', 'Sao chép', '${name}C', 'Chỉnh sửa', [
      modifier,
      6,
    ], Icons.copy_outlined),
    HostShortcut('paste', 'Dán', '${name}V', 'Chỉnh sửa', [
      modifier,
      25,
    ], Icons.content_paste),
    HostShortcut('cut', 'Cắt', '${name}X', 'Chỉnh sửa', [
      modifier,
      27,
    ], Icons.content_cut),
    HostShortcut('undo', 'Hoàn tác', '${name}Z', 'Chỉnh sửa', [
      modifier,
      29,
    ], Icons.undo),
    HostShortcut(
      'redo',
      'Làm lại',
      mac ? 'Cmd+Shift+Z' : 'Ctrl+Y',
      'Chỉnh sửa',
      mac ? const [227, 225, 29] : const [224, 28],
      Icons.redo,
    ),
    HostShortcut('select-all', 'Chọn tất cả', '${name}A', 'Chỉnh sửa', [
      modifier,
      4,
    ], Icons.select_all),
    HostShortcut('find', 'Tìm trong ứng dụng', '${name}F', 'Chỉnh sửa', [
      modifier,
      9,
    ], Icons.find_in_page_outlined),
    HostShortcut('save', 'Lưu', '${name}S', 'Chỉnh sửa', [
      modifier,
      22,
    ], Icons.save_outlined),
  ]);
}

class HostShortcutsSheet extends StatefulWidget {
  final MobileInput input;
  final bool Function() current;
  final ShortcutHost? detectedHost;
  const HostShortcutsSheet({
    super.key,
    required this.input,
    required this.current,
    this.detectedHost,
  });
  @override
  State<HostShortcutsSheet> createState() => _HostShortcutsSheetState();
}

class _HostShortcutsSheetState extends State<HostShortcutsSheet>
    with WidgetsBindingObserver {
  ShortcutHost? host;
  bool busy = false, foreground = true;
  int revision = 0;
  String message = '';
  bool get current =>
      foreground &&
      widget.input.enabled &&
      widget.current() &&
      (ModalRoute.of(context)?.isCurrent ?? false);
  @override
  void initState() {
    super.initState();
    host = widget.detectedHost;
    final lifecycle = WidgetsBinding.instance.lifecycleState;
    foreground = lifecycle == null || lifecycle == AppLifecycleState.resumed;
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didUpdateWidget(covariant HostShortcutsSheet oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.input != widget.input) {
      revision++;
      unawaited(oldWidget.input.releaseAll());
      host = widget.detectedHost;
      busy = false;
      message = '';
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    foreground = state == AppLifecycleState.resumed;
    if (!foreground) {
      revision++;
      unawaited(widget.input.releaseAll());
    }
    // Keep an in-flight request busy until its cleanup finishes. A lifecycle
    // resume cannot make a half-sent chord start again.
    if (mounted) setState(() => message = '');
  }

  @override
  void dispose() {
    revision++;
    WidgetsBinding.instance.removeObserver(this);
    unawaited(widget.input.releaseAll());
    super.dispose();
  }

  Future<void> run(HostShortcut action) async {
    if (busy || !current || host == null) return;
    final input = widget.input, attempt = ++revision;
    bool valid() =>
        mounted && widget.input == input && revision == attempt && current;
    setState(() {
      busy = true;
      message = '';
    });
    try {
      final queued = await input.chord(action.codes, current: valid);
      if (valid()) {
        setState(
          () => message = queued
              ? 'Đã gửi ${action.keys}. Kết quả phụ thuộc ứng dụng trên máy tính.'
              : 'Chưa gửi trọn tổ hợp phím. Thả các phím đang giữ và kiểm tra kết nối.',
        );
      }
    } finally {
      if (mounted && widget.input == input) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) => SafeArea(
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(20, 8, 8, 0),
          child: Row(
            children: [
              const Expanded(
                child: Text(
                  'Phím tắt máy tính',
                  style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
                ),
              ),
              IconButton(
                tooltip: 'Đóng bảng phím tắt',
                onPressed: () => Navigator.pop(context),
                icon: const Icon(Icons.close),
              ),
            ],
          ),
        ),
        Flexible(
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 24),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  widget.detectedHost == null
                      ? 'Chọn hệ điều hành của máy đang được điều khiển.'
                      : 'Bộ phím theo máy được điều khiển. Bạn có thể đổi nếu thông tin thiết bị chưa đúng.',
                ),
                const SizedBox(height: 12),
                SegmentedButton<ShortcutHost>(
                  emptySelectionAllowed: true,
                  segments: const [
                    ButtonSegment(
                      value: ShortcutHost.windows,
                      label: Text('Windows'),
                      icon: Icon(Icons.desktop_windows_outlined),
                    ),
                    ButtonSegment(
                      value: ShortcutHost.macos,
                      label: Text('macOS'),
                      icon: Icon(Icons.laptop_mac),
                    ),
                  ],
                  selected: {?host},
                  onSelectionChanged: busy || !current
                      ? null
                      : (value) => setState(() {
                          host = value.firstOrNull;
                          message = '';
                        }),
                ),
                const SizedBox(height: 12),
                const Text(
                  'Tác động lên cửa sổ đang chọn ở máy tính. Sao chép/Dán dùng clipboard của máy tính.',
                ),
                if (!current) const Text('Phiên chưa sẵn sàng nhận phím.'),
                if (busy) ...[
                  const SizedBox(height: 8),
                  const LinearProgressIndicator(),
                  const Text('Đang gửi tổ hợp phím…'),
                ],
                if (message.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Semantics(liveRegion: true, child: Text(message)),
                  ),
                if (host != null)
                  for (final group in const ['Cửa sổ', 'Chỉnh sửa']) ...[
                    Padding(
                      padding: const EdgeInsets.symmetric(vertical: 12),
                      child: Text(
                        group,
                        style: const TextStyle(fontWeight: FontWeight.w600),
                      ),
                    ),
                    SessionActionGrid(
                      children: [
                        for (final action in hostShortcuts(
                          host!,
                        ).where((action) => action.group == group))
                          _HostShortcutButton(
                            key: ValueKey('host-shortcut-${action.id}'),
                            action: action,
                            onPressed: busy || !current
                                ? null
                                : () => run(action),
                          ),
                      ],
                    ),
                  ],
              ],
            ),
          ),
        ),
      ],
    ),
  );
}

class _HostShortcutButton extends StatelessWidget {
  final HostShortcut action;
  final VoidCallback? onPressed;
  const _HostShortcutButton({super.key, required this.action, this.onPressed});
  @override
  Widget build(BuildContext context) => Tooltip(
    message: '${action.label} · ${action.keys} trên máy tính',
    child: OutlinedButton(
      onPressed: onPressed,
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(0, SanserTokens.touchTarget),
        padding: const EdgeInsets.all(12),
        alignment: Alignment.centerLeft,
      ),
      child: Row(
        children: [
          Icon(action.icon, size: 19),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  action.label,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
                Text(
                  action.keys,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontSize: 12),
                ),
              ],
            ),
          ),
        ],
      ),
    ),
  );
}
