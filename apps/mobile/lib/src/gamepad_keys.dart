import 'dart:async';
import 'package:flutter/foundation.dart';
import 'api.dart';
import 'mobile_input.dart';
import 'native_engine.dart';

/// Keyboard profile, not a virtual Xbox controller. Separate ownership for
/// every source prevents trigger/thumbstick or touch Shift releasing each other.
class GamepadKeyMapper {
  GamepadKeyMapper(this.input);
  final MobileInput input;
  final _owners = <String, Object>{};
  final _held = <String, int>{};
  bool _closed = false;
  bool armed = false;
  void _key(String source, int code, bool down) {
    if (_closed || (_held.containsKey(source) == down)) return;
    final owner = _owners.putIfAbsent(source, Object.new);
    if (down) {
      _held[source] = code;
    } else {
      _held.remove(source);
    }
    unawaited(input.hold(code, down, owner));
  }

  void _axis(String source, double value, int negative, int positive) {
    final old = _held[source];
    final next = value >= (old == positive ? .20 : .35)
        ? positive
        : value <= -(old == negative ? .20 : .35)
        ? negative
        : null;
    if (old == next) return;
    if (old != null) _key(source, old, false);
    if (next != null) _key(source, next, true);
  }

  bool update(Json state) {
    if (_closed) return false;
    final mask = state['buttons'];
    final values = [
      for (final key in ['lx', 'ly', 'rx', 'ry', 'lt', 'rt']) state[key],
    ];
    if (state['connected'] is! bool ||
        state['sample'] is! bool ||
        mask is! int ||
        mask < 0 ||
        mask > 0x3fff ||
        values.any((v) => v is! num || !v.isFinite) ||
        values.take(4).any((v) => (v as num).abs() > 1) ||
        values.skip(4).any((v) => (v as num) < 0 || v > 1)) {
      release();
      armed = false;
      return false;
    }
    if (state['connected'] != true) {
      release();
      armed = false;
      return true;
    }
    if (state['sample'] != true) return true;
    if (!armed) {
      armed =
          mask == 0 &&
          values.take(4).every((v) => (v as num).abs() <= .20) &&
          values.skip(4).every((v) => (v as num) <= .35);
      return true;
    }
    const buttons = [44, 41, 8, 21, 20, 9, 40, 43, 225, 224, 82, 81, 80, 79];
    for (var i = 0; i < buttons.length; i++) {
      _key('button$i', buttons[i], mask & (1 << i) != 0);
    }
    _axis('lx', (values[0] as num).toDouble(), 4, 7);
    _axis('ly', (values[1] as num).toDouble(), 22, 26);
    _axis('rx', (values[2] as num).toDouble(), 80, 79);
    _axis('ry', (values[3] as num).toDouble(), 81, 82);
    _key(
      'lt',
      225,
      (values[4] as num) >= (_held.containsKey('lt') ? 0.35 : 0.55),
    );
    _key(
      'rt',
      224,
      (values[5] as num) >= (_held.containsKey('rt') ? 0.35 : 0.55),
    );
    return true;
  }

  void release() {
    for (final item in _held.entries.toList()) {
      _key(item.key, item.value, false);
    }
    armed = false;
  }

  void dispose() {
    release();
    _closed = true;
  }
}

class GamepadKeyCapture extends ChangeNotifier {
  GamepadKeyCapture(
    this.engine,
    this.input, {
    required this.current,
    required this.attempt,
    this.onReleased,
  }) : mapper = GamepadKeyMapper(input);
  final NativeEngine engine;
  final MobileInput input;
  final bool Function() current;
  final int Function() attempt;
  final VoidCallback? onReleased;
  final GamepadKeyMapper mapper;
  static int _nextCapture = 0;
  int _capture = 0, _attempt = 0;
  bool _closed = false, _capturing = false, _sourceClosed = false;
  bool connected = false;
  bool analog = false, _analogArmed = false;
  bool get armed => analog ? _analogArmed : mapper.armed;
  void setAnalog(bool value) {
    if (_closed || value == analog) return;
    setCapture(false);
    analog = value;
    notifyListeners();
  }

