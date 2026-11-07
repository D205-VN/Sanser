import 'dart:math' as math;
import 'package:flutter/material.dart';
import 'design_tokens.dart';
import 'adaptive_layout.dart';

/// Stays inside the safe viewport, including after rotation or keyboard resize.
class FloatingSessionControls extends StatefulWidget {
  final Widget content;
  final Widget? footer;
  final bool initiallyCompact;
  final double expandedHeight;
  final IconData icon;
  final String title;
  final String statusLabel;
  final bool connected;
  const FloatingSessionControls({
    super.key,
    required this.content,
    this.footer,
    this.initiallyCompact = true,
    this.expandedHeight = 360,
    this.icon = Icons.touch_app_outlined,
    this.title = 'Điều khiển',
    this.statusLabel = 'Phiên từ xa',
    this.connected = false,
  });
  @override
  State<FloatingSessionControls> createState() =>
      _FloatingSessionControlsState();
}

class _FloatingSessionControlsState extends State<FloatingSessionControls> {
  Offset anchor = const Offset(1, 0);
  late bool expanded = !widget.initiallyCompact;
  bool hidden = false;

  Widget _summary() => Column(
    mainAxisAlignment: MainAxisAlignment.center,
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Text(
        widget.title,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: const TextStyle(
          fontSize: 12,
          fontWeight: FontWeight.w600,
          color: SanserTokens.text,
        ),
      ),
      const SizedBox(height: 3),
      Text(
        widget.statusLabel,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          fontSize: 10,
          color: widget.connected ? SanserTokens.success : SanserTokens.muted,
        ),
      ),
    ],
  );

  @override
  Widget build(BuildContext context) => SafeArea(
    child: LayoutBuilder(
      builder: (context, constraints) {
        if (constraints.maxWidth < 100 || constraints.maxHeight < 72) {
          return const SizedBox.shrink();
        }
        final textScale = MediaQuery.textScalerOf(context);
        final compactHeight = math.max(
          56.0,
          textScale.scale(12) * 1.3 + textScale.scale(10) * 1.3 + 16,
        );
        final fullPanel =
            expanded &&
            !hidden &&
            constraints.maxHeight >=
                compactHeight + textScale.scale(14) * 1.3 + 120;
        final tablet = wideMobileWindow(constraints.biggest);
        final width = math.min(
          hidden
              ? 36.0
              : fullPanel
              ? (tablet ? 400.0 : 320.0)
              : 184.0,
          math.max(0.0, constraints.maxWidth - 16),
        );
        final height = math.min(
          fullPanel
              ? (tablet
                    ? math.max(
                        widget.expandedHeight,
                        math.min(600.0, constraints.maxHeight * .72),
                      )
                    : widget.expandedHeight)
              : compactHeight,
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
                  color: SanserTokens.surface,
                  elevation: 6,
                  shadowColor: Colors.black54,
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(
                      hidden ? 12 : SanserTokens.panelRadius,
                    ),
                    side: const BorderSide(color: SanserTokens.borderStrong),
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
                      : !fullPanel
                      ? Tooltip(
                          message: 'Mở điều khiển · kéo để di chuyển',
                          child: InkWell(
                            onTap: () => setState(() => expanded = true),
                            child: Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 12,
                              ),
                              child: Row(
                                children: [
                                  Icon(
                                    widget.icon,
                                    size: 21,
                                    color: SanserTokens.accent,
                                  ),
                                  const SizedBox(width: 10),
                                  if (height >= compactHeight)
                                    Expanded(child: _summary())
                                  else
                                    const Spacer(),
                                  const Icon(
                                    Icons.expand_more,
                                    size: 18,
                                    color: SanserTokens.muted,
                                  ),
                                ],
                              ),
                            ),
                          ),
                        )
                      : Column(
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
                                Expanded(child: _summary()),
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
                            Expanded(
                              child: SingleChildScrollView(
                                padding: const EdgeInsets.all(12),
                                child: widget.content,
                              ),
                            ),
                            if (widget.footer != null)
                              Padding(
                                padding: const EdgeInsets.fromLTRB(
                                  12,
                                  8,
                                  12,
                                  12,
                                ),
                                child: SizedBox(
                                  width: double.infinity,
                                  child: widget.footer!,
                                ),
                              ),
                          ],
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
