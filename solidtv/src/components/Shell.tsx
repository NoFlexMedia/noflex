// SPDX-License-Identifier: AGPL-3.0-only
//
// The navigation and the page beside it. TVs have one Back key, doing double duty:
// from a title it goes back; from a destination it opens the navigation; from the
// navigation it goes Home; from Home's navigation it leaves the app. Left is the
// other way in, for whatever a page doesn't take itself.

import { useLocation, useNavigate } from "@solidjs/router";
import { activeElement, type ElementNode } from "@solidtv/solid";
import { createComputed, createEffect, on, type ParentProps } from "solid-js";
import { ui } from "$lib/ui.js";
import { SCREEN } from "../theme.js";
import Nav from "$components/Nav.js";

export default function Shell(props: ParentProps) {
  const navigate = useNavigate();
  const location = useLocation();
  let nav: ElementNode | undefined;
  let page: ElementNode | undefined;
  let lastFocused: ElementNode | undefined;

  // A kept-alive page leaves `page.children` while hidden but keeps its `parent`.
  function within(element: ElementNode): "showing" | "hidden" | "outside" {
    let node: ElementNode | undefined = element;
    while (node && node.parent !== page) node = node.parent;
    if (!node) return "outside";
    return page!.children.indexOf(node) >= 0 ? "showing" : "hidden";
  }

  function focusPage() {
    if (lastFocused && !lastFocused.destroyed && within(lastFocused) === "showing") lastFocused.setFocus();
    else page?.setFocus();
  }

  // Tracked here rather than when the nav opens, since a pointer reaches the nav without
  // asking. And a hidden page whose data has just arrived autofocuses like any other,
  // which would leave the keys working somewhere nobody can see.
  createEffect(on(activeElement, (element) => {
    if (!element || element.destroyed) return;
    const place = within(element as ElementNode);
    if (place === "showing") lastFocused = element as ElementNode;
    else if (place === "hidden") focusPage();
  }));

  // Each page brings its own art, if it has any. Ahead of effects, or this would
  // clear what a page with its data already cached has just set.
  createComputed(on(() => location.pathname, () => { ui.backdrop = null; }, { defer: true }));

  const inNav = () => !!nav?.states.has("$focus");
  const atDestination = () => location.pathname.indexOf("/works/") !== 0 && !/\/all$/.test(location.pathname);

  function openNav(): boolean {
    if (inNav()) return false;
    nav?.setFocus();
    return true;
  }

  function closeNav(): boolean {
    if (!inNav()) return false;
    focusPage();
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
