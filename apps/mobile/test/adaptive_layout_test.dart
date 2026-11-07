import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/floating_controls.dart';
import 'package:sanser_mobile/src/screens.dart';
import 'package:sanser_mobile/src/settings.dart';
import 'package:sanser_mobile/src/theme.dart';
import 'session_actions_test.dart' show ActionEngine, active, finish;
import 'widget_test.dart' show MemorySecrets;

void viewport(WidgetTester tester, Size size) {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
}

void main() {
  Future<void> show(
    WidgetTester tester,
    Widget child, {
    Size size = const Size(1200, 900),
    double textScale = 1,
  }) async {
    viewport(tester, size);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetViewInsets);
    await tester.pumpWidget(
      MaterialApp(
        theme: sanserTheme(),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: TextScaler.linear(textScale)),
          child: child!,
        ),
        home: child,
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'tablet rail keeps selection when resized into split screen or landscape phone',
    (tester) async {
      final c = MobileController(SanserApi(MemorySecrets()), ActionEngine())
        ..account = {'email': 'demo@example.com'};
      await show(tester, SanserHome(controller: c));
      expect(find.byType(NavigationRail), findsOneWidget);
      expect(find.byType(NavigationBar), findsNothing);
      await tester.tap(
        find.descendant(
          of: find.byType(NavigationRail),
          matching: find.text('Tài khoản'),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('demo@example.com'), findsOneWidget);
      viewport(tester, const Size(620, 900));
      await tester.pumpAndSettle();
      expect(find.byType(NavigationRail), findsNothing);
      expect(
        tester.widget<NavigationBar>(find.byType(NavigationBar)).selectedIndex,
        3,
      );
      expect(find.text('demo@example.com'), findsOneWidget);
      viewport(tester, const Size(1024, 768));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<NavigationRail>(find.byType(NavigationRail))
            .selectedIndex,
        3,
      );
      viewport(tester, const Size(844, 390));
      await tester.pumpAndSettle();
      expect(find.byType(NavigationRail), findsNothing);
      expect(
        tester.widget<NavigationBar>(find.byType(NavigationBar)).selectedIndex,
        3,
      );
      expect(tester.takeException(), isNull);
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'device cards use natural-height columns then stack at narrow widths or large text',
    (tester) async {
      final c = MobileController(SanserApi(MemorySecrets()), ActionEngine())
        ..account = {'email': 'demo@example.com'}
        ..capabilities = {'available': true}
        ..devices = [
          for (var i = 0; i < 4; i++)
            {
              'id': 'demo-$i',
              'name': 'Máy tính thử $i · tên dài có thể xuống dòng',
              'platform': i.isEven ? 'Windows' : 'macOS',
              'status': 'online',
            },
        ];
      await show(tester, SanserHome(controller: c));
      final cards = find.byType(DeviceCard);
      expect(
        tester.getTopLeft(cards.at(0)).dy,
        tester.getTopLeft(cards.at(1)).dy,
      );
      expect(
        tester.getTopLeft(cards.at(1)).dx,
        greaterThan(tester.getTopLeft(cards.at(0)).dx),
      );
      viewport(tester, const Size(620, 900));
      await tester.pumpAndSettle();
      expect(
        tester.getTopLeft(cards.at(1)).dy,
        greaterThan(tester.getTopLeft(cards.at(0)).dy),
      );
      await show(tester, SanserHome(controller: c), textScale: 2);
      expect(
        tester.getTopLeft(cards.at(1)).dy,
        greaterThan(tester.getTopLeft(cards.at(0)).dy),
      );
      expect(tester.takeException(), isNull);
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'tablet settings is a bounded dialog, close stays reachable after scroll and keyboard resize',
    (tester) async {
      final c = MobileController(SanserApi(MemorySecrets()), ActionEngine());
      await show(
        tester,
        Scaffold(
          body: Builder(
            builder: (context) => Center(
              child: TextButton(
                onPressed: () => showMobileSettings(context, c),
                child: const Text('Open settings'),
              ),
            ),
          ),
        ),
        textScale: 1.6,
      );
      await tester.tap(find.text('Open settings'));
      await tester.pumpAndSettle();
      expect(find.byType(Dialog), findsOneWidget);
      expect(
        tester
            .getSize(find.byKey(const ValueKey('mobile-settings-pane')))
            .width,
        lessThanOrEqualTo(680),
      );
      await tester.drag(
        find.byType(SingleChildScrollView),
        const Offset(0, -600),
      );
      await tester.pumpAndSettle();
      viewport(tester, const Size(700, 720));
      tester.view.viewInsets = const FakeViewPadding(bottom: 240);
      await tester.pumpAndSettle();
      final close = find.byTooltip('Đóng cài đặt');
      expect(tester.getRect(close).bottom, lessThanOrEqualTo(480));
      expect(tester.takeException(), isNull);
      await tester.tap(close);
      await tester.pumpAndSettle();
      expect(find.byType(Dialog), findsNothing);
      c.dispose();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets('phone settings remains a scrollable sheet above the keyboard', (
    tester,
  ) async {
    final c = MobileController(SanserApi(MemorySecrets()), ActionEngine());
    await show(
      tester,
      Scaffold(
        body: Builder(
          builder: (context) => Center(
            child: TextButton(
              onPressed: () => showMobileSettings(context, c),
              child: const Text('Open settings'),
            ),
          ),
        ),
      ),
      size: const Size(390, 844),
    );
    await tester.tap(find.text('Open settings'));
    await tester.pumpAndSettle();
    expect(find.byType(Dialog), findsNothing);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    await tester.pumpAndSettle();
    final pane = find.byType(SingleChildScrollView);
    expect(tester.getRect(pane).bottom, lessThanOrEqualTo(544));
    await tester.drag(pane, const Offset(0, -500));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Đóng cài đặt'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Đóng cài đặt'), findsNothing);
    expect(tester.takeException(), isNull);
    c.dispose();
  }, variant: TargetPlatformVariant.only(TargetPlatform.linux));

  testWidgets(
    'floating panel grows on tablet and keeps sticky actions in bounds when rotated or resized',
    (tester) async {
      var stops = 0;
      await show(
        tester,
        Scaffold(
          body: FloatingSessionControls(
            initiallyCompact: false,
            expandedHeight: 400,
            footer: OutlinedButton(
              onPressed: () => stops++,
              child: const Text('Stop preview'),
            ),
            content: Column(
              children: [
                for (var i = 0; i < 30; i++)
                  SizedBox(height: 48, child: Text('Action $i')),
              ],
            ),
          ),
        ),
        textScale: 1.8,
      );
      final panel = find.byKey(const ValueKey('session-controls-drag'));
      expect(tester.getSize(panel).width, 400);
      expect(tester.getSize(panel).height, 600);
      await tester.drag(
        find.byType(SingleChildScrollView),
        const Offset(0, -400),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Stop preview'));
      expect(stops, 1);
      await tester.drag(
        find.byIcon(Icons.drag_indicator_rounded),
        const Offset(-200, 300),
      );
      await tester.pumpAndSettle();
      viewport(tester, const Size(650, 900));
      await tester.pumpAndSettle();
      expect(tester.getSize(panel).width, 320);
      tester.view.viewInsets = const FakeViewPadding(bottom: 460);
      await tester.pumpAndSettle();
      expect(tester.getRect(panel).bottom, lessThanOrEqualTo(440));
      await tester.tap(find.text('Stop preview'));
      expect(stops, 2);
      await tester.tap(find.byTooltip('Ẩn điều khiển vào cạnh màn hình'));
      await tester.pumpAndSettle();
      viewport(tester, const Size(900, 650));
      await tester.pumpAndSettle();
      expect(tester.getSize(panel).width, 36);
      expect(tester.getRect(panel).bottom, lessThanOrEqualTo(190));
      await tester.tap(find.byTooltip('Hiện nút điều khiển'));
      await tester.pumpAndSettle();
      expect(
        find.byTooltip('Mở điều khiển · kéo để di chuyển'),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'resizing a live tablet session preserves keyboard ownership and read-only gates',
    (tester) async {
      final engine = ActionEngine();
      final control = active(engine);
      await show(tester, SanserHome(controller: control));
      await tester.tap(find.byTooltip('Mở điều khiển · kéo để di chuyển'));
      await tester.pumpAndSettle();
      final keyboard = find.widgetWithText(OutlinedButton, 'Bàn phím ngoài');
      await tester.ensureVisible(keyboard);
      await tester.pumpAndSettle();
      await tester.tap(keyboard);
      await tester.pump();
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(engine.input.last, ('session-a', 2, 225, true));
      final count = engine.input.length;
      viewport(tester, const Size(650, 900));
      await tester.pumpAndSettle();
      expect(engine.input.length, count);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(engine.input.last, ('session-a', 2, 225, false));
      await finish(tester, control);
      final viewer = active(engine, viewOnly: true);
      await show(tester, SanserHome(controller: viewer));
      await tester.tap(find.byTooltip('Mở điều khiển · kéo để di chuyển'));
      await tester.pumpAndSettle();
      expect(find.text('Bàn phím ngoài'), findsNothing);
      expect(find.text('Phím game'), findsNothing);
      expect(tester.takeException(), isNull);
      await finish(tester, viewer);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );
  testWidgets('large text and very short viewports keep controls recoverable', (
    tester,
  ) async {
    await show(
      tester,
      const Scaffold(
        body: FloatingSessionControls(
          initiallyCompact: false,
          content: Text('Actions'),
        ),
      ),
      size: const Size(900, 90),
      textScale: 2.5,
    );
    final panel = find.byKey(const ValueKey('session-controls-drag'));
    expect(tester.getRect(panel).bottom, lessThanOrEqualTo(90));
    expect(find.byTooltip('Mở điều khiển · kéo để di chuyển'), findsOneWidget);
    expect(tester.takeException(), isNull);
    final c = MobileController(SanserApi(MemorySecrets()), ActionEngine())
      ..account = {'email': 'demo@example.com'};
    await show(
      tester,
      SanserHome(controller: c),
      size: const Size(1024, 700),
      textScale: 2,
    );
    tester.view.viewInsets = const FakeViewPadding(bottom: 350);
    await tester.pumpAndSettle();
    expect(find.byType(NavigationRail), findsOneWidget);
    expect(tester.takeException(), isNull);
    await finish(tester, c);
  }, variant: TargetPlatformVariant.only(TargetPlatform.linux));
}
