import 'package:flutter/material.dart';

/// Window-based rather than device-name/orientation checks: narrow tablet
/// windows keep the phone layout; a landscape phone keeps bottom navigation.
bool wideMobileWindow(Size size) => size.width >= 840 && size.height >= 600;

const _destinations = [
  (Icons.devices_rounded, 'Máy tính'),
  (Icons.screen_share_outlined, 'Chia sẻ'),
  (Icons.settings_outlined, 'Cài đặt'),
  (Icons.person_outline_rounded, 'Tài khoản'),
];

class MobileWorkspace extends StatelessWidget {
  const MobileWorkspace({
    super.key,
    required this.appBar,
    required this.body,
    required this.selectedIndex,
    required this.onSelected,
  });
  final PreferredSizeWidget appBar;
  final Widget body;
  final int selectedIndex;
  final ValueChanged<int> onSelected;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) {
      final wide = wideMobileWindow(box.biggest);
      return Scaffold(
        appBar: appBar,
        body: Row(
          children: [
            if (wide) ...[
              SafeArea(
                right: false,
                child: NavigationRail(
                  key: const ValueKey('workspace-rail'),
                  selectedIndex: selectedIndex,
                  onDestinationSelected: onSelected,
                  labelType: NavigationRailLabelType.all,
                  scrollable: true,
                  destinations: [
                    for (final item in _destinations)
                      NavigationRailDestination(
                        icon: Icon(item.$1),
                        label: Text(
                          item.$2,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                  ],
                ),
              ),
              const VerticalDivider(width: 1),
            ],
            Expanded(
              key: const ValueKey('workspace-content'),
              child: Align(
                alignment: Alignment.topCenter,
                child: ConstrainedBox(
                  constraints: BoxConstraints(
                    maxWidth: selectedIndex == 0 ? 1080 : 720,
                  ),
                  child: body,
                ),
              ),
            ),
          ],
        ),
        bottomNavigationBar: wide
            ? null
            : NavigationBar(
                selectedIndex: selectedIndex,
                onDestinationSelected: onSelected,
                destinations: [
                  for (final item in _destinations)
                    NavigationDestination(icon: Icon(item.$1), label: item.$2),
                ],
              ),
      );
    },
  );
}

/// Natural card height, with no fixed aspect ratio that could clip large text.
class MobileDeviceCards extends StatelessWidget {
  const MobileDeviceCards({super.key, required this.children});
  final List<Widget> children;
  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) {
      final scale = (MediaQuery.textScalerOf(context).scale(16) / 16).clamp(
        1.0,
        2.0,
      );
      final columns = box.maxWidth >= 2 * 320 * scale + 14 ? 2 : 1;
      final width = (box.maxWidth - (columns - 1) * 14) / columns;
      return Wrap(
        spacing: 14,
        runSpacing: 14,
        children: [
          for (final card in children) SizedBox(width: width, child: card),
        ],
      );
    },
  );
}
