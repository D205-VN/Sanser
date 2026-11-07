import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'dart:convert';
import 'dart:math' as math;
import 'controller.dart';
import 'files.dart';
import 'adaptive_layout.dart';

Future<void> showMobileSettings(
  BuildContext context,
  MobileController controller,
) {
  if (wideMobileWindow(MediaQuery.sizeOf(context))) {
    return showDialog<void>(
      context: context,
      builder: (_) => Dialog(
        constraints: const BoxConstraints(maxWidth: 680),
        child: _SettingsPane(controller: controller),
      ),
    );
  }
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    useSafeArea: true,
    showDragHandle: true,
    constraints: const BoxConstraints(maxWidth: 680),
    builder: (context) => Padding(
      padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(context).bottom),
      child: SafeArea(
        top: false,
        child: _SettingsPane(controller: controller, heightFraction: .8),
      ),
    ),
  );
}

class _SettingsPane extends StatelessWidget {
  const _SettingsPane({required this.controller, this.heightFraction = 1});
  final MobileController controller;
  final double heightFraction;
  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) => SizedBox(
      key: const ValueKey('mobile-settings-pane'),
      height: math.min(760.0, box.maxHeight * heightFraction),
      child: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 12, 8, 12),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    'Cài đặt',
                    style: Theme.of(context).textTheme.headlineSmall,
                  ),
                ),
                IconButton(
                  tooltip: 'Đóng cài đặt',
                  onPressed: () => Navigator.of(context).pop(),
                  icon: const Icon(Icons.close_rounded),
                ),
              ],
            ),
          ),
          const Divider(height: 1),
          Expanded(
            child: SingleChildScrollView(
              padding: const EdgeInsets.all(20),
              child: MobileSettings(controller: controller),
            ),
          ),
        ],
      ),
    ),
  );
}

