// SPDX-License-Identifier: AGPL-3.0-only
//
// The web's sidebar, in its collapsed form until focus reaches it. The same
// destinations, less Administration and Watch together: see AGENTS.md.

import { useLocation, useNavigate } from "@solidjs/router";
import { Column } from "@solidtv/solid/primitives";
import { createMemo, createSignal, For, Show, type Accessor } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { CATEGORIES } from "$lib/categories.js";
import type { IconName } from "$lib/icons.generated.js";
import { app, destinations, switchProfile } from "$lib/session.js";
import { color, gradient, motion, NAV_FADE, NAV_OPEN_WIDTH, NAV_WIDTH, radius, SCREEN, toward, type, weight } from "../theme.js";
import BrandMark from "$components/BrandMark.js";
import Icon from "$components/Icon.js";
import ServerStatus from "$components/ServerStatus.js";

/** Somewhere to go, or with `action` something to do instead. */
interface Entry { href: string; label: string; icon: IconName; action?: () => void }

const LIBRARY_ICON: Record<string, IconName> = Object.fromEntries(CATEGORIES.map((category) => [category.value, category.icon]));

const ITEM = 72;
const ITEM_MIN = 56;
const ITEM_GAP = 6;
const LIST_HEIGHT = SCREEN.height - 240;
const SOLID = NAV_WIDTH / (NAV_WIDTH + NAV_FADE);
// Eased at both ends: a straight ramp from solid draws a line where it starts.
const FADE_COLORS = [color.bg, color.bg, 0x07080bb8, 0x07080b48, 0x07080b00];
const FADE_STOPS = [0, SOLID, SOLID + (1 - SOLID) * 0.3, SOLID + (1 - SOLID) * 0.7, 1];

function isActive(pathname: string, href: string): boolean {
  return href === "/" ? pathname === "/" : pathname.indexOf(href) === 0;
}

function NavItem(props: NodeProps & { entry: Entry; open: Accessor<boolean>; current: boolean; size: number }) {
  const [focused, setFocused] = createSignal(false);
  const ink = () => (focused() ? color.lightInk : props.current ? color.accent : color.text2);
  return (
    <view
      {...props}
      width={NAV_OPEN_WIDTH - 40}
      height={props.size}
      color={focused() ? color.light : color.clear}
      borderRadius={radius.m}
      transition={{ color: motion.fast }}
      onFocusChanged={setFocused}
    >
      <Icon name={props.entry.icon} size={36} x={18} y={(props.size - 36) / 2} color={ink()} />
      <text x={76} height={props.size} lineHeight={props.size} fontSize={type.caption} fontWeight={weight.semibold} color={ink()} alpha={props.open() ? 1 : 0} transition={{ alpha: motion.fast }}>
        {props.entry.label}
      </text>
    </view>
  );
}

export default function Nav(props: NodeProps & { onPicked: () => void }) {
  const navigate = useNavigate();
  const location = useLocation();
  const [open, setOpen] = createSignal(false);

  const entries = createMemo<Entry[]>(() => [
    { href: "/", label: "Home", icon: "house" },
    { href: "/search", label: "Search", icon: "magnifying-glass" },
    ...destinations().map((category) => ({ href: `/browse/${category.slug}`, label: category.label, icon: category.icon })),
    { href: "/collections", label: "Collections", icon: "cards-three" },
    { href: "/live-tv", label: "Live TV", icon: "television" },
    ...(app.showLibraries ? app.libraries.map((library): Entry => ({ href: `/libraries/${library.id}`, label: library.name, icon: LIBRARY_ICON[library.kind] ?? "books" })) : []),
    { href: "/account", label: app.viewer?.name ?? "Account", icon: "user" },
    // Always there, as it's also the way to add a second person to a TV with one.
    { href: "", label: "Switch profile", icon: "user-switch", action: () => void switchProfile() },
  ]);
  // Rows give up a little height before the list runs under the status dot.
  const itemSize = () => Math.max(ITEM_MIN, Math.min(ITEM, Math.floor((LIST_HEIGHT - (entries().length - 1) * ITEM_GAP) / entries().length)));

  const pick = (entry: Entry) => {
    if (entry.action) entry.action();
    else if (location.pathname === entry.href) props.onPicked();
    else navigate(entry.href);
    return true;
  };

  return (
    <view
      {...props}
      width={open() ? NAV_OPEN_WIDTH : NAV_WIDTH + NAV_FADE}
      height={SCREEN.height}
      zIndex={100}
      forwardFocus={0}
      transition={{ width: motion.out }}
      onFocusChanged={setOpen}
      linearGradient={open() ? gradient([color.bgRaised, color.bgRaised], toward.right) : gradient(FADE_COLORS, toward.right, FADE_STOPS)}
    >
      <Column x={20} y={132} width={NAV_OPEN_WIDTH - 40} height={LIST_HEIGHT} gap={ITEM_GAP} scroll="none">
        <For each={entries()}>
          {(entry) => <NavItem entry={entry} size={itemSize()} open={open} current={!entry.action && isActive(location.pathname, entry.href)} onEnter={() => pick(entry)} />}
        </For>
      </Column>
      <BrandMark x={28} y={44} size={56} />
      <Show when={open()}>
        <text x={100} y={44} height={56} lineHeight={56} fontSize={type.lead} fontWeight={weight.bold}>NoFlex</text>
      </Show>
      <ServerStatus x={20} y={SCREEN.height - 96} open={open} />
    </view>
  );
}

