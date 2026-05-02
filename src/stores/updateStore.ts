/**
 * 14.A: store состояния auto-updater'а.
 *
 * State machine:
 *   idle ──checkForUpdates()──▶ checking
 *   checking ─update найден─▶ available
 *   checking ─нет update'а──▶ idle
 *   available ─юзер нажал «обновить»─▶ downloading (progress)
 *   downloading ─успех──▶ installed (relaunch автоматом)
 *   any ─ошибка──▶ error → idle через 5с
 *
 * `dismissed` — в settings.dismissedUpdateVersions, не здесь.
 * `lastCheckAt` — для cooldown'а между проверками.
 */

import { create } from "zustand";
import { AvailableUpdate } from "../lib/updater";

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "available"; update: AvailableUpdate }
  | { kind: "downloading"; update: AvailableUpdate; progress: number }
  | { kind: "installed" }
  | { kind: "error"; message: string };

type Store = {
  state: UpdateState;
  /** Когда была последняя проверка (unix-ms). 0 = ни разу. */
  lastCheckAt: number;
  setState: (s: UpdateState) => void;
  setLastCheckAt: (t: number) => void;
};

export const useUpdateStore = create<Store>((set) => ({
  state: { kind: "idle" },
  lastCheckAt: 0,
  setState: (state) => set({ state }),
  setLastCheckAt: (lastCheckAt) => set({ lastCheckAt }),
}));