class MobileSettings extends StatelessWidget {
  final MobileController controller;
  const MobileSettings({super.key, required this.controller});
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final p = controller.preferences;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text(
                    'Đường kết nối',
                    style: TextStyle(fontWeight: FontWeight.w600),
                  ),
                  const SizedBox(height: 10),
                  DropdownButtonFormField<String>(
                    isExpanded: true,
                    key: ValueKey(p.networkMode),
                    initialValue: p.networkMode,
                    decoration: const InputDecoration(
                      labelText: 'Chọn cho phiên tiếp theo',
                    ),
                    items: const [
                      DropdownMenuItem(
                        value: 'auto',
                        child: Text(
                          'Auto · ưu tiên Direct',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      DropdownMenuItem(
                        value: 'direct',
                        child: Text(
                          'Direct · UDP trực tiếp',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      DropdownMenuItem(
                        value: 'relay',
                        child: Text(
                          'Relay · qua máy chủ',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                    ],
                    onChanged: (value) {
                      if (value != null) {
                        controller.setPreferences(
                          p.copyWith(networkMode: value),
                        );
                      }
                    },
                  ),
                  const SizedBox(height: 10),
                  const Text(
                    'Auto thử Direct trước rồi chuyển Relay nếu mạng chặn. Direct chỉ dùng khi hai máy xác minh được đường UDP.',
                    style: TextStyle(color: Colors.white60, fontSize: 12),
                  ),
                  const SizedBox(height: 16),
                  DropdownButtonFormField<int>(
                    isExpanded: true,
                    key: ValueKey('bitrate-${p.videoBitrateMbps}'),
                    initialValue: p.videoBitrateMbps,
                    decoration: const InputDecoration(
                      labelText: 'Bitrate video tối đa',
                    ),
                    items: [
                      for (final rate in const [2, 4, 6, 8, 12])
                        DropdownMenuItem(
                          value: rate,
                          child: Text(
                            '$rate Mbps${rate == 6 ? ' · mặc định' : ''}',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                    ],
                    onChanged: (rate) {
                      if (rate != null) {
                        controller.setPreferences(
                          p.copyWith(videoBitrateMbps: rate),
                        );
                      }
                    },
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    'Áp dụng cho phiên tiếp theo. Mức thấp hơn giảm tải mạng; Host có thể tự giảm thêm khi RTT tăng.',
                    style: TextStyle(color: Colors.white60, fontSize: 12),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),
          Card(
            child: Column(
              children: [
                SwitchListTile.adaptive(
                  secondary: const Icon(Icons.screen_rotation_rounded),
                  title: const Text('Tự xoay ngang khi kết nối'),
                  subtitle: const Text('Trở về màn hình dọc khi thoát phiên.'),
                  value: p.autoLandscape,
                  onChanged: (value) => controller.setPreferences(
                    p.copyWith(autoLandscape: value),
                  ),
                ),
                const Divider(height: 1),
                SwitchListTile.adaptive(
                  secondary: const Icon(Icons.volume_up_outlined),
                  title: const Text('Âm thanh khi kết nối'),
                  subtitle: const Text('Áp dụng cho phiên kết nối tiếp theo.'),
                  value: p.soundOnConnect,
                  onChanged: (value) => controller.setPreferences(
                    p.copyWith(soundOnConnect: value),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          Card(
            child: Column(
              children: [
                SwitchListTile.adaptive(
                  secondary: const Icon(Icons.touch_app_outlined),
                  title: const Text('Nút nổi gọn'),
                  subtitle: const Text(
                    'Mở phiên bằng một nút nhỏ. Kéo để di chuyển; mở nút để ẩn vào cạnh màn hình.',
                  ),
                  value: p.compactControls,
                  onChanged: (value) => controller.setPreferences(
                    p.copyWith(compactControls: value),
                  ),
                ),
                const Divider(height: 1),
                SwitchListTile.adaptive(
                  secondary: const Icon(Icons.vibration_rounded),
                  title: const Text('Rung khi thao tác'),
                  subtitle: const Text(
                    'Rung nhẹ khi chạm, giữ hoặc nhấn nút điều khiển. Tùy thiết bị và cài đặt hệ thống.',
                  ),
                  value: p.haptics,
                  onChanged: (value) =>
                      controller.setPreferences(p.copyWith(haptics: value)),
                ),
                const Divider(height: 1),
                SwitchListTile.adaptive(
                  secondary: const Icon(Icons.speed_rounded),
                  title: const Text('Hiện độ trễ kết nối'),
                  value: p.showLatency,
                  onChanged: (value) =>
                      controller.setPreferences(p.copyWith(showLatency: value)),
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          DropdownButtonFormField<String>(
            isExpanded: true,
            initialValue: p.pointerMode,
            decoration: const InputDecoration(
              labelText: 'Thao tác cảm ứng mặc định',
            ),
            items: const [
              DropdownMenuItem(
                value: 'direct',
                child: Text(
                  'Chạm trực tiếp',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              DropdownMenuItem(
                value: 'touchpad',
                child: Text(
                  'Touchpad · rê chuột',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
            onChanged: (mode) {
              if (mode != null) {
                controller.setPreferences(p.copyWith(pointerMode: mode));
              }
            },
          ),
          const SizedBox(height: 16),
          if (controller.capabilities['files'] == true)
            ListTile(
              leading: const Icon(Icons.folder_open_outlined),
              title: const Text('Tệp đã nhận'),
              subtitle: const Text(
                'Lưu ra thiết bị kể cả sau khi ngắt kết nối.',
              ),
              trailing: const Icon(Icons.chevron_right),
              onTap: () => showReceivedFiles(context, controller.engine),
            ),
          const Text(
            'Cài đặt được lưu trên điện thoại này.',
            style: TextStyle(color: Colors.white54),
          ),
          TextButton.icon(
            icon: const Icon(Icons.content_copy_rounded),
            label: const Text('Sao chép chẩn đoán phiên'),
            onPressed: () async {
              await Clipboard.setData(
                ClipboardData(
                  text: const JsonEncoder.withIndent(
                    '  ',
                  ).convert(controller.diagnosticReport()),
                ),
              );
              if (context.mounted) {
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(
                    content: Text(
                      'Đã sao chép số đo mạng, video và âm thanh. Không kèm khóa phiên.',
                    ),
                  ),
                );
              }
            },
          ),
        ],
      );
    },
  );
}
