// SPDX-License-Identifier: AGPL-3.0-only

import 'package:built_value/serializer.dart';

/// The schema's `DateTime`: RFC 3339 on the wire, UTC when sent.
class DateTimeSerializer extends PrimitiveSerializer<DateTime> {
  DateTimeSerializer();

  @override
  Iterable<Type> get types => const [DateTime];

  @override
  String get wireName => 'DateTime';

  @override
  Object serialize(
    Serializers serializers,
    DateTime object, {
    FullType specifiedType = FullType.unspecified,
  }) => object.toUtc().toIso8601String();

  @override
  DateTime deserialize(
    Serializers serializers,
    Object serialized, {
    FullType specifiedType = FullType.unspecified,
  }) => DateTime.parse(serialized as String);
}
