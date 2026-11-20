import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'widget_test.dart' show MemorySecrets;

void main() {
  const challenge = {
    'error': {
      'code': 'email_verification_required',
      'message': 'A Sanser verification code was sent to your email.',
      'challengeToken': 'sn2_e_challenge',
      'challengeExpiresAt': 2000000000,
      'emailHint': 'te****@example.com',
    },
  };

  test(
    'a new phone gets the email step, then signs in with the code',
    () async {
      final secrets = MemorySecrets();
      final requests = <http.Request>[];
      final api = SanserApi(
        secrets,
        client: MockClient((request) async {
          requests.add(request);
          if (request.url.path.endsWith('/auth/login')) {
            return http.Response(jsonEncode(challenge), 401);
          }
          return http.Response(
            jsonEncode({'accessToken': 'a', 'refreshToken': 'r'}),
            200,
          );
        }),
      )..useServer('https://server.example');
      Object? thrown;
      try {
        await api.login(
          'test@example.com',
          'StrongPass123',
          deviceName: 'phone',
          platform: 'iOS',
        );
      } catch (error) {
        thrown = error;
      }
      expect(thrown, isA<EmailCodeRequired>());
      final step = thrown! as EmailCodeRequired;
      expect(step.challengeToken, 'sn2_e_challenge');
      expect(step.emailHint, 'te****@example.com');
      expect(step.register, isFalse);
      expect(
        await secrets.read('mobile.https://server.example.refresh'),
        isNull,
      );
      final login = jsonDecode(requests.first.body) as Map<String, dynamic>;
      expect(login['installationId'], isA<String>());

      await api.verifyEmailCode(step.challengeToken, '123 456');
      expect(requests.last.url.path, '/api/v2/auth/email/verify');
      expect(jsonDecode(requests.last.body), {
        'challengeToken': 'sn2_e_challenge',
        'code': '123456',
      });
      expect(requests.last.headers['authorization'], isNull);
      expect(await secrets.read('mobile.https://server.example.refresh'), 'r');
      expect(await api.accessToken(), 'a');
      api.close();
    },
  );

  test('sign-up returns the email step marked as a registration', () async {
    final api = SanserApi(
      MemorySecrets(),
      client: MockClient(
        (_) async => http.Response(jsonEncode(challenge), 401),
      ),
    )..useServer('https://server.example');
    await expectLater(
      api.login(
        'test@example.com',
        'StrongPass123',
        deviceName: 'phone',
        platform: 'iOS',
        register: true,
      ),
      throwsA(
        isA<EmailCodeRequired>().having(
          (step) => step.register,
          'register',
          true,
        ),
      ),
    );
    api.close();
  });

  test('a wrong password stays an ordinary failure', () async {
    final api = SanserApi(
      MemorySecrets(),
      client: MockClient(
        (_) async => http.Response(
          jsonEncode({
            'error': {
              'code': 'unauthorized',
              'message': 'authentication is required',
            },
          }),
          401,
        ),
      ),
    )..useServer('https://server.example');
    await expectLater(
      api.login('a@example.com', 'x', deviceName: 'phone', platform: 'iOS'),
      throwsA(
        isA<ApiFailure>().having(
          (failure) => failure.code,
          'code',
          'unauthorized',
        ),
      ),
    );
    api.close();
  });
}
