// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex Connect from the controlling side: the viewer's other devices, which
// of them are listening, what they're playing, and the choice of where a Play
// goes. The state is whatever the server last said; a device's position is
// advanced here between its reports.

import { CastToDeviceStore, ConnectDeviceUpdatesStore, ControlDeviceStore } from "$houdini";
import type { ConnectDeviceFields$data } from "$houdini";
import { deviceId } from "$lib/device.js";
import { request } from "$lib/graphql.js";
import { attempt } from "$lib/ui.svelte.js";

/** `via` names the route that reaches a device other than through the server, such as a Cast session. */
export type RemoteDevice = ConnectDeviceFields$data & { heardAt: number; via?: string };
export type Control = "PLAY" | "PAUSE" | "SEEK" | "STOP";

/** How a device the server doesn't know of is played on and steered, so it shares the picker and the bar. */
export interface Route {
  cast(work: { id: string }, options: { editionId?: string; mediaFileId?: string; positionMs?: number }): Promise<boolean>;
  control(device: RemoteDevice, action: Control, positionMs?: number): Promise<void>;
}

const updates = new ConnectDeviceUpdatesStore();
const TARGET_KEY = "noflex.castTarget";

function remembered(): string | null {
  try { return sessionStorage.getItem(TARGET_KEY); } catch { return null; }
}

class Connect {
  /** Every device of the viewer's, this browser's own excluded. */
  devices = $state.raw<RemoteDevice[]>([]);
  /** Where Play goes; null plays here. */
  targetId = $state<string | null>(remembered());
  /** Ticks while something is playing remotely, so a position advances smoothly. */
  now = $state(Date.now());

  /** Devices reached by a route of their own, which whoever owns the route keeps current. */
  external = $state.raw<RemoteDevice[]>([]);
  routes = new Map<string, Route>();

  /** Devices that can be played on right now. */
  available = $derived([...this.devices, ...this.external].filter((item) => item.online));
  target = $derived(this.available.find((item) => item.device.id === this.targetId) ?? null);
  /** What the now-playing bar shows: the chosen device if it has something, else whichever does. */
  active = $derived(
    (this.target?.playback ? this.target : null) ?? this.available.find((item) => item.playback) ?? null,
  );

  /** Where `device` is up to, advanced since its last report while playing. */
  position(device: RemoteDevice): number {
    const playback = device.playback;
    if (!playback) return 0;
    const elapsed = playback.state === "PLAYING" ? this.now - device.heardAt : 0;
    const at = playback.positionMs + elapsed;
    return Math.max(0, playback.durationMs ? Math.min(at, playback.durationMs) : at);
  }

  setTarget(id: string | null): void {
    this.targetId = id;
    try { id ? sessionStorage.setItem(TARGET_KEY, id) : sessionStorage.removeItem(TARGET_KEY); } catch { /* remembered per tab only */ }
  }

  #apply(list: ConnectDeviceFields$data[]): void {
    const self = deviceId();
    const heardAt = Date.now();
    this.devices = list.filter((item) => item.device.id !== self).map((item) => ({ ...item, heardAt }));
    if (this.targetId && !this.target) this.setTarget(null);
  }

  #replace(fresh: ConnectDeviceFields$data): void {
    const heardAt = Date.now();
    this.devices = this.devices.map((item) => (item.device.id === fresh.device.id ? { ...fresh, heardAt } : item));
  }

  /** Follow the server's list for as long as the viewer is signed in. Returns the disposer. */
  run(): () => void {
    const unsubscribe = updates.subscribe((result) => {
      if (result.data) this.#apply(result.data.connectDevices);
    });
    void updates.listen();
    const tick = setInterval(() => { if (this.active) this.now = Date.now(); }, 500);
    return () => {
      unsubscribe();
      void updates.unlisten();
      clearInterval(tick);
      this.devices = [];
    };
  }

  /** Start `work` on the chosen device, which picks the version it plays best. */
  async cast(work: { id: string }, options: { editionId?: string; mediaFileId?: string; positionMs?: number } = {}): Promise<boolean> {
    const target = this.target;
    if (!target) return false;
    const route = target.via ? this.routes.get(target.via) : undefined;
    if (route) return route.cast(work, options);
    const sent = await attempt(() => request(new CastToDeviceStore(), {
      device: target.device.id,
      work: work.id,
      edition: options.editionId ?? null,
      media: options.mediaFileId ?? null,
      position: options.positionMs ?? null,
    }), `Playing on ${target.device.name}`);
    if (sent) this.#replace(sent.castToDevice);
    return sent !== null;
  }

  /** Steer the device that's playing. The control answers at once; the device's word follows. */
  async control(device: RemoteDevice, action: Control, positionMs?: number): Promise<void> {
    const route = device.via ? this.routes.get(device.via) : undefined;
    if (route) return route.control(device, action, positionMs);
    const playback = device.playback;
    // Stopped is stopped: the bar goes now, and the device's word that it's idle only confirms it.
    if (action === "STOP") this.devices = this.devices.map((item) => (item.device.id === device.device.id ? { ...item, playback: null } : item));
    if (playback && action !== "STOP") {
      const state = action === "PAUSE" ? "PAUSED" : action === "PLAY" ? "PLAYING" : playback.state;
      const position = action === "SEEK" && positionMs !== undefined ? positionMs : this.position(device);
      this.#replace({ ...device, playback: { ...playback, state, positionMs: position } });
    }
    const sent = await attempt(() => request(new ControlDeviceStore(), { device: device.device.id, action, position: positionMs ?? null }));
    if (sent) this.#replace(sent.controlDevice);
  }
}

export const connect = new Connect();
