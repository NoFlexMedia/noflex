// SPDX-License-Identifier: AGPL-3.0-only
//
// Downloads belong to the account that made them, so leaving it removes them.
// That can be gigabytes on a metered connection to get back, so it's asked
// before rather than discovered after.

import '../../di.dart';
import '../../services/session.dart';
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

Future<void> changeServer() async {
  if (await _confirmLosingDownloads('Change server?', 'Change server')) {
    await getIt<SessionService>().forgetServer();
  }
}
