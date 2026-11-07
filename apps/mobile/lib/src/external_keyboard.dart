import 'dart:async';
import 'dart:ui' as ui;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'mobile_input.dart';

/// Match native/desktop/keyboard.h. Consumer/media/system keys remain local.
int? externalKeyboardHid(PhysicalKeyboardKey key) {
  final usage = key.usbHidUsage;
  if ((usage & 0xffff0000) != 0x00070000) return null;
  final code = usage & 0xffff;
  return (code >= 4 && code <= 49) ||
          (code >= 51 && code <= 69) ||
          (code >= 73 && code <= 100) ||
          (code >= 224 && code <= 231)
      ? code
      : null;
}

class ExternalKeyboardInput {
  ExternalKeyboardInput(this.input, {required this.current, this.onReleased});
  final MobileInput input;
  final bool Function() current;
  final VoidCallback? onReleased;
  final _pressed = <PhysicalKeyboardKey, (int, Object)>{};
  final _repeating = <Object>{};
  bool _closed = false;

  KeyEventResult handle(KeyEvent event) {
    if (_closed) return KeyEventResult.ignored;
    if (!current()) {
      release();
      return KeyEventResult.ignored;
    }
    if (event.deviceType != ui.KeyEventDeviceType.keyboard) {
      return KeyEventResult.ignored;
    }
    final code = externalKeyboardHid(event.physicalKey);
    if (code == null) return KeyEventResult.ignored;
    if (event is KeyUpEvent) {
      final held = _pressed.remove(event.physicalKey);
      if (held == null) return KeyEventResult.ignored;
      unawaited(input.hold(held.$1, false, held.$2));
      return KeyEventResult.handled;
    }
    // OS resynchronization and keys held before focus/capture are not presses
    // authorized by this session. A synthesized up can still release ownership.
    if (event.synthesized) return KeyEventResult.ignored;
    if (event is KeyRepeatEvent) {
      final held = _pressed[event.physicalKey];
      if (held == null) return KeyEventResult.ignored;
      if (code < 224 && code != 57 && code != 83 && _repeating.add(held.$2)) {
        unawaited(
          input.repeatKey(code, held.$2).whenComplete(() {
            _repeating.remove(held.$2);
          }),
        );
      }
      return KeyEventResult.handled;
    }
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    if (code == 41 &&
        _pressed.values.any((held) => held.$1 == 224 || held.$1 == 228) &&
        _pressed.values.any((held) => held.$1 == 226 || held.$1 == 230)) {
      release();
      unawaited(input.releaseAll());
      onReleased?.call();
      return KeyEventResult.handled;
    }
    if (!_pressed.containsKey(event.physicalKey)) {
      final owner = Object();
      _pressed[event.physicalKey] = (code, owner);
      unawaited(input.hold(code, true, owner));
    }
    return KeyEventResult.handled;
  }

  void release() {
    final held = _pressed.values.toList();
    _pressed.clear();
    for (final key in held) {
      unawaited(input.hold(key.$1, false, key.$2));
    }
  }

  void dispose() {
    release();
    _closed = true;
  }
}

/// Capture is opt-in, focus-scoped and confined to the video/game surface.
/// Sheets, text editors, account fields and background apps keep their keys.
class ExternalKeyboardSurface extends StatefulWidget {
  const ExternalKeyboardSurface({
    super.key,
    required this.input,
    required this.enabled,
    required this.current,
    required this.child,
    required this.onReleased,
    this.forwardKeys = true,
    this.onFocusChanged,
  });
  final MobileInput input;
  final bool enabled;
  final bool Function() current;
  final Widget child;
  final VoidCallback onReleased;
  final bool forwardKeys;
  final ValueChanged<bool>? onFocusChanged;
  @override
  State<ExternalKeyboardSurface> createState() =>
      _ExternalKeyboardSurfaceState();
}

class _ExternalKeyboardSurfaceState extends State<ExternalKeyboardSurface>
    with WidgetsBindingObserver {
  final focus = FocusNode(debugLabel: 'Sanser remote hardware keyboard');
  late ExternalKeyboardInput keyboard;
  bool foreground = true;
  bool get wanted =>
      mounted &&
      widget.enabled &&
      widget.current() &&
      foreground &&
      (ModalRoute.of(context)?.isCurrent ?? false);
  ExternalKeyboardInput _create() => ExternalKeyboardInput(
    widget.input,
    current: () => wanted && focus.hasPrimaryFocus,
    onReleased: () {
      focus.unfocus();
      widget.onReleased();
    },
  );
  @override
  void initState() {
    super.initState();
    foreground =
        WidgetsBinding.instance.lifecycleState == null ||
        WidgetsBinding.instance.lifecycleState == AppLifecycleState.resumed;
    keyboard = _create();
    focus.addListener(_focusChanged);
    WidgetsBinding.instance.addObserver(this);
    if (widget.enabled) _requestFocus();
  }

  void _focusChanged() {
    if (!focus.hasPrimaryFocus) keyboard.release();
    widget.onFocusChanged?.call(wanted && focus.hasPrimaryFocus);
  }

  void _requestFocus() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (wanted) focus.requestFocus();
    });
  }

  @override
  void didUpdateWidget(covariant ExternalKeyboardSurface oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.input != widget.input) {
      keyboard.dispose();
      keyboard = _create();
    }
    if (!widget.enabled || !widget.current()) {
      keyboard.release();
      focus.unfocus();
    } else if (!oldWidget.enabled || oldWidget.input != widget.input) {
      _requestFocus();
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (!wanted) keyboard.release();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    foreground = state == AppLifecycleState.resumed;
    if (!foreground) {
      keyboard.release();
      focus.unfocus();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    focus.removeListener(_focusChanged);
    widget.onFocusChanged?.call(false);
    keyboard.dispose();
    focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Focus(
    focusNode: focus,
    canRequestFocus: wanted,
    onKeyEvent: (_, event) =>
        widget.forwardKeys ? keyboard.handle(event) : KeyEventResult.ignored,
    child: Listener(
      behavior: HitTestBehavior.translucent,
      onPointerDown: (_) {
        if (wanted) focus.requestFocus();
      },
      child: widget.child,
    ),
  );
}
