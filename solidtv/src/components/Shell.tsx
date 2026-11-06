// SPDX-License-Identifier: AGPL-3.0-only
//
// The navigation and the page beside it. TVs have one Back key, doing double duty:
// from a title it goes back; from a destination it opens the navigation; from the
// navigation it goes Home; from Home's navigation it leaves the app. Left is the
// other way in, for whatever a page doesn't take itself.

import { useLocation, useNavigate } from "@solidjs/router";
import { activeElement, type ElementNode } from "@solidtv/solid";
import { createEffect, on, type ParentProps } from "solid-js";
import { ui } from "$lib/ui.js";
import { SCREEN } from "../theme.js";
import Nav from "$components/Nav.js";

export default function Shell(props: ParentProps) {
  const navigate = useNavigate();
  const location = useLocation();
  let nav: ElementNode | undefined;
  let page: ElementNode | undefined;
  let lastFocused: ElementNode | undefined;

  // Each page brings its own art, if it has any.
  createEffect(on(() => location.pathname, () => { ui.backdrop = null; }, { defer: true }));

  const inNav = () => !!nav?.states.has("$focus");
  const atDestination = () => location.pathname.indexOf("/works/") !== 0 && !/\/all$/.test(location.pathname);

  function openNav(): boolean {
    if (inNav()) return false;
    lastFocused = activeElement() as ElementNode | undefined;
    nav?.setFocus();
    return true;
  }

  function closeNav(): boolean {
    if (!inNav()) return false;
    if (lastFocused && !lastFocused.destroyed) lastFocused.setFocus();
    else page?.setFocus();
    return true;
  }

  function back(): boolean {
    if (inNav()) {
      // Unhandled, for App to leave.
      if (location.pathname === "/") return false;
      navigate("/");
      return true;
    }
    if (atDestination()) return openNav();
    navigate(-1);
    return true;
  }

  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={1} onBack={back} onLeft={openNav} onRight={closeNav}>
      <Nav ref={(node: ElementNode) => { nav = node; }} onPicked={() => page?.setFocus()} />
      <view ref={(node: ElementNode) => { page = node; }} width={SCREEN.width} height={SCREEN.height} forwardFocus={0} children={props.children} />
    </view>
  );
}
