// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:gql_exec/gql_exec.dart';
import 'package:noflex/services/graphql.dart';

Response answer(Map<String, dynamic>? data, [List<GraphQLError>? errors]) =>
    Response(data: data, errors: errors, response: const {});

GraphQLError failed(String field, {String? code}) => GraphQLError(
  message: '$field failed',
  path: [field],
  extensions: code == null ? null : {'code': code},
);

void main() {
  group('refusalOf', () {
    test('a required field left out, as a wrong password answers', () {
      final refusal = refusalOf(
        answer({'__typename': 'Mutation'}, [failed('login', code: 'NOPE')]),
      );
      expect(refusal?.message, 'login failed');
      expect(refusal?.codes, ['NOPE']);
    });

    test('no data at all, as a query the server rejects answers', () {
      final refusal = refusalOf(
        answer(null, const [GraphQLError(message: 'unknown field')]),
      );
      expect(refusal?.message, 'unknown field');
    });

    test('an optional field that failed leaves the rest usable', () {
      expect(
        refusalOf(
          answer({'work': null, 'libraries': <Object>[]}, [failed('work')]),
        ),
        isNull,
      );
    });

    test('no errors', () {
      expect(refusalOf(answer({'login': <String, Object>{}})), isNull);
    });
  });
}
