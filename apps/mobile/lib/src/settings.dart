import 'package:flutter/material.dart';
import 'controller.dart';

Future<void> showMobileSettings(
  BuildContext context,
  MobileController controller,
) => showModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  builder: (context) => SafeArea(
    child: SizedBox(
      height: MediaQuery.sizeOf(context).height * .8,
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Cài đặt', style: Theme.of(context).textTheme.headlineSmall),
            const SizedBox(height: 16),
            MobileSettings(controller: controller),
          ],
        ),
      ),
    ),
  ),
);

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
                    key: ValueKey(p.networkMode),
                    initialValue: p.networkMode,
                    decoration: const InputDecoration(
                      labelText: 'Chọn cho phiên tiếp theo',
                    ),
                    items: const [
                      DropdownMenuItem(
                        value: 'auto',
                        child: Text('Auto · ưu tiên Direct'),
                      ),
                      DropdownMenuItem(
                        value: 'direct',
                        child: Text('Direct · UDP trực tiếp'),
                      ),
                      DropdownMenuItem(
                        value: 'relay',
                        child: Text('Relay · qua máy chủ'),
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
          const Text(
            'Cài đặt được lưu trên điện thoại này.',
            style: TextStyle(color: Colors.white54),
          ),
        ],
      );
    },
  );
}
