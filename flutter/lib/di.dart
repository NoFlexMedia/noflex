// SPDX-License-Identifier: AGPL-3.0-only

import 'package:get_it/get_it.dart';

import 'features/downloads/downloads_service.dart';
import 'features/home/home_service.dart';
import 'features/player/decoder_probe.dart';
import 'features/player/playback_service.dart';
import 'services/device_identity.dart';
import 'services/graphql.dart';
import 'services/nav_service.dart';
import 'services/network_service.dart';
import 'services/session.dart';
import 'services/toast_service.dart';
import 'services/tokens.dart';
import 'services/watch_state.dart';

final getIt = GetIt.instance;

void configureDependencies() {
  getIt
    ..registerSingleton(TokenStore())
    ..registerSingleton(DeviceIdentity())
    ..registerSingleton(NavService())
    ..registerSingleton(ToastService())
    ..registerSingleton(NetworkService())
    ..registerSingleton(WatchStateService())
    ..registerSingleton(DecoderProbeService())
    ..registerSingleton(GraphqlService(getIt<TokenStore>()))
    ..registerSingleton(
      SessionService(
        tokens: getIt<TokenStore>(),
        graphql: getIt<GraphqlService>(),
        identity: getIt<DeviceIdentity>(),
        decoders: getIt<DecoderProbeService>(),
        watchState: getIt<WatchStateService>(),
      ),
    )
    ..registerSingleton(DownloadsService())
    ..registerLazySingleton(HomeService.new)
    ..registerSingleton(PlaybackService());
}
