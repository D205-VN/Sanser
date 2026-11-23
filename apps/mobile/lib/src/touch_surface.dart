import 'dart:async';
import 'package:flutter/material.dart';
import 'mobile_input.dart';

enum TouchMode { direct, touchpad, pan }

class MobileTouchSurface extends StatefulWidget {
  final MobileInput input;
  final TouchMode mode;
  final Size videoSize;
  final int resetZoom;
  final VoidCallback? onTapFeedback, onHoldFeedback;

  /// False while the host has remote control off: no pointer is drawn.
  final bool pointerVisible;
  const MobileTouchSurface({
    super.key,
    required this.input,
    required this.mode,
    required this.videoSize,
    this.resetZoom = 0,
    this.onTapFeedback,
    this.onHoldFeedback,
    this.pointerVisible = true,
  });
  @override
  State<MobileTouchSurface> createState() => _MobileTouchSurfaceState();
}

class _MobileTouchSurfaceState extends State<MobileTouchSurface>
    with WidgetsBindingObserver {
  double scale = 1, startScale = 1, scrollRemainder = 0;
  Offset pan = Offset.zero,
      startPan = Offset.zero,
      startFocal = Offset.zero,
      lastFocal = Offset.zero;
  bool moved = false, multi = false, dragging = false;

  /// Direct touch: a one-finger swipe scrolls like a phone app.
  bool scrolling = false;
  Timer? fling;
  double flingVelocity = 0;

  /// Direct touch: hold, then move, to drag.
  Offset? holdOrigin;
  Size viewport = Size.zero;
  Timer? viewportTimer;
  final Set<int> fingers = {};
  Offset? dragOrigin;
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didUpdateWidget(covariant MobileTouchSurface old) {
    super.didUpdateWidget(old);
    if (old.mode != widget.mode) {
      unawaited(widget.input.releaseAll());
      dragging = false;
    }
    if (old.resetZoom != widget.resetZoom) {
      scale = 1;
      pan = Offset.zero;
      _viewport();
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state != AppLifecycleState.resumed) {
      _stopFling();
      unawaited(widget.input.releaseAll());
      dragging = false;
    }
  }

  void _viewport() {
    viewportTimer ??= Timer(const Duration(milliseconds: 16), () {
      viewportTimer = null;
      unawaited(
        widget.input.engine
            .viewport(
              widget.input.sessionId,
              scale,
              viewport.width == 0 ? 0 : pan.dx / viewport.width,
              viewport.height == 0 ? 0 : pan.dy / viewport.height,
            )
            .catchError((Object _) {}),
      );
    });
  }

  Offset? _position(Offset point) =>
      remotePosition(point, viewport, widget.videoSize, scale, pan);
  void _point(Offset point) {
    final p = _position(point);
    if (p != null) widget.input.move(p);
  }

  /// Screen pixels of finger travel per wheel step.
  static const double scrollStep = 24;

  void _scrollBy(double dy) {
    scrollRemainder += dy;
    final lines = (scrollRemainder / scrollStep).truncate();
    if (lines != 0) {
      scrollRemainder -= lines * scrollStep;
      unawaited(widget.input.scroll(lines));
    }
  }

  void _stopFling() {
    fling?.cancel();
    fling = null;
  }

  /// Keeps scrolling after a quick swipe, slowing down like a phone list.
  void _startFling(double velocity) {
    _stopFling();
    if (velocity.abs() < 300) return;
    flingVelocity = velocity.clamp(-6000.0, 6000.0);
    fling = Timer.periodic(const Duration(milliseconds: 16), (timer) {
      if (!mounted) return timer.cancel();
      _scrollBy(flingVelocity * .016);
      flingVelocity *= .92;
      if (flingVelocity.abs() < 120) _stopFling();
    });
  }

  void _start(ScaleStartDetails d) {
    _stopFling();
    startScale = scale;
    startPan = pan;
    startFocal = lastFocal = d.localFocalPoint;
    moved = false;
    multi = d.pointerCount > 1;
    scrollRemainder = 0;
  }

  void _update(ScaleUpdateDetails d) {
    final delta = d.localFocalPoint - lastFocal;
    lastFocal = d.localFocalPoint;
    multi = multi || d.pointerCount > 1;
    if (multi && dragging) {
      unawaited(widget.input.button(0, false));
      dragging = false;
    }
    if (multi) scrolling = false;
    if ((d.localFocalPoint - startFocal).distance > 6 ||
        (d.scale - 1).abs() > .03) {
      moved = true;
    }
    if (widget.mode == TouchMode.pan ||
        (widget.mode == TouchMode.direct && multi)) {
      if (d.pointerCount > 1) scale = (startScale * d.scale).clamp(1.0, 3.0);
      final limit = Offset(
        viewport.width * (scale - 1) / 2,
        viewport.height * (scale - 1) / 2,
      );
      final proposed = startPan + d.localFocalPoint - startFocal;
      pan = Offset(
        proposed.dx.clamp(-limit.dx, limit.dx),
        proposed.dy.clamp(-limit.dy, limit.dy),
      );
      _viewport();
      setState(() {});
      return;
    }
    if (widget.mode == TouchMode.touchpad) {
      if (multi) {
        scrollRemainder += delta.dy;
        final lines = (scrollRemainder / 18).truncate();
        if (lines != 0) {
          scrollRemainder -= lines * 18;
          unawaited(widget.input.scroll(lines));
        }
      } else {
        final fit = applyBoxFit(
          BoxFit.contain,
          widget.videoSize,
          viewport,
        ).destination;
        widget.input.move(
          widget.input.cursor +
              Offset(
                delta.dx / (fit.width * scale),
                delta.dy / (fit.height * scale),
              ),
        );
        setState(() {});
      }
    } else if (!multi && moved) {
      // Direct touch: a one-finger swipe scrolls the window under the finger,
      // and the content follows the finger as in phone apps. Dragging is
      // hold-then-move (see the long-press handlers).
      if (!scrolling) {
        if (_position(dragOrigin ?? startFocal) == null) return;
        scrolling = true;
        scrollRemainder = 0;
        _point(dragOrigin ?? startFocal);
        // Count the travel since the finger touched down, so the content
        // keeps up with the finger.
        _scrollBy((d.localFocalPoint - (dragOrigin ?? startFocal)).dy);
        return;
      }
      _scrollBy(delta.dy);
    }
  }

  void _end(ScaleEndDetails d) {
    if (dragging) {
      unawaited(widget.input.button(0, false));
      dragging = false;
    }
    if (scrolling) {
      scrolling = false;
      _startFling(d.velocity.pixelsPerSecond.dy);
    }
  }

  void _holdStart(LongPressStartDetails d) {
    if (widget.mode == TouchMode.pan) return;
    if (widget.mode == TouchMode.touchpad) {
      unawaited(widget.input.click(1));
      if (widget.input.enabled) widget.onHoldFeedback?.call();
      return;
    }
    if (_position(d.localPosition) == null) return;
    _stopFling();
    holdOrigin = d.localPosition;
    _point(d.localPosition);
    if (widget.input.enabled) widget.onHoldFeedback?.call();
  }

  void _holdMove(LongPressMoveUpdateDetails d) {
    final origin = holdOrigin;
    if (widget.mode != TouchMode.direct || origin == null) return;
    if (!dragging) {
      if ((d.localPosition - origin).distance < 6) return;
      dragging = true;
      unawaited(widget.input.button(0, true));
    }
    _point(d.localPosition);
  }

  void _holdEnd() {
    if (widget.mode != TouchMode.direct || holdOrigin == null) return;
    holdOrigin = null;
    if (dragging) {
      unawaited(widget.input.button(0, false));
      dragging = false;
    } else {
      // Held without moving: right click where the finger was.
      unawaited(widget.input.click(1));
    }
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) {
      final next = box.biggest;
      if (next != viewport) {
        viewport = next;
        scale = 1;
        pan = Offset.zero;
        _viewport();
      }
      final cursor = remoteScreenPosition(
        widget.input.cursor,
        viewport,
        widget.videoSize,
        scale,
        pan,
      );
      return Listener(
        onPointerDown: (event) {
          if (fingers.isEmpty) {
            dragOrigin = event.localPosition;
            _stopFling();
          }
          fingers.add(event.pointer);
        },
        onPointerUp: (event) => fingers.remove(event.pointer),
        onPointerCancel: (event) {
          fingers.remove(event.pointer);
          dragging = false;
          scrolling = false;
          holdOrigin = null;
          _stopFling();
          unawaited(widget.input.releaseAll());
        },
        child: GestureDetector(
          key: const ValueKey('mobile-touch-surface'),
          behavior: HitTestBehavior.opaque,
          onScaleStart: _start,
          onScaleUpdate: _update,
          onScaleEnd: _end,
          onTapUp: (d) {
            if (widget.mode == TouchMode.pan) return;
            if (widget.mode == TouchMode.direct) {
              if (_position(d.localPosition) == null) return;
              _point(d.localPosition);
            }
            unawaited(widget.input.click());
            if (widget.input.enabled) widget.onTapFeedback?.call();
          },
          onLongPressStart: _holdStart,
          onLongPressMoveUpdate: _holdMove,
          onLongPressEnd: (_) => _holdEnd(),
          onLongPressCancel: () {
            if (dragging) unawaited(widget.input.button(0, false));
            dragging = false;
            holdOrigin = null;
          },
          child: Stack(
            children: [
              if (widget.mode == TouchMode.touchpad &&
                  widget.input.enabled &&
                  widget.pointerVisible)
                // The arrow's tip is its hot spot: it sits exactly where the
                // host clicks.
                Positioned(
                  key: const ValueKey('touchpad-pointer'),
                  left: cursor.dx,
                  top: cursor.dy,
                  child: const IgnorePointer(
                    child: CustomPaint(
                      size: pointerArrowSize,
                      painter: PointerArrowPainter(),
                    ),
                  ),
                ),
              if (scale > 1)
                Positioned(
                  left: 12,
                  bottom: 12,
                  child: IgnorePointer(
                    child: Text(
                      '${scale.toStringAsFixed(1)}×',
                      style: const TextStyle(color: Colors.white70),
                    ),
                  ),
                ),
            ],
          ),
        ),
      );
    },
  );
  @override
  void dispose() {
    viewportTimer?.cancel();
    _stopFling();
    WidgetsBinding.instance.removeObserver(this);
    unawaited(widget.input.releaseAll());
    super.dispose();
  }
}

