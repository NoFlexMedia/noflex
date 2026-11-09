// SPDX-License-Identifier: AGPL-3.0-only
//
// Downloads belong to the account that made them, so leaving it removes them.
// That can be gigabytes on a metered connection to get back, so it's asked
// before rather than discovered after. Stepping away to "Who's watching?" keeps
// them: the account is still signed in here.

import '../../di.dart';
import '../../services/session.dart';
import '../../services/tokens.dart';
import '../../widgets/nf_overlays.dart';
import 'downloads_service.dart';

Future<bool> _confirmLosingDownloads(String title, String confirmLabel) async {
  final count = getIt<DownloadsService>().items.value.length;
  if (count == 0) return true;
  return confirmAction(
    title: title,
    message:
        'This removes $count ${count == 1 ? 'download' : 'downloads'} from this device.',
    confirmLabel: confirmLabel,
    danger: true,
  );
}

Future<void> signOut() async {
  if (await _confirmLosingDownloads('Sign out?', 'Sign out')) {
    await getIt<SessionService>().logout();
  }
}

/// Leaving a server leaves everyone signed in on this device, not only whoever
/// is asking.
Future<void> changeServer() async {
  final accounts = getIt<TokenStore>().accounts.value;
  final confirmed = accounts.length > 1
      ? await confirmAction(
          title: 'Change server?',
          message:
              'This signs out all ${accounts.length} profiles on this device and removes their downloads.',
          confirmLabel: 'Change server',
          danger: true,
        )
      : await _confirmLosingDownloads('Change server?', 'Change server');
  if (confirmed) await getIt<SessionService>().forgetServer();
}
