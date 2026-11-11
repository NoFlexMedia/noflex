// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex's Cast receiver. It plays what a sender loads and nothing else: it makes
// no requests to a NoFlex server, so it works whatever the server's address or
// scheme, and nothing it is sent can be mistaken for a credential. The sender
// (web/src/lib/cast.svelte.ts) owns the session, resumes from the viewer's place
// and reports progress. `customData.protocol` says which dialect a sender speaks.

const SUPPORTED_PROTOCOL = 1;
/** Where a sender asks this device what it decodes. */
const NAMESPACE = "urn:x-cast:dev.noflex.cast";

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

function capabilities() {
  return context.getDeviceCapabilities() || {};
}

/**
 * One of the sender's questions (`MediaQuestion` in lib/ts/codecSupport.ts):
 * the sender runs the web client's own probe and only the answers come from here.
 */
function answer(question) {
  const caps = capabilities();
  const codecs = question.codecs || "";
  if (question.hdr === "pq-dynamic") return false;
  if (question.hdr && caps.is_hdr_supported === false) return false;
  if (/^(dvh1|dvhe|dav1|dvav)\./.test(codecs) && caps.is_dv_supported === false) return false;
  // canDisplayType has no channel count: Dolby is passed to the TV whole, the rest decode to 5.1 at most.
  if (question.channels && question.channels > (codecs === "ec-3" ? 8 : 6)) return false;
  try {
    return context.canDisplayType(question.mime, codecs || undefined, question.width, question.height, question.framerate);
  } catch {
    return false;
  }
}

context.addCustomMessageListener(NAMESPACE, (event) => {
  const message = event.data || {};
  if (message.type === "device") {
    const caps = capabilities();
    context.sendCustomMessage(NAMESPACE, event.senderId, {
      type: "device",
      id: message.id,
      hdr: caps.is_hdr_supported === true,
      dolbyVision: caps.is_dv_supported === true,
      touch: caps.touch_input_supported === true,
    });
  } else if (message.type === "probe") {
    const questions = Array.isArray(message.questions) ? message.questions : [];
    context.sendCustomMessage(NAMESPACE, event.senderId, { type: "probe", id: message.id, answers: questions.map(answer) });
  }
});

const options = new cast.framework.CastReceiverOptions();
options.customNamespaces = { [NAMESPACE]: cast.framework.system.MessageType.JSON };
options.playbackConfig = new cast.framework.PlaybackConfig();
// A transcode starts a little behind the sender's request, so give it room before it pauses.
options.playbackConfig.autoResumeDuration = 5;
options.disableIdleTimeout = false;
options.maxInactivity = 3600;
context.start(options);
