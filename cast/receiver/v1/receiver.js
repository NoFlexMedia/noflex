// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex's Cast receiver. It plays what a sender loads and nothing else: it makes
// no requests to a NoFlex server, so it works whatever the server's address or
// scheme, and nothing it is sent can be mistaken for a credential. The sender
// (web/src/lib/cast.svelte.ts) owns the session, resumes from the viewer's place
// and reports progress. `customData.protocol` says which dialect a sender speaks.

const SUPPORTED_PROTOCOL = 1;

const context = cast.framework.CastReceiverContext.getInstance();
const player = context.getPlayerManager();
const messages = cast.framework.messages;

player.setMessageInterceptor(messages.MessageType.LOAD, (request) => {
  const media = request.media;
  if (!media) return request;
  const protocol = media.customData && media.customData.protocol;
  if (protocol !== undefined && protocol > SUPPORTED_PROTOCOL) {
    // A newer sender than this receiver: play what it asked, as the media message alone says.
    console.warn("sender speaks protocol", protocol, "; this receiver speaks", SUPPORTED_PROTOCOL);
  }
  // A growing transcode is still a film, not a live stream: seeking and the timeline must work.
  media.streamType = messages.StreamType.BUFFERED;
  return request;
});

const options = new cast.framework.CastReceiverOptions();
options.playbackConfig = new cast.framework.PlaybackConfig();
// A transcode starts a little behind the sender's request, so give it room before it pauses.
options.playbackConfig.autoResumeDuration = 5;
options.disableIdleTimeout = false;
options.maxInactivity = 3600;
context.start(options);
