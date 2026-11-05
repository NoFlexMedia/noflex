// SPDX-License-Identifier: AGPL-3.0-only
//
// What libmpv's reports of position and end of file really mean. It says the
// same things of a stream that ran out as of a film that finished, and these
// stand between that and telling the server someone watched the whole film.

/// Probed lengths and what a file actually plays differ by this much, in ms.
const _endTolerance = 30000;
const _lastFrameWindow = 1500;
const _jumpTolerance = 10000;

/// A stream that ended this far short of the title's length didn't finish it.
bool endedEarly({required int lengthMs, required int reachedMs}) =>
    lengthMs - reachedMs > _endTolerance;

/// Further from [b] than ordinary playback or a seek's rounding would land.
bool isFarFrom(int a, int b) => (a - b).abs() > _jumpTolerance;

/// libmpv seeks itself to the last frame when a stream runs out with nothing on
/// screen. Nobody watched their way there, and counting it as progress would
/// mark the title watched. [streamMs] and [streamLengthMs] are the stream's own
/// clock, the rest source time.
bool isFalseEnd({
  required int streamMs,
  required int streamLengthMs,
  required int atMs,
  required int reachedMs,
  required int requestedMs,
}) =>
    streamLengthMs > 0 &&
    streamLengthMs - streamMs < _lastFrameWindow &&
    atMs - reachedMs > _jumpTolerance &&
    isFarFrom(atMs, requestedMs);
