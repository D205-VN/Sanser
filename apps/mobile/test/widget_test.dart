import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/main.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/native_engine.dart';

class MemorySecrets implements SecretStore {
  final values = <String, String>{};
  @override
  Future<String?> read(String key) async => values[key];
  @override
  Future<void> write(String key, String value) async {
    values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    values.remove(key);
  }
}

class PreviewEngine extends NativeEngine {
  @override
  Future<Json> capabilities() async => {'available': false};
  @override
  Stream<Json> get updates => const Stream.empty();
}

void main() {
  testWidgets(
    'Flutter sign-in validates fields without exposing server settings by default',
    (tester) async {
      final controller = MobileController(
        SanserApi(MemorySecrets()),
        PreviewEngine(),
      );
      await tester.pumpWidget(SanserApp(controller: controller));
      await tester.pumpAndSettle();
      expect(find.text('Máy tính của bạn.\nTrong tầm tay.'), findsOneWidget);
      expect(find.text('Địa chỉ HTTPS'), findsNothing);
      await tester.ensureVisible(
        find.widgetWithText(FilledButton, 'Đăng nhập'),
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Đăng nhập'));
      await tester.pumpAndSettle();
      expect(find.text('Nhập email của bạn'), findsOneWidget);
      expect(find.text('Nhập mật khẩu'), findsOneWidget);
    },
  );
  testWidgets('phone and landscape layouts do not overflow', (tester) async {
    final controller = MobileController(
      SanserApi(MemorySecrets()),
      PreviewEngine(),
    );
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpWidget(SanserApp(controller: controller));
    await tester.pumpAndSettle();
    for (final size in [const Size(360, 780), const Size(780, 360)]) {
      tester.view.physicalSize = size;
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
    }
  });
}
