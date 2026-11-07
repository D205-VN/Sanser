import 'dart:async';
import 'package:flutter/widgets.dart';
import 'native_engine.dart';

/// Only small input messages cross Flutter. Video/audio remain native. Moves
/// coalesce; button/key transitions remain ordered and session-fenced.
class MobileInput {
  final NativeEngine engine;
  final String sessionId;
  final bool enabled;
  final void Function()? onError;
  final bool Function()? current;
  Offset cursor = const Offset(.5, .5);
  final Map<int, Set<Object>> _owners = {};
  Future<void> _queue = Future.value();
  Timer? _moveTimer;
  Offset? _pendingMove;
  bool _closed = false;
  int _resetEpoch = 0;
  bool _chordPending = false;
  MobileInput(
    this.engine,
    this.sessionId, {
    required this.enabled,
    this.onError,
    this.current,
  });
  int _coordinate(double value) => (value.clamp(0.0, 1.0) * 65535).round();
  Future<void> _send(
    int kind, {
    int code = 0,
    int delta = 0,
    bool down = false,
    Offset? position,
  }) {
    if (_closed || !enabled) return Future.value();
    final at = position ?? cursor;
    _queue = _queue
        .then(
          (_) => current?.call() == false
              ? Future<void>.value()
              : engine.remoteInput(
                  sessionId,
                  kind,
                  x: _coordinate(at.dx),
                  y: _coordinate(at.dy),
                  code: code,
                  delta: delta,
                  down: down,
                ),
        )
        .catchError((Object _) async {
          _resetEpoch++;
          _owners.clear();
          if (current?.call() == false) return;
          try {
            await engine.remoteInput(sessionId, 4);
          } catch (_) {
            /* disconnected */
          }
          onError?.call();
        });
    return _queue;
  }

  void move(Offset normalized) {
    if (!enabled || _closed) return;
    cursor = Offset(
      normalized.dx.clamp(0.0, 1.0),
      normalized.dy.clamp(0.0, 1.0),
    );
    _pendingMove = cursor;
    _moveTimer ??= Timer(const Duration(milliseconds: 8), flushMove);
  }

  void flushMove() {
    _moveTimer?.cancel();
    _moveTimer = null;
    final position = _pendingMove;
    _pendingMove = null;
    if (position != null) unawaited(_send(0, position: position));
  }

  Future<void> button(int code, bool down) {
    flushMove();
    return _send(1, code: code, down: down);
  }

  Future<void> click([int code = 0]) async {
    await button(code, true);
    await button(code, false);
  }

  Future<void> scroll(int lines) {
    flushMove();
    return _send(3, delta: lines.clamp(-20, 20));
  }

  Future<void> hold(int code, bool down, Object owner) {
    if (!enabled || _closed) return Future.value();
    final holders = _owners.putIfAbsent(code, () => {}),
        wasHeld = holders.isNotEmpty;
    if (down) {
      holders.add(owner);
    } else {
      holders.remove(owner);
    }
    if (wasHeld == holders.isNotEmpty) return Future.value();
    return _send(2, code: code, down: holders.isNotEmpty);
  }

  Future<void> tapKey(int code) async {
    final owner = Object();
    await hold(code, true, owner);
    await hold(code, false, owner);
  }

  /// One-shot keyboard chord, with source-owned releases in reverse order.
  /// Reset/failure cancels the remaining presses, even if input becomes ready
  /// again before a slow native reply returns. Never auto-retry a shortcut.
  Future<bool> chord(List<int> codes, {bool Function()? current}) async {
    final keys = List<int>.of(codes);
    if (keys.isEmpty ||
        keys.length > 4 ||
        keys.toSet().length != keys.length ||
        keys.any(
          // Match native/desktop/keyboard.h: never reserve a reliable input
          // sequence for a usage the receiving host will reject.
          (code) =>
              !((code >= 4 && code <= 49) ||
                  (code >= 51 && code <= 69) ||
                  (code >= 73 && code <= 100) ||
                  (code >= 224 && code <= 231)),
        )) {
      return false;
    }
    final epoch = _resetEpoch;
    bool valid() =>
        !_closed &&
        enabled &&
        epoch == _resetEpoch &&
        this.current?.call() != false &&
        current?.call() != false;
    if (_chordPending ||
        !valid() ||
        _owners.values.any((owners) => owners.isNotEmpty)) {
      return false;
    }
    _chordPending = true;
    final owner = Object(), pressed = <int>[];
    var completed = false;
    try {
      await drain();
      if (!valid() || _owners.values.any((owners) => owners.isNotEmpty)) {
        return false;
      }
      for (final code in keys) {
        if (!valid()) return false;
        pressed.add(code);
        await hold(code, true, owner);
      }
      completed = valid();
    } finally {
      for (final code in pressed.reversed) {
        await hold(code, false, owner);
      }
      _chordPending = false;
    }
    return completed && valid();
  }

  /// Repeat only a key this source still owns. Reset/release cannot resurrect
  /// it, and another source's held key is not enough to authorize a repeat.
  Future<void> repeatKey(int code, Object owner) {
    if (_closed || !enabled || !(_owners[code]?.contains(owner) ?? false)) {
      return Future.value();
    }
    return _send(2, code: code, down: true);
  }

  Future<void> releaseAll() {
    _resetEpoch++;
    _moveTimer?.cancel();
    _moveTimer = null;
    _pendingMove = null;
    _owners.clear();
    return _send(4);
  }

  Future<void> drain() => _queue;
  void dispose() {
    unawaited(releaseAll());
    _closed = true;
  }
}

/// Reverse zoom/pan and letterboxing before mapping a finger to host pixels.
Offset? remotePosition(
  Offset point,
  Size viewport,
  Size video,
  double scale,
  Offset pan,
) {
  if (viewport.isEmpty || video.isEmpty) return null;
  final center = viewport.center(Offset.zero);
  final local = (point - center - pan) / scale + center;
  final fit = applyBoxFit(BoxFit.contain, video, viewport).destination;
  final rect = Alignment.center.inscribe(fit, Offset.zero & viewport);
  if (!rect.contains(local)) return null;
  return Offset(
    (local.dx - rect.left) / rect.width,
    (local.dy - rect.top) / rect.height,
  );
}

Offset remoteScreenPosition(
  Offset normalized,
  Size viewport,
  Size video,
  double scale,
  Offset pan,
) {
  if (viewport.isEmpty || video.isEmpty) return Offset.zero;
  final fit = applyBoxFit(BoxFit.contain, video, viewport).destination;
  final rect = Alignment.center.inscribe(fit, Offset.zero & viewport);
  final host = Offset(
    rect.left + normalized.dx * rect.width,
    rect.top + normalized.dy * rect.height,
  );
  final center = viewport.center(Offset.zero);
  return center + (host - center) * scale + pan;
}
