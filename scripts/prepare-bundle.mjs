// scripts/prepare-bundle.mjs
//
// Prebuild-хук для `tauri build`. Делает три вещи:
//
//  1. Собирает `nemefisto-helper.exe` в release-режиме.
//  2. Копирует получившийся `target/release/nemefisto-helper.exe` в
//     `src-tauri/binaries/nemefisto-helper-<triplet>.exe` — Tauri ожидает
//     externalBin с triplet-суффиксом.
//  3. Проверяет, что все остальные sidecar (xray, tun2socks) и ресурсы
//     (wintun.dll, geoip.dat, geosite.dat) на месте — иначе bundle не
//     соберётся.
//
// Запускается автоматически через npm-скрипт `tauri:bundle` (см. package.json).
//
// Если `nemefisto-helper.exe` заблокирован запущенным сервисом —
// печатаем понятную инструкцию и выходим с ошибкой (для release-сборки
// это критично, в отличие от dev).

import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..");
const SRC_TAURI = join(ROOT, "src-tauri");
const BINARIES = join(SRC_TAURI, "binaries");
const TARGET_RELEASE = join(SRC_TAURI, "target", "release");

// Тот же triplet что и для остальных sidecar (xray, tun2socks). Если
// добавится поддержка ARM64 или Linux — расширим определение.
const TRIPLET = "x86_64-pc-windows-msvc";

const REQUIRED_RESOURCES = [
  "xray-x86_64-pc-windows-msvc.exe",
  "mihomo-x86_64-pc-windows-msvc.exe",
  "tun2socks-x86_64-pc-windows-msvc.exe",
  "wintun.dll",
  "geoip.dat",
  "geosite.dat",
];

function fail(msg) {
  console.error(`\n[prepare-bundle] ОШИБКА: ${msg}\n`);
  process.exit(1);
}

function info(msg) {
  console.log(`[prepare-bundle] ${msg}`);
}

// ── 0. Курица-яйцо с tauri-build ──────────────────────────────────────
// `tauri-build` (build.rs пакета `vpn-client`) запускается перед компиляцией
// ЛЮБОГО бинаря пакета и валидирует существование всех externalBin путей.
// Если файла helper-а ещё нет, шаг 1 (cargo build --bin nemefisto-helper)
// упадёт. Поэтому создаём placeholder если файла нет — это устраивает
// build.rs, а на шаге 2 placeholder перезаписывается реальным бинарём.
const targetHelperPath = join(
  BINARIES,
  `nemefisto-helper-${TRIPLET}.exe`
);
if (!existsSync(BINARIES)) {
  mkdirSync(BINARIES, { recursive: true });
}
if (!existsSync(targetHelperPath)) {
  info("создаю placeholder для helper.exe (нужен tauri-build)");
  writeFileSync(targetHelperPath, Buffer.alloc(0));
}

// ── 1. Собираем helper в release ──────────────────────────────────────
info("компилирую nemefisto-helper.exe в release...");
const buildResult = spawnSync(
  "cargo",
  [
    "build",
    "--manifest-path",
    join(SRC_TAURI, "Cargo.toml"),
    "--bin",
    "nemefisto-helper",
    "--release",
  ],
  {
    stdio: ["inherit", "inherit", "pipe"],
    shell: true,
    encoding: "utf8",
  }
);

const stderr = buildResult.stderr ?? "";
process.stderr.write(stderr);

if (buildResult.status !== 0) {
  // Файл занят запущенным сервисом
  if (
    /os error 5/i.test(stderr) ||
    /access is denied/i.test(stderr) ||
    /отказано в доступе/i.test(stderr)
  ) {
    fail(
      "nemefisto-helper.exe в target/release/ заблокирован запущенным сервисом.\n" +
        "         Останови сервис админом перед сборкой:\n" +
        "           sc stop NemefistoHelper\n" +
        "           src-tauri\\target\\release\\nemefisto-helper.exe uninstall"
    );
  }
  fail(`cargo build завершился с кодом ${buildResult.status}`);
}

// ── 2. Копируем helper в binaries/ с triplet-суффиксом ────────────────
const sourceHelper = join(TARGET_RELEASE, "nemefisto-helper.exe");

if (!existsSync(sourceHelper)) {
  fail(`не найден ${sourceHelper} после cargo build (странно)`);
}

try {
  copyFileSync(sourceHelper, targetHelperPath);
  const size = (statSync(targetHelperPath).size / 1024 / 1024).toFixed(1);
  info(
    `helper скопирован → binaries/nemefisto-helper-${TRIPLET}.exe (${size} МБ)`
  );
} catch (e) {
  fail(`не удалось скопировать helper: ${e.message}`);
}

// ── 2b. Дублируем под именем `nemefisto_helper.exe` для tauri-bundler ─
// Tauri 2 при сборке installer'а конвертирует имена `[[bin]]` из
// kebab-case (`nemefisto-helper`) в snake_case (`nemefisto_helper.exe`)
// и ищет файл по этому имени в `target/<profile>/`. Cargo же создаёт
// файл строго по `[[bin]] name` (с дефисом). Чтобы не переименовывать
// bin (имя зашито в helper_bootstrap, service.rs, prepare-bundle и др.),
// просто кладём ещё одну копию рядом — это удовлетворяет bundler.
const sourceHelperUnderscored = join(TARGET_RELEASE, "nemefisto_helper.exe");
try {
  copyFileSync(sourceHelper, sourceHelperUnderscored);
  info(
    `helper дублирован → target/release/nemefisto_helper.exe (для tauri-bundler)`
  );
} catch (e) {
  fail(`не удалось дублировать helper для bundler: ${e.message}`);
}

// ── 3. Проверка остальных файлов ──────────────────────────────────────
const missing = REQUIRED_RESOURCES.filter(
  (f) => !existsSync(join(BINARIES, f))
);
if (missing.length > 0) {
  fail(
    `в src-tauri/binaries/ отсутствуют файлы: ${missing.join(", ")}.\n` +
      "         Скачай их вручную (xray-core, tun2socks, wintun.dll, geoip.dat,\n" +
      "         geosite.dat) и положи в binaries/ перед release-сборкой."
  );
}

info("готов к bundle: все sidecar и ресурсы на месте.");
