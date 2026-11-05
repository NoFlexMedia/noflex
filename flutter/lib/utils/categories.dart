// SPDX-License-Identifier: AGPL-3.0-only
//
// The server's `ContentCategory` values, as the app names and draws them.

import 'package:flutter/widgets.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

import '../graphql/__generated__/schema.schema.gql.dart';

class Category {
  const Category(
    this.value,
    this.label,
    this.icon, {
    this.shelves = false,
    this.workCount = 0,
  });

  final GContentCategory value;
  final String label;
  final IconData icon;

  /// Opens on recommendations and new arrivals, with the full grid a tab away.
  final bool shelves;
  final int workCount;

  Category withCount(int count) =>
      Category(value, label, icon, shelves: shelves, workCount: count);
}

final categories = <Category>[
  Category(
    GContentCategory.MOVIES,
    'Movies',
    PhosphorIconsRegular.filmStrip,
    shelves: true,
  ),
  Category(
    GContentCategory.SHOWS,
    'TV Shows',
    PhosphorIconsRegular.television,
    shelves: true,
  ),
  Category(
    GContentCategory.ANIME,
    'Anime',
    PhosphorIconsRegular.sparkle,
    shelves: true,
  ),
  Category(GContentCategory.MUSIC, 'Music', PhosphorIconsRegular.musicNotes),
  Category(
    GContentCategory.AUDIOBOOKS,
    'Audiobooks',
    PhosphorIconsRegular.bookOpen,
  ),
  Category(GContentCategory.PHOTOS, 'Photos', PhosphorIconsRegular.image),
];
