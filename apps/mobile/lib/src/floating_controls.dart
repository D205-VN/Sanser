import 'dart:math' as math;
import 'package:flutter/material.dart';

/// Stays inside the safe viewport, including after rotation or keyboard resize.
class FloatingSessionControls extends StatefulWidget {
  final Widget content;
  final bool initiallyCompact;
  final IconData icon;
  const FloatingSessionControls({
    super.key,
    required this.content,
    this.initiallyCompact = true,
    this.icon = Icons.touch_app_outlined,
  });
  @override
  State<FloatingSessionControls> createState() =>
      _FloatingSessionControlsState();
}

class _FloatingSessionControlsState extends State<FloatingSessionControls> {
  Offset anchor = const Offset(1, 0);
  late bool expanded = !widget.initiallyCompact;
  bool hidden = false;

  @override
  Widget build(BuildContext context) => SafeArea(
    child: LayoutBuilder(
      builder: (context, constraints) {
        final width = math.min(
          hidden
              ? 36.0
              : expanded
              ? 300.0
              : 56.0,
          math.max(0.0, constraints.maxWidth - 16),
        );
        final height = math.min(
          expanded && !hidden ? 310.0 : 56.0,
          math.max(0.0, constraints.maxHeight - 16),
        );
        final rangeX = math.max(0.0, constraints.maxWidth - width - 16);
        final rangeY = math.max(0.0, constraints.maxHeight - height - 16);
        final edge = anchor.dx < .5 ? 0.0 : 1.0;
        return Stack(
          children: [
            Positioned(
              left: 8 + (hidden ? edge : anchor.dx) * rangeX,
              top: 8 + anchor.dy * rangeY,
              width: width,
              height: height,
              child: GestureDetector(
                key: const ValueKey('session-controls-drag'),
                onPanUpdate: (details) => setState(() {
                  anchor = Offset(
                    rangeX == 0
                        ? 0
                        : (anchor.dx + details.delta.dx / rangeX).clamp(0, 1),
                    rangeY == 0
                        ? 0
                        : (anchor.dy + details.delta.dy / rangeY).clamp(0, 1),
                  );
                }),
                child: Material(
                  color: const Color(0xf2191f2c),
                  elevation: 8,
                  shadowColor: Colors.black54,
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(hidden ? 14 : 22),
                    side: const BorderSide(color: Colors.white12),
                  ),
                  clipBehavior: Clip.antiAlias,
                  child: hidden
                      ? IconButton(
                          tooltip: 'Hiện nút điều khiển',
                          onPressed: () => setState(() {
                            hidden = false;
                            expanded = false;
                          }),
                          icon: Icon(
                            edge == 0
                                ? Icons.chevron_right
                                : Icons.chevron_left,
                          ),
                        )
                      : !expanded
                      ? IconButton(
                          tooltip: 'Mở điều khiển · kéo để di chuyển',
                          onPressed: () => setState(() => expanded = true),
                          icon: Icon(
                            widget.icon,
                            color: const Color(0xff78dfc0),
                          ),
                        )
                      : SingleChildScrollView(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Row(
                                children: [
                                  const SizedBox(width: 12),
                                  const Icon(
                                    Icons.drag_indicator_rounded,
                                    size: 20,
                                    color: Colors.white54,
                                  ),
                                  const SizedBox(width: 6),
                                  const Expanded(
                                    child: Text(
                                      'Sanser',
                                      style: TextStyle(
                                        fontWeight: FontWeight.w600,
                                      ),
                                    ),
                                  ),
                                  IconButton(
                                    tooltip: 'Ẩn điều khiển vào cạnh màn hình',
                                    onPressed: () => setState(() {
                                      hidden = true;
                                      expanded = false;
                                    }),
                                    icon: const Icon(
                                      Icons.visibility_off_outlined,
                                      size: 20,
                                    ),
                                  ),
                                  IconButton(
                                    tooltip: 'Thu gọn điều khiển',
                                    onPressed: () =>
                                        setState(() => expanded = false),
                                    icon: const Icon(
                                      Icons.unfold_less_rounded,
                                      size: 20,
                                    ),
                                  ),
                                ],
                              ),
                              const Divider(height: 1),
                              Padding(
                                padding: const EdgeInsets.all(12),
                                child: widget.content,
                              ),
                            ],
                          ),
                        ),
                ),
              ),
            ),
          ],
        );
      },
    ),
  );
}