/// Keyboard-mapped controls, not an emulated Xbox controller. Every held key
/// has an owner so releasing a stick never releases another pressed control.
class MobileGameControls extends StatefulWidget {
  final MobileInput input;
  final VoidCallback? onPressFeedback;
  const MobileGameControls({
    super.key,
    required this.input,
    this.onPressFeedback,
  });
  @override
  State<MobileGameControls> createState() => _MobileGameControlsState();
}

class _MobileGameControlsState extends State<MobileGameControls> {
  Offset stick = Offset.zero;
  void _stick(Offset p) {
    final wasActive = stick.dx.abs() > 12 || stick.dy.abs() > 12;
    stick = Offset(
      (p.dx - 60).clamp(-42.0, 42.0),
      (p.dy - 60).clamp(-42.0, 42.0),
    );
    for (final key in [26, 4, 22, 7]) {
      final down = switch (key) {
        26 => stick.dy < -12,
        22 => stick.dy > 12,
        4 => stick.dx < -12,
        _ => stick.dx > 12,
      };
      unawaited(widget.input.hold(key, down, 'stick'));
    }
    if (!wasActive &&
        (stick.dx.abs() > 12 || stick.dy.abs() > 12) &&
        widget.input.enabled) {
      widget.onPressFeedback?.call();
    }
    setState(() {});
  }

