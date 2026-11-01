import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/floating_controls.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'package:sanser_mobile/src/screens.dart';
import 'widget_test.dart' show MemorySecrets;

class IdleEngine extends NativeEngine {
  @override
  Stream<Json> get updates => const Stream.empty();
  @override
  Future<Json> capabilities() async => {'available': false};
  @override
  Future<void> disconnect() async {}
}

void main() {
  testWidgets(
    'floating controls drag, hide, recover and stay inside rotated viewport',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 844);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: FloatingSessionControls(content: Text('Session actions')),
          ),
        ),
      );
      final handle = find.byKey(const ValueKey('session-controls-drag'));
      final before = tester.getTopLeft(handle);
      await tester.drag(handle, const Offset(-150, 220));
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(handle).dx, lessThan(before.dx));
      expect(tester.getTopLeft(handle).dy, greaterThan(before.dy));
      await tester.tap(find.byTooltip('Mở điều khiển · kéo để di chuyển'));
      await tester.pumpAndSettle();
      expect(find.text('Session actions'), findsOneWidget);
      await tester.tap(find.byTooltip('Ẩn điều khiển vào cạnh màn hình'));
      await tester.pumpAndSettle();
      expect(find.text('Session actions'), findsNothing);
      expect(tester.getSize(handle).width, 36);
      tester.view.physicalSize = const Size(844, 390);
      await tester.pumpAndSettle();
      final rect = tester.getRect(handle);
      expect(rect.right, lessThanOrEqualTo(844));
      expect(rect.bottom, lessThanOrEqualTo(390));
      await tester.tap(find.byTooltip('Hiện nút điều khiển'));
      await tester.pumpAndSettle();
      expect(
        find.byTooltip('Mở điều khiển · kéo để di chuyển'),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('settings are accessible and persist on this phone', (
    tester,
  ) async {
    final store = MemorySecrets();
    final c = MobileController(SanserApi(store), IdleEngine())
      ..account = {'email': 'test@example.com'};
    await tester.pumpWidget(MaterialApp(home: SanserHome(controller: c)));
    await tester.tap(find.text('Cài đặt'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Tự xoay ngang khi kết nối'));
    await tester.pumpAndSettle();
    expect(c.preferences.autoLandscape, false);
    expect(
      store.values['mobile.preferences'],
      contains('"autoLandscape":false'),
    );
    await tester.pumpWidget(const SizedBox.shrink());
    c.dispose();
    final restored = MobileController(SanserApi(store), IdleEngine());
    await restored.initialize();
    expect(restored.preferences.autoLandscape, false);
    restored.dispose();
  });

  testWidgets('session requests landscape and restores portrait on disconnect', (
    tester,
  ) async {
    final calls = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        calls.add(call);
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    final c = MobileController(SanserApi(MemorySecrets()), IdleEngine())
      ..phase = SessionPhase.streaming;
    // No remote session ID: avoid a real API request when exercising teardown.
    c.active = null;
    var show = true;
    late StateSetter update;
    await tester.pumpWidget(
      MaterialApp(
        home: StatefulBuilder(
          builder: (context, setState) {
            update = setState;
            return show
                ? RemoteSession(controller: c)
                : const Scaffold(body: Text('Home'));
          },
        ),
      ),
    );
    await tester.pump();
    expect(
      calls
          .where((c) => c.method == 'SystemChrome.setPreferredOrientations')
          .last
          .arguments,
      ['DeviceOrientation.landscapeLeft', 'DeviceOrientation.landscapeRight'],
    );
    update(() => show = false);
    await tester.pumpAndSettle();
    expect(
      calls
          .where((c) => c.method == 'SystemChrome.setPreferredOrientations')
          .last
          .arguments,
      ['DeviceOrientation.portraitUp'],
    );
    c.dispose();
  });
}
