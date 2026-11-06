// SPDX-License-Identifier: AGPL-3.0-only
//
// Design tokens, ported from web/src/app.css. The renderer takes 0xRRGGBBAA, so
// the web's rgb(… / a) values are spelled out here, as flutter/lib/theme.dart does.
// Type and spacing are the web's scaled for a screen ten feet away.

export const color = {
  bg: 0x07080bff,
  bgRaised: 0x0c0e12ff,
  bgOverlay: 0x090a0ec7,
  surface: 0xffffff09,
  surface2: 0xffffff0f,
  surface3: 0xffffff1a,
  line: 0xffffff13,
  lineStrong: 0xffffff24,
  text: 0xf4f5f1ff,
  text2: 0xf4f5f1b3,
  text3: 0xf4f5f175,
  accent: 0xd4ff5aff,
  accentSoft: 0xd4ff5a24,
  accentInk: 0x10150aff,
  danger: 0xff6b6bff,
  dangerSoft: 0xff6b6b1f,
  warning: 0xffc164ff,
  success: 0x6ee7a8ff,
  focus: 0xa5dcffff,
  light: 0xf4f5f1ff,
  lightInk: 0x0b0c0fff,
  live: 0xff4d5eff,
  black: 0x000000ff,
  clear: 0x00000000,
} as const;

/** The default page tint, until artwork gives it another. */
export const AMBIENT = 0x5a688cff;

export const radius = { xs: 8, s: 12, m: 16, l: 22, xl: 30, pill: 999 } as const;

/** The smallest size that reads across a room is `caption`. */
export const type = { caption: 24, body: 28, lead: 32, title: 40, heading: 56, display: 84 } as const;

export const weight = { regular: 400, semibold: 600, bold: 700 } as const;

// `--ease-spring` is a linear() curve the renderer can't take; this overshoots the same way.
export const ease = { out: "cubic-bezier(0.16, 1, 0.3, 1)", inOut: "cubic-bezier(0.65, 0, 0.35, 1)", spring: "cubic-bezier(0.34, 1.56, 0.64, 1)" } as const;
export const duration = { fast: 140, base: 240, slow: 520 } as const;

export const motion = {
  out: { duration: duration.base, easing: ease.out },
  spring: { duration: duration.slow, easing: ease.spring },
  fast: { duration: duration.fast, easing: ease.out },
} as const;

export const SCREEN = { width: 1920, height: 1080 } as const;
/** Width of the collapsed navigation, which the page sits beside. */
export const NAV_WIDTH = 112;
export const NAV_OPEN_WIDTH = 400;
/** Title-safe: TVs may overscan, so nothing that matters goes nearer the edge. */
export const GUTTER = 64;
/** The nav's fade past its own width, so what scrolls under it dissolves rather than cuts off. */
export const NAV_FADE = 28;
/** Clear of the fade by more than a focused card grows. */
export const PAGE_X = NAV_WIDTH + NAV_FADE + 20;
export const PAGE_WIDTH = SCREEN.width - PAGE_X;

/** Which way a gradient's colors run, first to last. */
export const toward = { down: 0, up: Math.PI, right: Math.PI / 2 } as const;

/** A linear gradient's shader props, its colors evenly spread unless given stops. */
export function gradient(colors: number[], angle: number, stops?: number[]) {
  return { colors, angle, stops: stops ?? colors.map((_, index) => index / (colors.length - 1)) };
}
