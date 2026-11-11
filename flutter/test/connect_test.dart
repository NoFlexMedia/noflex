// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/auth/connect_screen.dart';

void main() {
  test('normalizeOrigin takes what people actually type', () {
    expect(normalizeOrigin('192.168.1.20:28793'), 'http://192.168.1.20:28793');
    expect(
      normalizeOrigin('  media.example.com/ '),
      'http://media.example.com',
    );
    expect(
      normalizeOrigin('https://media.example.com/graphql'),
      'https://media.example.com',
    );
    expect(normalizeOrigin('http://nas.local:28793/'), 'http://nas.local:28793');
  });

  test('normalizeOrigin refuses what can’t be a server', () {
    expect(normalizeOrigin(''), isNull);
    expect(normalizeOrigin('ftp://nas.local'), isNull);
  });
}
