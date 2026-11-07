import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/host_shortcuts.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'session_actions_test.dart'
    show ActionEngine, active, showControls, finish;

class SlowKeys extends ActionEngine {
  Completer<void>? pending;
  int? failOn;
  int? failOnRelease;
  @override
  Future<void> remoteInput(
    String id,
    int kind, {
    int x = 0,
    int y = 0,
    int code = 0,
    int delta = 0,
    bool down = false,
  }) async {
    input.add((id, kind, code, down));
    if (kind == 2 && down) {
      final gate = pending;
      pending = null;
      await gate?.future;
      if (code == failOn) throw StateError('transport refused key');
    }
    if (kind == 2 && !down && code == failOnRelease) {
      throw StateError('transport refused key release');
    }
  }
}

Future<void> flush() => Future<void>.delayed(Duration.zero);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'platform inference uses declared host OS only, never a name or phone default',
    () {
      for (final value in ['macOS', ' Darwin ', 'mac', 'osx']) {
        expect(shortcutHost(value), ShortcutHost.macos);
      }
      for (final value in ['Windows', 'win32', ' WIN64 ']) {
        expect(shortcutHost(value), ShortcutHost.windows);
      }
      for (final value in [
        null,
        '',
        'ios',
        'android',
        'linux',
        'MacBook Air',
        'Windows laptop',
        'browser-development',
      ]) {
        expect(shortcutHost(value), isNull);
      }
    },
  );

  test(
    'fixed Mac/Windows profiles send balanced HID edges, including all modifier releases',
    () async {
      final engine = ActionEngine(),
          input = MobileInput(engine, 's', enabled: true);
      for (final host in ShortcutHost.values) {
        final actions = hostShortcuts(host);
        expect(
          actions.map((action) => action.id).toSet().length,
          actions.length,
        );
        for (final action in actions) {
          engine.input.clear();
          expect(await input.chord(action.codes), true, reason: action.id);
          expect(engine.input, [
            for (final code in action.codes) ('s', 2, code, true),
            for (final code in action.codes.reversed) ('s', 2, code, false),
          ], reason: '${host.name}/${action.id}');
        }
      }
      List<int> codes(ShortcutHost host, String id) =>
          hostShortcuts(host).singleWhere((action) => action.id == id).codes;
      expect(codes(ShortcutHost.macos, 'close'), [227, 26]);
      expect(codes(ShortcutHost.windows, 'close'), [226, 61]);
      expect(codes(ShortcutHost.macos, 'redo'), [227, 225, 29]);
      expect(codes(ShortcutHost.windows, 'redo'), [224, 28]);
      input.dispose();
      await input.drain();
    },
  );

  test(
    'invalid/read-only/stale and held-key chords emit nothing and do not steal another owner',
    () async {
      final engine = ActionEngine(),
          input = MobileInput(engine, 's', enabled: true);
      final viewer = MobileInput(engine, 'v', enabled: false);
      expect(await viewer.chord([227, 26]), false);
      for (final codes in <List<int>>[
        [],
        [227, 227],
        [0],
        [50],
        [70],
        [101],
        [116],
        [232],
        [224, 225, 226, 227, 4],
      ]) {
        expect(await input.chord(codes), false);
      }
      expect(await input.chord([227, 26], current: () => false), false);
      expect(engine.input, isEmpty);
      await input.hold(225, true, 'external-keyboard');
      expect(await input.chord([227, 26]), false);
      expect(engine.input, [('s', 2, 225, true)]);
      await input.hold(225, false, 'external-keyboard');
      expect(engine.input.last, ('s', 2, 225, false));
      input.dispose();
      await input.drain();
      viewer.dispose();
    },
  );

  test(
    'repeated taps are bounded; reset while awaiting modifier cancels all remaining presses',
    () async {
      final engine = SlowKeys(),
          input = MobileInput(engine, 's', enabled: true);
      final gate = Completer<void>();
      engine.pending = gate;
      final pending = input.chord([227, 225, 26]);
      await flush();
      expect(engine.input, [('s', 2, 227, true)]);
      expect(await input.chord([224, 4]), false);
      final reset = input.releaseAll();
      gate.complete();
      expect(await pending, false);
      await reset;
      expect(engine.input, [('s', 2, 227, true), ('s', 4, 0, false)]);
      expect(await input.chord([224, 4]), true);
      input.dispose();
      await input.drain();
    },
  );

  test(
    'native error resets instead of sending a shortcut after a rejected modifier',
    () async {
      final engine = SlowKeys()..failOn = 227;
      var errors = 0;
      final input = MobileInput(
        engine,
        's',
        enabled: true,
        onError: () => errors++,
      );
      expect(await input.chord([227, 26]), false);
      expect(engine.input, [('s', 2, 227, true), ('s', 4, 0, false)]);
      expect(errors, 1);
      engine.failOn = null;
      expect(await input.chord([224, 4]), true);
      input.dispose();
      await input.drain();
    },
  );

  test(
    'cancel while awaiting a key releases only chord ownership and never presses a later key',
    () async {
      final engine = SlowKeys(),
          input = MobileInput(engine, 's', enabled: true);
      var current = true;
      final gate = Completer<void>();
      engine.pending = gate;
      final pending = input.chord([227, 26], current: () => current);
      await flush();
      // An independent source acquired the same modifier during the native wait.
      await input.hold(227, true, 'other');
      current = false;
      gate.complete();
      expect(await pending, false);
      expect(engine.input, [('s', 2, 227, true)]);
      await input.hold(227, false, 'other');
      expect(engine.input.last, ('s', 2, 227, false));
      input.dispose();
      await input.drain();
    },
  );

  test(
    'snapshot cannot change under a pending send; a failed key-up resets and reports failure',
    () async {
      final engine = SlowKeys(),
          input = MobileInput(engine, 's', enabled: true);
      final keys = [227, 26], gate = Completer<void>();
      engine.pending = gate;
      final pending = input.chord(keys);
      await flush();
      keys
        ..clear()
        ..add(4);
      gate.complete();
      expect(await pending, true);
      expect(engine.input, [
        ('s', 2, 227, true),
        ('s', 2, 26, true),
        ('s', 2, 26, false),
        ('s', 2, 227, false),
      ]);
      engine.input.clear();
      engine.failOnRelease = 26;
      expect(await input.chord([227, 26]), false);
      expect(engine.input.last, ('s', 4, 0, false));
      input.dispose();
      await input.drain();
    },
  );

  testWidgets(
    'background and resume during native wait cannot revive half of a shortcut',
    (tester) async {
      final engine = SlowKeys(), c = active(engine);
      c.active!['hostDeviceId'] = 'h';
      c.devices = [
        {'id': 'h', 'platform': 'macos'},
      ];
      await showControls(tester, c);
      await tester.tap(find.text('Phím tắt máy tính'));
      await tester.pumpAndSettle();
      final gate = Completer<void>();
      engine.pending = gate;
      final button = find.byKey(const ValueKey('host-shortcut-switch'));
      await tester.tap(button);
      await tester.pump();
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      await tester.tap(button);
      await tester.pump();
      gate.complete();
      await tester.pumpAndSettle();
      expect(engine.input.where((event) => event.$2 == 2), [
        ('session-a', 2, 227, true),
      ]);
      expect(engine.input.last.$2, 4);
      await tester.tap(find.byTooltip('Đóng bảng phím tắt'));
      await tester.pumpAndSettle();
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'unknown host requires explicit OS selection; inferred host comes from the matching device',
    (tester) async {
      final engine = ActionEngine(), c = active(engine);
      c.devices = [
        {'id': 'other-device', 'platform': 'macos'},
      ];
      await showControls(tester, c);
      await tester.tap(find.text('Phím tắt máy tính'));
      await tester.pumpAndSettle();
      expect(
        find.text('Chọn hệ điều hành của máy đang được điều khiển.'),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('host-shortcut-switch')), findsNothing);
      expect(engine.input.where((event) => event.$2 == 2), isEmpty);
      await tester.tap(find.text('Windows'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('host-shortcut-switch')));
      await tester.pumpAndSettle();
      expect(engine.input.where((event) => event.$2 == 2), [
        ('session-a', 2, 226, true),
        ('session-a', 2, 43, true),
        ('session-a', 2, 43, false),
        ('session-a', 2, 226, false),
      ]);
      await tester.tap(find.byTooltip('Đóng bảng phím tắt'));
      await tester.pumpAndSettle();
      c.active!['hostDeviceId'] = 'host-mac';
      c.devices.add({'id': 'host-mac', 'platform': 'macOS'});
      await tester.tap(find.text('Phím tắt máy tính'));
      await tester.pumpAndSettle();
      final segment = tester.widget<SegmentedButton<ShortcutHost>>(
        find.byType(SegmentedButton<ShortcutHost>),
      );
      expect(segment.selected, {ShortcutHost.macos});
      expect(find.text('Thu nhỏ cửa sổ'), findsOneWidget);
      expect(find.text('Cmd+M'), findsOneWidget);
      expect(find.text('Win+D'), findsNothing);
      await tester.tap(find.byTooltip('Đóng bảng phím tắt'));
      await tester.pumpAndSettle();
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets('view-only and pending sessions cannot open host shortcuts', (
    tester,
  ) async {
    final engine = ActionEngine(), viewer = active(engine, viewOnly: true);
    await showControls(tester, viewer);
    expect(find.text('Phím tắt máy tính'), findsNothing);
    await finish(tester, viewer);
    final pending = active(engine)..phase = SessionPhase.approval;
    await showControls(tester, pending);
    final button = find.ancestor(
      of: find.text('Phím tắt máy tính'),
      matching: find.byType(OutlinedButton),
    );
    expect(tester.widget<OutlinedButton>(button).onPressed, isNull);
    await finish(tester, pending);
  }, variant: TargetPlatformVariant.only(TargetPlatform.linux));

  testWidgets(
    'large-text short landscape sheet scrolls; disconnect disables keys without closing the app',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(640, 320);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      final engine = ActionEngine(),
          input = MobileInput(engine, 's', enabled: true);
      var ready = true;
      late StateSetter update;
      await tester.pumpWidget(
        MaterialApp(
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(textScaler: TextScaler.linear(1.6)),
            child: child!,
          ),
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () => showModalBottomSheet<void>(
                  context: context,
                  isScrollControlled: true,
                  builder: (_) => StatefulBuilder(
                    builder: (_, setState) {
                      update = setState;
                      return HostShortcutsSheet(
                        input: input,
                        current: () => ready,
                        detectedHost: ShortcutHost.windows,
                      );
                    },
                  ),
                ),
                child: const Text('open'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(
        find.byKey(const ValueKey('host-shortcut-save')),
      );
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      update(() => ready = false);
      await tester.pumpAndSettle();
      for (final button in tester.widgetList<OutlinedButton>(
        find.byType(OutlinedButton),
      )) {
        expect(button.onPressed, isNull);
      }
      expect(engine.input.where((event) => event.$2 == 2), isEmpty);
      await tester.tap(find.byTooltip('Đóng bảng phím tắt'));
      await tester.pumpAndSettle();
      expect(find.text('open'), findsOneWidget);
      input.dispose();
      await input.drain();
    },
  );

  testWidgets(
    'closing the sheet during native send cancels the rest; controls do not queue repeated chords',
    (tester) async {
      final engine = SlowKeys(), c = active(engine);
      c.active!['hostDeviceId'] = 'h';
      c.devices = [
        {'id': 'h', 'platform': 'macos'},
      ];
      await showControls(tester, c);
      await tester.tap(find.text('Phím tắt máy tính'));
      await tester.pumpAndSettle();
      final gate = Completer<void>();
      engine.pending = gate;
      final button = find.byKey(const ValueKey('host-shortcut-switch'));
      await tester.tap(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pump();
      expect(engine.input.where((event) => event.$2 == 2), [
        ('session-a', 2, 227, true),
      ]);
      await tester.tap(find.byTooltip('Đóng bảng phím tắt'));
      await tester.pumpAndSettle();
      gate.complete();
      await tester.pumpAndSettle();
      expect(engine.input.where((event) => event.$2 == 2), [
        ('session-a', 2, 227, true),
      ]);
      expect(engine.input.last.$2, 4);
      expect(tester.takeException(), isNull);
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );
}
