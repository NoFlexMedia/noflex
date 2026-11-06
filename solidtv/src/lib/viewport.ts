// SPDX-License-Identifier: AGPL-3.0-only
//
// The 1080p design space, fitted whole inside the window. A TV is 16:9 and fills it.
// The canvas still takes the whole window, with the space centered by the app's root
// node rather than by moving the canvas: useMouse reads window coordinates.

import { SCREEN } from "../theme.js";

const windowed = __TARGET__ !== "nativescript" && typeof window !== "undefined" && window.innerWidth > 0 && window.innerHeight > 0;
// The NativeScript hosts size their own canvas, and draw the 16:9 space at its left.
const height = typeof window !== "undefined" && window.innerHeight ? window.innerHeight : SCREEN.height;
const width = windowed ? window.innerWidth : Infinity;

/** Window pixels per design pixel. */
export const scale = Math.min(width / SCREEN.width, height / SCREEN.height);
export const appWidth = windowed ? width / scale : SCREEN.width;
export const appHeight = windowed ? height / scale : SCREEN.height;
export const originX = (appWidth - SCREEN.width) / 2;
export const originY = (appHeight - SCREEN.height) / 2;
