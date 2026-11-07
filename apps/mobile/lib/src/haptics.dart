import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// Local touch feedback only: never waits on the network, queues vibrations,
/// or treats a vibration as acknowledgement from the remote computer.
class MobileHaptics with WidgetsBindingObserver {
  MobileHaptics({
    required this.enabled,
    required this.current,
    Duration Function()? elapsed,
  }) : _elapsed = elapsed {
    final state = WidgetsBinding.instance.lifecycleState;
    _foreground = state == null || state == AppLifecycleState.resumed;
    WidgetsBinding.instance.addObserver(this);
  }

  final bool Function() enabled;
  final bool Function() current;
  final Duration Function()? _elapsed;
  final _clock = Stopwatch()..start();
  Duration? _last;
  bool _pending = false, _closed = false, _foreground = true;

  void tap() => _pulse(impact: false);
  void hold() => _pulse(impact: true);

  void _pulse({required bool impact}) {
    if (_closed || _pending || !_foreground || !enabled() || !current()) return;
    if (defaultTargetPlatform != TargetPlatform.iOS &&
        defaultTargetPlatform != TargetPlatform.android) {
      return;
    }
    final now = _elapsed?.call() ?? _clock.elapsed;
    if (_last != null && now - _last! < const Duration(milliseconds: 75)) {
      return;
    }
    _last = now;
    _pending = true;
    unawaited(_emit(impact));
  }

  Future<void> _emit(bool impact) async {
    try {
      if (impact) {
        await HapticFeedback.lightImpact();
      } else {
        await HapticFeedback.selectionClick();
      }
    } catch (_) {
      // Unsupported hardware/platform channels cannot interrupt input.
    } finally {
      _pending = false;
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _foreground = state == AppLifecycleState.resumed;
  }

  void dispose() {
    _closed = true;
    WidgetsBinding.instance.removeObserver(this);
  }
}
