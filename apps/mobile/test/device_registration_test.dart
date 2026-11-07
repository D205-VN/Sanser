import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'widget_test.dart' show MemorySecrets;

void main() {
  const canonical = '6b02e0b1-8b6d-44df-b3c1-428dc78ca7c6';
  const oldForeign = '115ebde0-2908-4e53-a045-08412f46ac27';
  Future<void> login(SanserApi api) => api.login(
    'phone@example.test',
    'test-password',
    deviceName: 'phone',
    platform: 'iOS',
  );

  test(
    'account switch isolates IDs and canonical ID survives reopening',
    () async {
      final secrets = MemorySecrets();
      final sent = <String>[];
      final client = MockClient((request) async {
        if (request.url.path.endsWith('/login')) {
          return http.Response('{"accessToken":"a","refreshToken":"r"}', 200);
        }
        sent.add((jsonDecode(request.body) as Json)['id'] as String);
        return http.Response(jsonEncode({'id': canonical}), 200);
      });
      final api = SanserApi(secrets, client: client)
        ..useServer('https://server.example');
      await login(api);
      final computer = await api.identity('computer');
      expect(
        await api.registerClientDevice({
          'computerId': computer,
        }, accountId: 'A'),
        canonical,
      );
      await api.registerClientDevice({'computerId': computer}, accountId: 'B');
      expect(sent[1], isNot(sent[0]));
      expect(sent[1], isNot(canonical));
      expect(await api.identity('computer'), computer);

      final reopened = SanserApi(secrets, client: client)
        ..useServer('https://server.example');
      await login(reopened);
      await reopened.registerClientDevice({}, accountId: 'A');
      expect(sent.last, canonical);
      reopened.useServer('https://other.example');
      await login(reopened);
      await reopened.registerClientDevice({}, accountId: 'A');
      expect(sent.last, isNot(canonical));
      api.close();
    },
  );

  test(
    'foreign restored ID is repaired once and accepted ID is persisted',
    () async {
      final secrets = MemorySecrets();
      secrets.values['mobile.https://server.example.client-device.account.A'] =
          oldForeign;
      final ids = <String>[];
      final api = SanserApi(
        secrets,
        client: MockClient((request) async {
          if (request.url.path.endsWith('/login')) {
            return http.Response('{"accessToken":"a","refreshToken":"r"}', 200);
          }
          ids.add((jsonDecode(request.body) as Json)['id'] as String);
          return ids.length == 1
              ? http.Response(
                  '{"error":{"message":"device id is already registered"}}',
                  409,
                )
              : http.Response(jsonEncode({'id': canonical}), 200);
        }),
      )..useServer('https://server.example');
      await login(api);
      expect(await api.registerClientDevice({}, accountId: 'A'), canonical);
      expect(ids, hasLength(2));
      expect(ids.first, oldForeign);
      expect(ids.last, isNot(oldForeign));
      await api.registerClientDevice({}, accountId: 'A');
      expect(ids.last, canonical);
      api.close();
    },
  );

  test('persistent conflict is bounded to two requests', () async {
    var registrations = 0;
    final api = SanserApi(
      MemorySecrets(),
      client: MockClient((request) async {
        if (request.url.path.endsWith('/login')) {
          return http.Response('{"accessToken":"a","refreshToken":"r"}', 200);
        }
        registrations++;
        return http.Response(
          '{"error":{"message":"device id is already registered"}}',
          409,
        );
      }),
    )..useServer('https://server.example');
    await login(api);
    await expectLater(
      api.registerClientDevice({}, accountId: 'A'),
      throwsA(isA<ApiFailure>().having((e) => e.status, 'status', 409)),
    );
    expect(registrations, 2);
    api.close();
  });

  test(
    'protocol and server failures never rotate or retry the device ID',
    () async {
      for (final status in [403, 409, 500]) {
        var registrations = 0;
        final secrets = MemorySecrets();
        final key = 'mobile.https://server.example.client-device.account.A';
        secrets.values[key] = oldForeign;
        final api = SanserApi(
          secrets,
          client: MockClient((request) async {
            if (request.url.path.endsWith('/login')) {
              return http.Response(
                '{"accessToken":"a","refreshToken":"r"}',
                200,
              );
            }
            registrations++;
            return http.Response(
              '{"error":{"message":"protocol mismatch"}}',
              status,
            );
          }),
        )..useServer('https://server.example');
        await login(api);
        await expectLater(
          api.registerClientDevice({}, accountId: 'A'),
          throwsA(isA<ApiFailure>().having((e) => e.status, 'status', status)),
        );
        expect(registrations, 1);
        expect(secrets.values[key], oldForeign);
        api.close();
      }
    },
  );
}
