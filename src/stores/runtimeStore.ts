import { create } from "zustand";

/**
 * Эфемерный runtime-стейт, не персистится в localStorage.
 * Сюда попадают значения, актуальные только в рамках текущей сессии:
 * - `currentSsid` — имя Wi-Fi сети к которой подключены сейчас (13.M);
 * - `autoDisconnectedBySsid` — флаг что VPN отключили мы сами по
 *   правилу trusted-сети, а не пользователь. Нужен чтобы при выходе
 *   из доверенной сети не переподключаться против воли пользователя
 *   (если он сам выключил — оставляем выключенным).
 */
type RuntimeState = {
  currentSsid: string | null;
  autoDisconnectedBySsid: boolean;
  setCurrentSsid: (s: string | null) => void;
  setAutoDisconnectedBySsid: (v: boolean) => void;
};

export const useRuntimeStore = create<RuntimeState>((set) => ({
  currentSsid: null,
  autoDisconnectedBySsid: false,
  setCurrentSsid: (s) => set({ currentSsid: s }),
  setAutoDisconnectedBySsid: (v) => set({ autoDisconnectedBySsid: v }),
}));
