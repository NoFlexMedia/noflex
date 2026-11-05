// SPDX-License-Identifier: AGPL-3.0-only
//
// Signals are the only way this app does reactivity. Nothing in the analyzer
// knows that, so it's checked here.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

const forbidden = {
  'setState(': 'Write to a signal; read it in a SignalBuilder.',
  'ChangeNotifier': 'Use a signal.',
  'ValueNotifier': 'Use a signal.',
  'ValueListenableBuilder': 'Use SignalBuilder.',
  'ListenableBuilder': 'Use SignalBuilder.',
  'StreamBuilder':
      'Mirror the stream into a signal at the seam; see graphql/query.dart.',
  'FutureBuilder': 'Hold the result in a signal.',
  'SignalsMixin': 'Deprecated in signals 7. Use SignalWidget or SignalBuilder.',
  'Watch(': 'Deprecated in signals 7. Use SignalBuilder.',
  '.watch(context)': 'Deprecated in signals 7. Use SignalBuilder.',
  'package:provider': 'Read services through getIt.',
  'package:flutter_riverpod': 'Read services through getIt.',
  'package:flutter_bloc': 'Read services through getIt.',
  'package:ferry_flutter': 'Use Query from graphql/query.dart.',
  'Icons.': 'Icons are Phosphor.',
};

void main() {
  test('reactivity is signals, everywhere', () {
    final problems = <String>[];
    final sources = Directory('lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where(
          (file) =>
              file.path.endsWith('.dart') &&
              !file.path.contains('__generated__'),
        );
    for (final file in sources) {
      final lines = file.readAsLinesSync();
      for (var i = 0; i < lines.length; i++) {
        final line = lines[i];
        if (line.trimLeft().startsWith('//')) continue;
        for (final entry in forbidden.entries) {
          final hit = entry.key == 'Icons.'
              ? RegExp(r'(^|[^A-Za-z])Icons\.').hasMatch(line)
              : line.contains(entry.key);
          if (hit) {
            problems.add(
              '${file.path}:${i + 1}: ${entry.key} — ${entry.value}',
            );
          }
        }
      }
    }
    expect(problems, isEmpty, reason: problems.join('\n'));
  });
}
