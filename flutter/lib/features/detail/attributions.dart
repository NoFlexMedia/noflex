// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:url_launcher/url_launcher.dart';

import '../../di.dart';
import '../../graphql/queries/fragments/__generated__/provider_attribution_fields.data.gql.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../widgets/provider_logo.dart';

/// The providers behind what a title's page shows, at the foot of it. Their
/// terms ask for this wherever their data is on screen.
class WorkAttributions extends StatelessWidget {
  const WorkAttributions({super.key, required this.credits, this.server});

  final List<GProviderAttributionFields> credits;
  final String? server;

  @override
  Widget build(BuildContext context) {
    if (credits.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 34),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Divider(height: 1),
          const SizedBox(height: 12),
          for (final credit in credits)
            _Credit(credit: credit, server: server, align: credits.length > 1),
        ],
      ),
    );
  }
}

class _Credit extends StatelessWidget {
  const _Credit({required this.credit, required this.align, this.server});

  final GProviderAttributionFields credit;
  final String? server;

  /// Holds the logo to a fixed width, so several credits' text lines up.
  final bool align;

  Future<void> _open(Uri link) async {
    var opened = false;
    try {
      opened = await launchUrl(link, mode: LaunchMode.externalApplication);
    } on Exception {
      // Reported below, as for a refusal.
    }
    if (!opened) getIt<ToastService>().show('Couldn’t open ${link.host}');
  }

  @override
  Widget build(BuildContext context) {
    final link = Uri.tryParse(credit.url ?? '');
    final body = Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          if (credit.logoUrl != null) ...[
            SizedBox(
              // The widest mark at this height.
              width: align ? 56 : null,
              child: Align(
                alignment: Alignment.centerLeft,
                widthFactor: 1,
                child: ProviderLogo(credit: credit, server: server),
              ),
            ),
            const SizedBox(width: 14),
          ],
          Flexible(child: Text(credit.text, style: NfType.caption)),
        ],
      ),
    );
    if (link == null || !link.isScheme('https') && !link.isScheme('http')) {
      return body;
    }
    return Semantics(
      link: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(NfRadii.xs),
        onTap: () => unawaited(_open(link)),
        child: body,
      ),
    );
  }
}