  Future<void> _nativeCapture(int attempt, int capture, bool enabled) => analog
      ? engine.analogControllerCapture(
          input.sessionId,
          attempt,
          capture,
          enabled,
        )
      : engine.controllerCapture(input.sessionId, attempt, capture, enabled);
  String? error;
  StreamSubscription<Json>? _events;
  void setCapture(bool value) {
    if (_closed) return;
    value = value && current() && !_sourceClosed;
    if (value == _capturing && (!value || _attempt == attempt())) return;
    final oldCapture = _capture, oldAttempt = _attempt;
    _capturing = false;
    mapper.release();
    _analogArmed = false;
    connected = false;
    if (oldCapture != 0) {
      unawaited(
        _nativeCapture(oldAttempt, oldCapture, false).catchError((Object _) {}),
      );
    }
    _capture = 0;
    if (!value) {
      notifyListeners();
      return;
    }
    _capturing = true;
    _attempt = attempt();
    _capture = ++_nextCapture;
    error = null;
    _events ??= engine.updates.listen(
      _event,
      onError: (Object _) {
        if (!_closed) {
          setCapture(false);
          error = 'Không nhận được dữ liệu tay cầm.';
          notifyListeners();
        }
      },
      onDone: () {
        if (!_closed) {
          _sourceClosed = true;
          setCapture(false);
          error = 'Kênh tay cầm đã đóng. Khởi động lại ứng dụng để thử tiếp.';
          notifyListeners();
        }
      },
    );
    final capture = _capture;
    unawaited(
      (() async {
        if (analog) await input.releaseAll();
        if (_closed ||
            capture != _capture ||
            !current() ||
            _attempt != attempt()) {
          return;
        }
        await _nativeCapture(_attempt, capture, true);
      })().catchError((Object _) {
        if (!_closed && capture == _capture) {
          setCapture(false);
          error = 'Không thể bật tay cầm. Thử lại khi kết nối sẵn sàng.';
          notifyListeners();
        }
      }),
    );
    notifyListeners();
  }

  void _event(Json event) {
    if (_closed ||
        !_capturing ||
        event['state'] != 'controller' ||
        event['sessionId'] != input.sessionId ||
        event['engineAttempt'] != _attempt ||
        event['captureId'] != _capture) {
      return;
    }
    if (!current() || _attempt != attempt()) {
      setCapture(false);
      return;
    }
    if (event['paused'] == true) {
      setCapture(false);
      onReleased?.call();
      return;
    }
    final buttons = event['buttons'];
    if (event['connected'] == true &&
        buttons is int &&
        (buttons & 0xc0) == 0xc0) {
      setCapture(false);
      onReleased?.call();
      return;
    }
    final wasConnected = connected, wasArmed = armed, oldError = error;
    if (analog) {
      if (event['analog'] != true ||
          event['connected'] is! bool ||
          event['analogArmed'] is! bool) {
        setCapture(false);
        error = 'Không nhận được trạng thái analog. Đã nhả tay cầm.';
        notifyListeners();
        return;
      }
      connected = event['connected'] == true;
      _analogArmed = connected && event['analogArmed'] == true;
      error = null;
    } else if (!mapper.update(event)) {
      connected = false;
      error = 'Dữ liệu tay cầm không hợp lệ. Đã nhả phím.';
    } else {
      connected = event['connected'] == true;
      error = null;
    }
    if (wasConnected != connected || wasArmed != armed || oldError != error) {
      notifyListeners();
    }
  }

  @override
  void dispose() {
    setCapture(false);
    _closed = true;
    _events?.cancel();
    mapper.dispose();
    super.dispose();
  }
}
