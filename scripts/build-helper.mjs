// scripts/build-helper.mjs
//
// Обёртка над `cargo build --bin nemefisto-helper` для npm `predev` хука.
// Запускается автоматически перед `npm run dev` (и через него — `tauri dev`).
//
// Главная проблема которую решает скрипт:
// helper установлен как Windows-сервис от SYSTEM, exe-файл захвачен
// процессом сервиса и `cargo build` падает с "os error 5: Отказано в доступе"
// при попытке перезаписать.
//
// Алгоритм:
//   1. cargo build → если успешно, готово.
//   2. Если ошибка содержит «os error 5» / «отказано в доступе» —
//      ВЫЗЫВАЕМ `sc stop NemefistoHelper` через PowerShell `-Verb RunAs`
//      (1 UAC-промпт). Старый сервис останавливается, файл освобождается.
//   3. Повторяем cargo build — теперь должна пройти.
//   4. Tauri-main при connect увидит, что helper не отвечает, поднимет
//      его через тот же UAC-install — но уже с НОВЫМ бинарём.
//
// При ЛЮБОЙ другой ошибке (compile error и т.п.) — exit с её кодом, dev
// не запустится — это правильно.

import { spawnSync } from "node:child_process";

function buildHelper() {
  return spawnSync(
    "cargo",
    ["build", "--manifest-path", "src-tauri/Cargo.toml", "--bin", "nemefisto-helper"],
    {
      stdio: ["inherit", "inherit", "pipe"],
      shell: true,
      encoding: "utf8",
    },
  );
}

function isFileLocked(stderr) {
  return (
    /os error 5/i.test(stderr) ||
    /access is denied/i.test(stderr) ||
    /отказано в доступе/i.test(stderr)
  );
}

/** Попытаться остановить helper-сервис через UAC-elevated `sc stop`.
 *  Возвращает true если sc вернул 0 (сервис остановлен или не существовал),
 *  false если пользователь отказал в UAC или sc упал. */
function stopHelperServiceElevated() {
  console.log(
    "[predev] пытаюсь остановить NemefistoHelper-сервис (потребуется UAC)…",
  );
  // Start-Process -Verb RunAs триггерит UAC. -Wait чтобы дождаться завершения.
  // -PassThru возвращает Process чтобы получить exit code. WindowStyle Hidden
  // прячет окно sc.exe от пользователя.
  const psCommand = `
    try {
      $p = Start-Process sc.exe -ArgumentList 'stop','NemefistoHelper' \
        -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ErrorAction Stop
      exit $p.ExitCode
    } catch {
      exit 1
    }
  `;
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-Command", psCommand],
    { stdio: ["ignore", "pipe", "pipe"], encoding: "utf8" },
  );
  // sc stop коды: 0 = успех, 1062 = не запущен (тоже ок для нас),
  // 1060 = не существует (тоже ок).
  const ok = [0, 1060, 1062].includes(result.status);
  if (ok) {
    console.log("[predev] сервис остановлен (или не был запущен).");
  } else {
    console.log(
      `[predev] sc stop вернул код ${result.status}; вероятно UAC отменён.`,
    );
  }
  return ok;
}

/** Подождать пока Windows реально освободит файл-хендл. После `sc stop`
 *  процесс завершается асинхронно — кратенький delay избегает гонки. */
function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

let result = buildHelper();
let stderr = result.stderr ?? "";
process.stderr.write(stderr);

if (result.status === 0) {
  process.exit(0);
}

if (isFileLocked(stderr)) {
  console.log(
    "\n[predev] nemefisto-helper.exe заблокирован запущенным сервисом.",
  );

  if (stopHelperServiceElevated()) {
    sleepSync(800);
    console.log("[predev] повторная сборка helper'а…");
    result = buildHelper();
    stderr = result.stderr ?? "";
    process.stderr.write(stderr);
    if (result.status === 0) {
      console.log("[predev] helper пересобран. Tauri-main поднимет его через UAC.");
      process.exit(0);
    }
  }

  console.log(
    "[predev] не удалось пересобрать helper. Останови сервис вручную:\n" +
      "         (admin) sc stop NemefistoHelper\n" +
      "[predev] Frontend dev продолжит со старым helper-ом — TUN-режим может\n" +
      "         не работать корректно для mihomo-passthrough подписок.\n",
  );
  process.exit(0);
}

// Реальная ошибка сборки — пробрасываем код, чтобы dev не запустился
// со сломанным helper-binary.
process.exit(result.status ?? 1);
