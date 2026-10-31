import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'widget_test.dart' show MemorySecrets;

void main() {
  test('HTTPS URL rejects credentials, query, insecure schemes', () {
    for (final url in [
      'http://example.com',
      'https://user:pass@example.com',
      'https://example.com?token=x',
      'https://example.com#x',
      'not a server',
    ]) {
      expect(() => serverUri(url), throwsA(isA<ApiFailure>()));
    }
    expect(
      serverUri(' https://example.com/base/ ').toString(),
      'https://example.com/base',
    );
  });
  test('stable installation identity is server scoped', () async {
    final api = SanserApi(MemorySecrets())..useServer('https://a.example');
    final first = await api.identity('installation');
    expect(await api.identity('installation'), first);
    api.useServer('https://b.example');
    expect(await api.identity('installation'), isNot(first));
    api.close();
  });
  test('expired access refreshes once and retries authorization', () async {
    final secrets = MemorySecrets();
    int calls = 0, refreshes = 0;
    final api = SanserApi(
      secrets,
      client: MockClient((request) async {
        if (request.url.path.endsWith('/login')) {
          return http.Response(
            jsonEncode({'accessToken': 'expired', 'refreshToken': 'refresh-1'}),
            200,
          );
        }
        if (request.url.path.endsWith('/refresh')) {
          refreshes++;
          return http.Response(
            jsonEncode({'accessToken': 'current', 'refreshToken': 'refresh-2'}),
            200,
          );
        }
        calls++;
        return request.headers['Authorization'] == 'Bearer current'
            ? http.Response('{"items":[]}', 200)
            : http.Response('{}', 401);
      }),
    )..useServer('https://server.example');
    await api.login(
      'a@example.com',
      'secret',
      deviceName: 'phone',
      platform: 'Android',
    );
    expect(await api.list('/api/v2/devices'), isEmpty);
    expect(calls, 2);
    expect(refreshes, 1);
    expect(secrets.values.values, contains('refresh-2'));
    expect(secrets.values.values, isNot(contains('expired')));
    api.close();
  });
  test(
    'concurrent requests share refresh and logout clears secure credentials',
    () async {
      final secrets = MemorySecrets();
      int refreshes = 0;
      final api = SanserApi(
        secrets,
        client: MockClient((request) async {
          if (request.url.path.endsWith('/refresh')) {
            refreshes++;
            await Future<void>.delayed(const Duration(milliseconds: 1));
            return http.Response(
              '{"accessToken":"a","refreshToken":"r2"}',
              200,
            );
          }
          return http.Response('{}', 204);
        }),
      )..useServer('https://server.example');
      await secrets.write('mobile.https://server.example.refresh', 'r');
      await api.restore();
      expect(await Future.wait([api.accessToken(), api.accessToken()]), [
        'a',
        'a',
      ]);
      expect(refreshes, 1);
      await api.logout();
      expect(secrets.values, isEmpty);
      api.close();
    },
  );
  test('malformed and oversized response are bounded', () async {
    for (final content in ['not json', jsonEncode('x' * 1048577)]) {
      final api = SanserApi(
        MemorySecrets(),
        client: MockClient((_) async => http.Response(content, 200)),
      )..useServer('https://server.example');
      await expectLater(
        api.request('/api/v2/auth/login', auth: false),
        throwsA(isA<ApiFailure>()),
      );
      api.close();
    }
  });
  test(
    'switching servers during login does not install old access token',
    () async {
      final response = Completer<http.Response>();
      final api = SanserApi(
        MemorySecrets(),
        client: MockClient((_) => response.future),
      )..useServer('https://a.example');
      final login = api.login(
        'a@example.com',
        'secret',
        deviceName: 'phone',
        platform: 'Android',
      );
      await Future<void>.delayed(Duration.zero);
      api.useServer('https://b.example');
      response.complete(
        http.Response('{"accessToken":"a","refreshToken":"r"}', 200),
      );
      await login;
      await expectLater(api.accessToken(), throwsA(isA<ApiFailure>()));
      api.close();
    },
  );
}
