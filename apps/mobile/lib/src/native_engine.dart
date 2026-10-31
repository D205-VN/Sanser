import 'package:flutter/services.dart';
import 'api.dart';

/// Only session configuration, input and numeric status cross this bridge.
/// Video decoding, transport, PCM playback and mic capture remain native.
class NativeEngine {
  static const channel = MethodChannel('com.sanser.mobile/engine');
  static const events = EventChannel('com.sanser.mobile/events');
  Stream<Json> get updates => events.receiveBroadcastStream().map(
    (e) => Map<String, dynamic>.from(e as Map),
  );
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
}
