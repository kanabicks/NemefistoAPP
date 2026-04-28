// scripts/build-helper.mjs
//
// Обёртка над `cargo build --bin nemefisto-helper` для npm `predev` хука.
// Запускается автоматически перед `npm run dev` (и через него — `tauri dev`).
//
// Главная проблема которую решает скрипт:
// helper установлен как Windows-сервис от SYSTEM, exe-файл захвачен
// процессом сервиса и `cargo build` падает с "os error 5: Отказано в доступе"
// при попытке перезаписать. Это не реальная ошибка сборки — просто файл занят
// уже работающим сервисом. В таком случае мы оставляем существующий
// nemefisto-helper.exe и продолжаем dev (frontend стартует, helper уже
// работает — пусть и со старой версией).
//
// При ЛЮБОЙ другой ошибке (compile error и т.п.) — exit с её кодом, dev
// не запустится — это правильно.
//
// Если ты только что правил helper-код и хочешь чтобы изменения применились,
// останови сервис вручную в админ-PowerShell:
//   .\src-tauri\target\debug\nemefisto-helper.exe uninstall
// или просто `sc stop NemefistoHelper`.

import { spawnSync } from "node:child_process";

const result = spawnSync(
  "cargo",
  ["build", "--manifest-path", "src-tauri/Cargo.toml", "--bin", "nemefisto-helper"],
  {
    stdio: ["inherit", "inherit", "pipe"],
    shell: true,
    encoding: "utf8",
  },
);

const stderr = result.stderr ?? "";
process.stderr.write(stderr);

if (result.status === 0) {
  process.exit(0);
}

// Проверяем сообщение об ошибке на маркеры «файл занят» в обеих локалях
// (Windows может выдавать русское сообщение).
const fileLocked =
  /os error 5/i.test(stderr) ||
  /access is denied/i.test(stderr) ||
  /отказано в доступе/i.test(stderr);

if (fileLocked) {
  console.log(
    "\n[predev] nemefisto-helper.exe заблокирован запущенным сервисом — пропускаю пересборку.",
  );
  console.log(
    "[predev] Если правил helper-код, останови сервис админом: \n" +
      "         .\\src-tauri\\target\\debug\\nemefisto-helper.exe uninstall",
  );
  console.log("[predev] Frontend dev продолжит работу с уже установленным helper-ом.\n");
  process.exit(0);
}

// Реальная ошибка сборки — пробрасываем код, чтобы dev не запустился
// со сломанным helper-binary.
process.exit(result.status ?? 1);
