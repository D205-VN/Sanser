import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/preferences.dart';
import 'package:sanser_mobile/src/chat_sheet.dart';
import 'package:sanser_mobile/src/chat.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'network_recovery_test.dart' show RecoveryFixture;

void collab(RecoveryFixture f, Object payload) => f.native.events.add({
  'state': 'collab',
  'sessionId': 'session',
  'engineAttempt': f.native.configs.last['engineAttempt'],
  'payload': payload,
});

void main() {
  group('SessionChat', () {
    test('accepts plain text and rejects control characters or 1 KB+', () {
      expect(SessionChat.valid('Xin chào 😀\nDòng 2'), true);
      expect(SessionChat.valid('   '), false);
      expect(SessionChat.valid('a\u0007b'), false);
      expect(SessionChat.valid('ế' * 341), true); // 1023 bytes.
      expect(SessionChat.valid('ế' * 342), false); // 1026 bytes.
    });

    test('composes the desktop wire format', () {
      final chat = SessionChat();
      final sent = chat.compose('Chào máy tính', 'An\u0000 Phone')!;
      final wire = jsonDecode(sent.payload) as Map;
      expect(wire['type'], 'chat');
      expect(wire['text'], 'Chào máy tính');
      expect(wire['name'], 'An Phone');
      expect(RegExp(r'^[0-9a-f]{16}$').hasMatch(wire['id'] as String), true);
      expect(sent.message.own, true);
      expect(chat.compose('', 'x'), isNull);
    });

    test('ignores drawing, policy, malformed and duplicate records', () {
      final chat = SessionChat();
      expect(chat.receive('{"type":"stroke","id":"s1","color":0}'), isNull);
      expect(chat.receive('{"type":"policy","annotations":true}'), isNull);
      expect(chat.receive('not json'), isNull);
      expect(chat.receive('{"type":"chat","id":"bad id","text":"x"}'), isNull);
      final first = chat.receive(
        '{"type":"chat","id":"h1","name":"Máy A","text":"<b>hi</b>"}',
      );
      expect(first?.text, '<b>hi</b>');
      expect(first?.name, 'Máy A');
      expect(chat.receive('{"type":"chat","id":"h1","text":"again"}'), isNull);
      expect(
        chat.receive('{"type":"chat","id":"h2","text":"no name"}')?.name,
        'Máy tính',
      );
      for (var i = 0; i < 150; i++) {
        chat.receive('{"type":"chat","id":"m$i","text":"$i"}');
      }
      expect(chat.messages.length, SessionChat.history);
    });
  });

  testWidgets('chat waits for the host capability, then sends and receives', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    expect(f.c.chatAvailable, false);
    expect(await f.c.sendChat('hello'), false);
    f.native.events.add({
      'state': 'frame',
      'sessionId': 'session',
      'engineAttempt': f.native.configs.last['engineAttempt'],
      'collab': true,
    });
    expect(f.c.chatAvailable, true);
    expect(await f.c.sendChat('Xin chào'), true);
    expect(f.native.chats.single, contains('"text":"Xin chào"'));
    expect(f.c.chat.messages.single.own, true);

    collab(f, '{"type":"chat","id":"h1","name":"Máy A","text":"Chào bạn"}');
    expect(f.c.chat.messages.last.text, 'Chào bạn');
    expect(f.c.unreadChat, 1);
    f.c.setChatOpen(true);
    expect(f.c.unreadChat, 0);
    collab(f, '{"type":"chat","id":"h2","text":"đang mở"}');
    expect(f.c.unreadChat, 0);

    // Old engine attempts cannot inject messages.
    f.native.events.add({
      'state': 'collab',
      'sessionId': 'session',
      'engineAttempt': -1,
      'payload': '{"type":"chat","id":"old","text":"cũ"}',
    });
    expect(f.c.chat.messages.length, 3);

    await f.c.disconnect();
    expect(f.c.chat.messages, isEmpty);
    await f.close();
  });

  testWidgets(
    'leaving the app keeps the session, hides video and notifies chat',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.c.setForeground(false);
      await f.c.backgrounded();
      expect(f.native.backgrounds.last, true);
      expect(f.c.active, isNotNull);
      expect(f.c.phase, SessionPhase.streaming);
      collab(
        f,
        '{"type":"chat","id":"h1","name":"Máy A","text":"Có ở đó không?"}',
      );
      expect(f.native.notifications.single, ('Máy A', 'Có ở đó không?'));
      f.c.setForeground(true);
      await f.c.foregrounded();
      expect(f.native.backgrounds.last, false);
      expect(f.native.configs.length, 1, reason: 'no reconnect needed');
      await f.close();
    },
  );

  testWidgets('a session dropped in the background reconnects on return', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    f.c.setForeground(false);
    await f.c.backgrounded();
    f.native.events.add({
      'state': 'stopped',
      'sessionId': 'session',
      'engineAttempt': f.native.configs.last['engineAttempt'],
      'message': 'timeout',
    });
    expect(f.c.active, isNotNull, reason: 'still the same session');
    expect(f.c.phase, SessionPhase.reconnecting);
    expect(f.native.notifications.single.$1, 'Sanser mất kết nối');
    await tester.pump(const Duration(seconds: 2));
    expect(f.native.configs.length, 1, reason: 'no reconnect while hidden');

    f.c.setForeground(true);
    await f.c.foregrounded();
    await tester.pump(const Duration(seconds: 1));
    await tester.pump(const Duration(milliseconds: 10));
    expect(f.native.configs.length, 2);
    f.native.frame();
    expect(f.c.phase, SessionPhase.streaming);
    expect(f.c.routeNotice, 'Đã kết nối lại sau khi quay lại Sanser.');
    await f.close();
  });

  testWidgets('keep-alive off still disconnects when the app leaves', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    await f.c.setPreferences(f.c.preferences.copyWith(backgroundMinutes: 0));
    await f.c.backgrounded();
    expect(f.c.active, isNull);
    await f.close();
  });

  testWidgets('the background limit ends the session and says why', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    await f.c.setPreferences(f.c.preferences.copyWith(backgroundMinutes: 5));
    await f.c.backgrounded();
    await tester.pump(const Duration(minutes: 5, seconds: 1));
    expect(f.c.active, isNull);
    expect(f.native.notifications.last.$1, 'Sanser đã ngắt kết nối');
    await f.close();
  });

  testWidgets('smooth playback setting reaches the native player', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    expect(f.native.smooth.last, false);
    await f.c.setPreferences(f.c.preferences.copyWith(smoothVideo: true));
    expect(f.native.smooth.last, true);
    final count = f.native.smooth.length;
    await f.c.setPreferences(f.c.preferences.copyWith(haptics: true));
    expect(f.native.smooth.length, count);
    expect(
      MobilePreferences.fromJson(f.c.preferences.toJson()).smoothVideo,
      true,
    );
    await f.close();
  });

  testWidgets('picture in picture follows the session and preference', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    expect(f.native.pip.last, true);
    await f.c.setPreferences(f.c.preferences.copyWith(pictureInPicture: false));
    expect(f.native.pip.last, false);
    await f.c.setPreferences(f.c.preferences.copyWith(pictureInPicture: true));
    expect(f.native.pip.last, true);
    f.native.events.add({'state': 'pip', 'active': true});
    expect(f.c.pictureInPicture, true);
    await f.c.disconnect();
    expect(f.native.pip.last, false);
    await f.close();
  });

  testWidgets('chat sheet sends, shows replies as text and clears unread', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    f.native.events.add({
      'state': 'frame',
      'sessionId': 'session',
      'engineAttempt': f.native.configs.last['engineAttempt'],
      'collab': true,
    });
    collab(
      f,
      '{"type":"chat","id":"h1","name":"Máy A","text":"<i>xin chào</i>"}',
    );
    expect(f.c.unreadChat, 1);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: ChatSheet(controller: f.c)),
      ),
    );
    await tester.pump();
    expect(f.c.unreadChat, 0);
    expect(find.text('<i>xin chào</i>'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('chat-input')), 'Đã thấy');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('chat-send')));
    await tester.pump();
    expect(f.native.chats.single, contains('Đã thấy'));
    expect(find.text('Đã thấy'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    expect(f.c.chatOpen, false);
    await f.close();
  });
}
