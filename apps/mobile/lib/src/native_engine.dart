import 'package:flutter/services.dart';
import 'api.dart';

/// Only configuration, input, text/clipboard data, file metadata and numeric status cross this bridge.
/// Video decoding, transport, PCM playback and mic capture remain native.
class NativeEngine {
  int _documentOperations = 0;
  bool get documentUIActive => _documentOperations > 0;
  Future<T> _document<T>(Future<T> Function() operation) async {
    _documentOperations++;
    try {
      return await operation();
    } finally {
      _documentOperations--;
    }
  }

  static const channel = MethodChannel('com.sanser.mobile/engine');
  static const events = EventChannel('com.sanser.mobile/events');
  late final Stream<Json> _updates = events.receiveBroadcastStream().map(
    (e) => Map<String, dynamic>.from(e as Map),
  );
  Stream<Json> get updates => _updates;
  Future<void> controllerCapture(
    String sessionId,
    int attempt,
    int captureId,
    bool enabled,
  ) => channel.invokeMethod<void>('controllerCapture', {
    'sessionId': sessionId,
    'engineAttempt': attempt,
    'captureId': captureId,
    'enabled': enabled,
  });
  Future<void> analogControllerCapture(
    String sessionId,
    int attempt,
    int captureId,
    bool enabled,
  ) => channel.invokeMethod<void>('controllerCapture', {
    'sessionId': sessionId,
    'engineAttempt': attempt,
    'captureId': captureId,
    'enabled': enabled,
    'mode': 'analog',
  });
  Future<Json> capabilities() async => Map<String, dynamic>.from(
    await channel.invokeMapMethod<String, dynamic>('capabilities') ?? {},
  );
  Future<void> connect(Json configuration) =>
      channel.invokeMethod<void>('connect', configuration);
  Future<void> disconnect() => channel.invokeMethod<void>('disconnect');
  Future<void> mute(bool muted) =>
      channel.invokeMethod<void>('mute', {'muted': muted});
  Future<void> voice({
    required bool joined,
    required bool microphone,
    required bool deafened,
  }) => channel.invokeMethod<void>('voice', {
    'joined': joined,
    'microphone': microphone,
    'deafened': deafened,
  });
  Future<void> key(int code, bool down) =>
      channel.invokeMethod<void>('key', {'code': code, 'down': down});
  Future<void> resetInput() => channel.invokeMethod<void>('resetInput');
  Future<bool> text(String sessionId, String text) async =>
      await channel.invokeMethod<bool>('text', {
        'sessionId': sessionId,
        'text': text,
      }) ??
      false;
  Future<String> clipboard(
    String sessionId, {
    required bool write,
    String text = '',
  }) async =>
      await channel.invokeMethod<String>('clipboard', {
        'sessionId': sessionId,
        'write': write,
        'text': text,
      }) ??
      '';
  Future<Json> files(String sessionId, Json action) async =>
      Map<String, dynamic>.from(
        await channel.invokeMapMethod<String, dynamic>('files', {
              'sessionId': sessionId,
              'action': action,
            }) ??
            {},
      );
  Future<Json?> pickFile(String sessionId) => _document(() async {
    final result = await channel.invokeMapMethod<String, dynamic>('filePick', {
      'sessionId': sessionId,
    });
    return result == null ? null : Map<String, dynamic>.from(result);
  });
  Future<List<Json>> fileInbox() async =>
      (await channel.invokeListMethod<dynamic>('fileInbox') ?? [])
          .map((value) => Map<String, dynamic>.from(value as Map))
          .toList();
  Future<bool> exportFile(String key) => _document(
    () async =>
        await channel.invokeMethod<bool>('fileExport', {'key': key}) ?? false,
  );
  Future<void> remoteInput(
    String sessionId,
    int kind, {
    int x = 0,
    int y = 0,
    int code = 0,
    int delta = 0,
    bool down = false,
  }) => channel.invokeMethod<void>('input', {
    'sessionId': sessionId,
    'kind': kind,
    'x': x,
    'y': y,
    'code': code,
    'delta': delta,
    'down': down,
  });
  Future<void> viewport(String sessionId, double scale, double x, double y) =>
      channel.invokeMethod<void>('viewport', {
        'sessionId': sessionId,
        'scale': scale,
        'x': x,
        'y': y,
      });
  Future<Json> p2p(Json request) async => Map<String, dynamic>.from(
    await channel.invokeMapMethod<String, dynamic>('p2p', request) ?? {},
  );
}
