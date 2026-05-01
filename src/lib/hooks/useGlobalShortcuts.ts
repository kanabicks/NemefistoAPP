import { useEffect, useRef } from "react";
import {
  isRegistered,
  register,
  unregister,
} from "@tauri-apps/plugin-global-shortcut";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSettingsStore } from "../../stores/settingsStore";
import { useVpnStore } from "../../stores/vpnStore";

/**
 * Регистрация глобальных горячих клавиш (этап 13.N).
 *
 * Действия:
 * - **toggle VPN** — connect / disconnect; в `starting`/`stopping` —
 *   игнор клик чтобы не дёрнуть параллельно две операции.
 * - **show/hide** — toggle visibility главного окна (как клик по трею).
 * - **switch mode** — переключение proxy ↔ TUN. Заблокировано пока
 *   VPN активен (как и в UI).
 *
 * Ре-регистрация происходит при изменении любого из accelerator-полей
 * settings: старая комбинация снимается, новая регистрируется. На
 * unmount всё снимается.
 *
 * Хук вызывается **только из главного окна** — глобальные хоткеи
 * процесс-уровня, регистрация во floating-окне продублирует обработчик
 * и каждый клик сработает дважды.
 */
export function useGlobalShortcuts() {
  const toggleVpn = useSettingsStore((s) => s.shortcutToggleVpn);
  const showHide = useSettingsStore((s) => s.shortcutShowHide);
  const switchMode = useSettingsStore((s) => s.shortcutSwitchMode);

  // Храним последние зарегистрированные значения чтобы корректно
  // снять их при изменении (новые значения уже в state, старые
  // нужно где-то держать).
  const registeredRef = useRef<{
    toggleVpn: string | null;
    showHide: string | null;
    switchMode: string | null;
  }>({ toggleVpn: null, showHide: null, switchMode: null });

  useEffect(() => {
    let cancelled = false;

    const safeUnregister = async (accel: string | null) => {
      if (!accel) return;
      try {
        if (await isRegistered(accel)) {
          await unregister(accel);
        }
      } catch (e) {
        console.warn("[shortcuts] unregister failed:", accel, e);
      }
    };

    const safeRegister = async (
      accel: string | null,
      handler: () => void
    ) => {
      if (!accel) return;
      try {
        if (await isRegistered(accel)) {
          await unregister(accel);
        }
        await register(accel, (event) => {
          // Tauri 2 шлёт два события: Pressed и Released. Реагируем
          // только на Pressed чтобы не сработать дважды.
          if (event.state === "Pressed") handler();
        });
      } catch (e) {
        console.warn("[shortcuts] register failed:", accel, e);
      }
    };

    const onToggleVpn = () => {
      const v = useVpnStore.getState();
      if (v.status === "running") {
        void v.disconnect();
      } else if (v.status === "stopped" || v.status === "error") {
        if (v.selectedIndex !== null) void v.connect();
      }
    };

    const onShowHide = async () => {
      const win = getCurrentWindow();
      try {
        const visible = await win.isVisible();
        if (visible) {
          await win.hide();
        } else {
          await win.show();
          await win.unminimize();
          await win.setFocus();
        }
      } catch (e) {
        console.warn("[shortcuts] show/hide failed:", e);
      }
    };

    const onSwitchMode = () => {
      const v = useVpnStore.getState();
      // Не переключаем посреди активной сессии — нужно сначала
      // отключиться, иначе старый туннель остаётся висеть.
      if (v.status !== "stopped" && v.status !== "error") return;
      // 13.R: в strict-режиме у пользователя только TUN — никаких
      // переключений не делаем, иначе глобальный шорткат бы возвращал
      // proxy и обходил настройку.
      if (useSettingsStore.getState().tunOnlyStrict) return;
      v.setMode(v.mode === "proxy" ? "tun" : "proxy");
    };

    (async () => {
      // Снимаем предыдущие
      const prev = registeredRef.current;
      await safeUnregister(prev.toggleVpn);
      await safeUnregister(prev.showHide);
      await safeUnregister(prev.switchMode);
      if (cancelled) return;

      // Регистрируем новые
      await safeRegister(toggleVpn, onToggleVpn);
      await safeRegister(showHide, onShowHide);
      await safeRegister(switchMode, onSwitchMode);
      if (cancelled) return;

      registeredRef.current = { toggleVpn, showHide, switchMode };
    })();

    return () => {
      cancelled = true;
      // Cleanup — снимаем при unmount чтобы не повисло на следующий
      // запуск. Если приложение закрывается — Tauri снимает сам, но
      // явный cleanup не вредит.
      const prev = registeredRef.current;
      void safeUnregister(prev.toggleVpn);
      void safeUnregister(prev.showHide);
      void safeUnregister(prev.switchMode);
      registeredRef.current = {
        toggleVpn: null,
        showHide: null,
        switchMode: null,
      };
    };
  }, [toggleVpn, showHide, switchMode]);
}
