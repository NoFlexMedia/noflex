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
import { app, destinations } from "$lib/session.js";
import { color, gradient, motion, NAV_FADE, NAV_OPEN_WIDTH, NAV_WIDTH, radius, SCREEN, toward, type, weight } from "../theme.js";
import BrandMark from "$components/BrandMark.js";
import Icon from "$components/Icon.js";
import ServerStatus from "$components/ServerStatus.js";

interface Entry { href: string; label: string; icon: IconName }

const LIBRARY_ICON: Record<string, IconName> = Object.fromEntries(CATEGORIES.map((category) => [category.value, category.icon]));

const ITEM = 72;
const SOLID = NAV_WIDTH / (NAV_WIDTH + NAV_FADE);
// Eased at both ends: a straight ramp from solid draws a line where it starts.
const FADE_COLORS = [color.bg, color.bg, 0x07080bb8, 0x07080b48, 0x07080b00];
const FADE_STOPS = [0, SOLID, SOLID + (1 - SOLID) * 0.3, SOLID + (1 - SOLID) * 0.7, 1];

function isActive(pathname: string, href: string): boolean {
  return href === "/" ? pathname === "/" : pathname.indexOf(href) === 0;
}

function NavItem(props: NodeProps & { entry: Entry; open: Accessor<boolean>; current: boolean }) {
  const [focused, setFocused] = createSignal(false);
  const ink = () => (focused() ? color.lightInk : props.current ? color.accent : color.text2);
  return (
    <view
      {...props}
      width={NAV_OPEN_WIDTH - 40}
      height={ITEM}
      color={focused() ? color.light : color.clear}
      borderRadius={radius.m}
      transition={{ color: motion.fast }}
      onFocusChanged={setFocused}
    >
      <Icon name={props.entry.icon} size={36} x={18} y={18} color={ink()} />
      <text x={76} height={ITEM} lineHeight={ITEM} fontSize={type.caption} fontWeight={weight.semibold} color={ink()} alpha={props.open() ? 1 : 0} transition={{ alpha: motion.fast }}>
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
  ]);

  const pick = (href: string) => {
    if (location.pathname === href) props.onPicked();
    else navigate(href);
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
      <Column x={20} y={132} width={NAV_OPEN_WIDTH - 40} height={SCREEN.height - 240} gap={6} scroll="none">
        <For each={entries()}>
          {(entry) => <NavItem entry={entry} open={open} current={isActive(location.pathname, entry.href)} onEnter={() => pick(entry.href)} />}
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

