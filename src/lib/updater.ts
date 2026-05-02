/**
 * 14.A: обёртка над `@tauri-apps/plugin-updater` + `plugin-process`.
 *
 * Endpoint и pubkey прописаны в `tauri.conf.json` (тот же ключ что в CI
 * подписывает релизы). При вызове `check()` плагин сам ходит в endpoint,
 * парсит `latest.json` и проверяет ed25519-подпись `.sig` файлов NSIS-
 * installer'а. Если хоть что-то не сходится — `null` (или throw в случае
 * сетевой ошибки), мы это просто логируем без громких ошибок.
 */

import { check, Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface AvailableUpdate {
  /** Версия из manifest'а (например "0.1.4"). */
  version: string;
  /** Текущая версия приложения. */
  currentVersion: string;
  /** Release notes (из тела GitHub Release / поля `notes` manifest'а). */
  notes: string;
  /** ISO-дата релиза (если есть). */
  date: string | null;
  /** Внутренний хэндл плагина для последующего downloadAndInstall. */
  handle: Update;
}

/**
 * Проверка обновлений. Возвращает `null` если уже на последней версии
 * или произошла сетевая ошибка (мы не пугаем юзера notwerk-ошибками).
 */
export async function checkForUpdates(): Promise<AvailableUpdate | null> {
  try {
    const update = await check();
    if (!update) return null;
    return {
      version: update.version,
      currentVersion: update.currentVersion,
      notes: update.body ?? "",
      date: update.date ?? null,
      handle: update,
    };
  } catch (e) {
    // Не показываем юзеру каждый network-fail. Логируем для диагностики.
    console.warn("[updater] check failed:", e);
    return null;
  }
}

/**
 * Скачивает и устанавливает обновление. После успешной установки
 * автоматически перезапускает приложение через `plugin-process`.
 *
 * `onProgress` зовётся после каждого chunk'а с прогрессом 0..1
 * (downloaded / contentLength). NSIS у нас обычно ~44 МБ.
 */
export async function downloadAndInstall(
  update: AvailableUpdate,
  onProgress?: (fraction: number) => void,
): Promise<void> {
  let downloaded = 0;
  let total = 0;

  await update.handle.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? 0;
        onProgress?.(0);
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        if (total > 0) {
          onProgress?.(Math.min(1, downloaded / total));
        }
        break;
      case "Finished":
        onProgress?.(1);
        break;
    }
  });

  // installMode=passive в tauri.conf.json — NSIS запускается с минимумом
  // UI и сам перезапускает app, но Tauri рекомендует звать relaunch()
  // на случай если NSIS не успел перехватить.
  await relaunch();
}
