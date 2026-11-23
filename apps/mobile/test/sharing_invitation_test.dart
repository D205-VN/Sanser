import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'controller_test.dart' show TestEngine;
import 'widget_test.dart' show MemorySecrets;

void main() {
  test('a new screen-sharing invitation is announced once', () async {
    var viewer = '';
    var state = 'invited';
    final native = TestEngine();
    final api = SanserApi(
      MemorySecrets(),
      client: MockClient((request) async {
        final body = switch (request.url.path) {
          '/api/v2/auth/login' => {'accessToken': 'a', 'refreshToken': 'r'},
          '/api/v2/account' => {'email': 'test@example.com'},
          '/api/v2/devices/register' => {
            'id': '6b02e0b1-8b6d-44df-b3c1-428dc78ca7c6',
          },
          '/api/v2/sharing' => {
            'items': [
              {
                'id': 'group',
                'state': 'active',
                'hostName': 'MacBook',
                'members': [
                  {'id': 'member', 'viewerDeviceId': viewer, 'state': state},
                ],
              },
            ],
          },
          '/api/v2/devices' => {'items': []},
          _ => <String, dynamic>{},
        };
        return http.Response(jsonEncode(body), 200);
      }),
    );
    final c = MobileController(api, native);
    await c.initialize();
    await c.login('test@example.com', 'password', 'https://server.example');
    expect(c.account, isNotNull);
    expect(c.pendingInvitations, isEmpty);
    viewer = c.deviceId;
    await c.refresh();
    expect(c.pendingInvitations, hasLength(1));
    expect(c.invitationSignal, 1);
    expect(c.newInvitationHost, 'MacBook');
    await c.refresh();
    expect(c.invitationSignal, 1, reason: 'the same invitation alerts once');
    state = 'accepted';
    await c.refresh();
    expect(c.pendingInvitations, isEmpty);
    c.dispose();
    await native.events.close();
  });
}