  void _release() {
    for (final key in [26, 4, 22, 7]) {
      unawaited(widget.input.hold(key, false, 'stick'));
    }
    setState(() => stick = Offset.zero);
  }

  Widget _button(String label, int code) => Listener(
    key: ValueKey('game-$code'),
    onPointerDown: (event) {
      unawaited(widget.input.hold(code, true, '$label:${event.pointer}'));
      if (widget.input.enabled) widget.onPressFeedback?.call();
    },
    onPointerUp: (event) =>
        unawaited(widget.input.hold(code, false, '$label:${event.pointer}')),
    onPointerCancel: (event) =>
        unawaited(widget.input.hold(code, false, '$label:${event.pointer}')),
    child: Container(
      width: 64,
      height: 52,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: const Color(0xa520293a),
        borderRadius: BorderRadius.circular(18),
        border: Border.all(color: Colors.white30),
      ),
      child: Text(
        label,
        style: const TextStyle(
          color: Colors.white,
          fontWeight: FontWeight.w600,
        ),
      ),
    ),
  );
  @override
  Widget build(BuildContext context) => SafeArea(
    child: Stack(
      children: [
        Positioned(
          left: 22,
          bottom: 22,
          child: GestureDetector(
            key: const ValueKey('game-stick'),
            behavior: HitTestBehavior.opaque,
            onPanDown: (d) => _stick(d.localPosition),
            onPanUpdate: (d) => _stick(d.localPosition),
            onPanEnd: (_) => _release(),
            onPanCancel: _release,
            child: Container(
              width: 120,
              height: 120,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: Colors.black26,
                border: Border.all(color: Colors.white30),
              ),
              child: Center(
                child: Transform.translate(
                  offset: stick,
                  child: Container(
                    width: 48,
                    height: 48,
                    decoration: const BoxDecoration(
                      shape: BoxShape.circle,
                      color: Color(0x99678ede),
                    ),
                    child: const Center(child: Text('WASD')),
                  ),
                ),
              ),
            ),
          ),
        ),
        Positioned(
          right: 22,
          bottom: 22,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  _button('Shift', 225),
                  const SizedBox(width: 10),
                  _button('E', 8),
                ],
              ),
              const SizedBox(height: 10),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  _button('Esc', 41),
                  const SizedBox(width: 10),
                  _button('Space', 44),
                ],
              ),
            ],
          ),
        ),
      ],
    ),
  );
  @override
  void dispose() {
    unawaited(widget.input.releaseAll());
    super.dispose();
  }
}

/// Size of the touchpad pointer; its tip is the top-left corner.
const pointerArrowSize = Size(14, 21);

/// A classic mouse arrow drawn with its tip at (0, 0), white with a dark
/// outline so it shows on light and dark screens.
class PointerArrowPainter extends CustomPainter {
  const PointerArrowPainter();

  @override
  void paint(Canvas canvas, Size size) {
    final arrow = Path()
      ..moveTo(0, 0)
      ..lineTo(0, 17)
      ..lineTo(4.2, 13.2)
      ..lineTo(7, 19.6)
      ..lineTo(9.8, 18.4)
      ..lineTo(7.1, 12.2)
      ..lineTo(12.6, 12.2)
      ..close();
    canvas
      ..drawPath(arrow, Paint()..color = Colors.white)
      ..drawPath(
        arrow,
        Paint()
          ..color = Colors.black
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2
          ..strokeJoin = StrokeJoin.round,
      );
  }

  @override
  bool shouldRepaint(PointerArrowPainter oldDelegate) => false;
}
