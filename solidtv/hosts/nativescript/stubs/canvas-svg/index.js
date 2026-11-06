// SPDX-License-Identifier: AGPL-3.0-only
//
// @nativescript/canvas-svg's xcframework has no tvOS slice and fails the build. The
// polyfill imports these names at load; icons are a sprite sheet here, so nothing
// renders SVG, and each is an empty class.
export class Circle {}
export class Ellipse {}
export class G {}
export class Image {}
export class Line {}
export class LinearGradient {}
export class Path {}
export class Polygon {}
export class Polyline {}
export class RadialGradient {}
export class Rect {}
export class Stop {}
export class Svg {
  static fromSrcSync() {
    return null;
  }
  static fromSrc() {
    return Promise.resolve(null);
  }
}
export class Text {}
export class Use {}
