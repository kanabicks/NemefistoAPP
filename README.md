# nemefisto

> приватный VPN-клиент под Windows на двух ядрах (sing-box и Mihomo)
> с защитой от DPI, утечек и локального детекта.
> ноль телеметрии · открытый код · auto-update · одна кнопка.

«VPN одной кнопкой»: подключение за ~1.5 секунды, минимум вопросов
к пользователю, максимум совместимости с современными протоколами
обхода блокировок. Архитектура изначально готова к портированию на
macOS, iOS и Android — UI отделён от системного слоя.

[![release](https://img.shields.io/github/v/release/kanabicks/NemefistoAPP?include_prereleases&label=release)](https://github.com/kanabicks/NemefistoAPP/releases)
[![tauri](https://img.shields.io/badge/tauri-2-blue)](https://v2.tauri.app/)
[![sing-box](https://img.shields.io/badge/sing--box-1.13-brightgreen)](https://github.com/SagerNet/sing-box)
[![mihomo](https://img.shields.io/badge/mihomo-1.19-orange)](https://github.com/MetaCubeX/mihomo)
[![license: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

---

## Скачать

Свежий релиз — на странице [Releases](https://github.com/kanabicks/NemefistoAPP/releases).
Скачай `Nemefisto_<version>_x64-setup.exe`, запусти, дальше installer
сделает всё сам.

> **SmartScreen ругается «Unknown publisher»** — это нормально, мы пока
> без EV code-signing сертификата ($80–500/год). Жми «More info» → «Run
> anyway». Установленный клиент сам обновляется на следующие версии.

После первой установки **обновления приходят автоматически**:
проверка раз в 6 часов, при найденной новой версии — модалка
«доступна v X.Y.Z [release notes →]». Подписи NSIS проверяются через
Tauri ed25519 (защита от MITM подмены installer'а).

---

## Что умеет

### VPN-движки
**Можно переключаться без переустановки**, выбор в Settings → движок:

- **sing-box 1.13** (default) — быстрый старт (~1.4с), built-in TUN
  через WinTUN с auto-route, нативный anti-DPI (`tls.fragment` +
  DoH-bootstrap server-resolve). Поддерживает: vless+REALITY/Vision,
  vmess, trojan, ss, hysteria2, **TUIC**, wireguard.
- **Mihomo (Clash Meta) 1.19** — нужен для **AnyTLS**, **Mieru**,
  **XHTTP** transport, и **per-process routing** через нативный
  `PROCESS-NAME` matcher.

### Поддержка панелей подписки
| Панель | Что отдаёт | Как обрабатываем |
|---|---|---|
| **Marzban / 3x-ui / x-ui** | xray-JSON конфиг | конвертируем в sing-box JSON через `convert_xray_json_to_singbox`, сохраняя routing/balancers |
| **Remnawave** | sing-box JSON напрямую | passthrough через `patch_singbox_json` — добавляем только наши mixed-inbound и SOCKS-auth |
| любая | base64 / raw список протокольных URI (vless://, vmess://, и т.п.) | universal subscription parser в `config/subscription.rs` |
| любая | полный mihomo YAML с proxy-groups | passthrough через `patch_full_yaml` |

Server-driven UX: подписка может прислать заголовки `X-Nemefisto-*`
(тема / фон / движок / маршрутизация / объявления) — клиент
автоматически применит дефолты, юзер всегда может переопределить.

### Режимы подключения
- **Системный прокси** — быстрый старт, один SOCKS5/HTTP inbound
  на loopback с **рандомизированным портом** в `[30000, 60000)` (защита
  от локального детекта VPN сторонними процессами).
- **TUN** — весь системный трафик через WinTUN-адаптер. Built-in TUN
  у обоих движков (нет сторонних tun2socks/tun2proxy).
- **LAN** — inbound доступен другим устройствам в Wi-Fi сети
  (с автогенерируемым SOCKS5 user/pass — креды показываются для
  копирования).

### Защита и приватность
- **Kill-switch** через Windows Filtering Platform (WFP) — фильтры
  на уровне ядра. **DYNAMIC session**: если процесс упал, фильтры
  снимаются автоматически (юзер не остаётся без интернета).
  + 5-уровневый watchdog от orphan-фильтров.
- **DNS leak protection** — блок весь :53/UDP+TCP кроме VPN-DNS.
- **WebRTC / DNS / IPv6 leak-test** через Cloudflare cdn-trace +
  ipwho.is + DoH whoami — авто после connect или вручную.
- **Orphan cleanup** на старте — TUN-адаптеры и half-routes от
  упавших сессий чистятся helper'ом.
- **Маскировка имени TUN** — `wlan99` / `Local Area Connection N` /
  `Ethernet N` вместо `nemefisto-<pid>` (защита от детекта VPN
  по `GetAdaptersAddresses`).
- **SOCKS5 inbound auth** для TUN/LAN-режимов (защита от чужих
  процессов которые могут пользоваться нашим SOCKS-портом).
- **Auto-update подписан** ed25519 (Tauri signing) — обновление
  не подменишь man-in-the-middle.

### Anti-DPI
- TCP-фрагментация TLS ClientHello (`tls.fragment`)
- UDP шумовые пакеты
- Server-address-resolve через DoH (минуя системный DNS)
- Hysteria2 obfs salamander
- Все опции переключаются в Settings или приходят из заголовков подписки

### UI / UX
- 🌐 **Двуязычный интерфейс** (RU / EN) с авто-детектом по
  `navigator.language` или вручную в Settings → Интерфейс → язык.
- 🎨 **5 тем** (dark / light / midnight / sunset / sand) + **5
  пресетов** (fluent / cupertino / vice / arcade / glacier). Тема
  «как в системе» автоматически меняется на dark/light по
  `prefers-color-scheme`.
- 🖼 **3D-фон** (4 сцены: crystal / tunnel / globe / particles).
- 🎯 **Drag-and-drop URL подписки** в окно — бросаешь ссылку из
  браузера, добавляется и сразу подгружается.
- ⌨ **Глобальные горячие клавиши** (`Ctrl+Shift+V` toggle, и др.).
- 🪟 **Floating window** — мини-окошко поверх всего со status-dot
  и live-скоростью ↑/↓.
- 🔌 **System tray** с быстрым connect/disconnect.
- 📡 **Bandwidth-метр** в реальном времени.
- 🛜 **SSID auto-mode** — VPN автоматически отключается в доверенных
  Wi-Fi (домашний роутер) и включается в чужих.
- 📥 **Backup настроек** через JSON-файл или deep-link.
- 🔧 **Routing-профили** geosite/geoip с авто-обновлением и
  поддержкой авто-минимальных RU-правил.
- ⚙ **Per-process routing** (Mihomo): «telegram через VPN, vk напрямую».
- 📦 **NSIS auto-update** — приложение обновляется само в фоне
  passive-режимом с подписанной ed25519-подписью.
- 🐛 **Кнопка «сообщить о проблеме»** в Settings → about — открывает
  GitHub Issues с pre-filled окружением.

---

## Системные требования

- **Windows 10** 1909+ или **Windows 11**
- **WebView2** (ставится автоматически если нет)
- **Admin-права один раз** для установки helper-сервиса (управление
  WinTUN и WFP). После установки helper работает как SYSTEM-service,
  app сам — как обычный пользователь.

---

## Сборка из исходников

```powershell
# Требуется Node.js 22+ и Rust stable.
git clone https://github.com/kanabicks/NemefistoAPP.git
cd NemefistoAPP
npm ci
npm run tauri:bundle
# Готовый installer: src-tauri/target/release/bundle/nsis/
```

Для разработки:

```powershell
npm run tauri dev
```

Helper-binary собирается автоматически через
`scripts/build-helper.mjs` (npm-pre-script `predev`).

---

## Архитектура

```
/
├── src/                   # React 19 + TypeScript + Tailwind v4
│   ├── components/        # Welcome, ServerSelector, SettingsPage, ...
│   ├── stores/            # Zustand: vpn / subscription / settings / toast / update
│   ├── lib/               # Утилиты, deep-links, leak-test, updater
│   ├── locales/{ru,en}/   # i18n переводы (react-i18next)
│   └── i18n.ts            # i18n config
├── src-tauri/             # Rust 2021
│   ├── src/
│   │   ├── vpn/           # State machine, sing-box, mihomo, leak-test
│   │   ├── config/        # Парсинг подписок, sing-box-конфиги, geofiles, routing
│   │   ├── platform/      # Windows-специфичный код (изолирован для портирования)
│   │   ├── ipc/           # Tauri commands
│   │   └── bin/nemefisto_helper/  # SYSTEM-service: WFP / TUN / mihomo / sing-box
│   └── binaries/          # sing-box.exe, mihomo.exe, wintun.dll, geo*.dat
├── docs/RELEASE.md        # Инструкция по выпуску релиза через CI
└── .github/workflows/     # Auto-build NSIS на push tag v*.*.*
```

**State machine коннекта**: Idle → Warming → Ready → Connecting →
Connected → Ready (после disconnect никогда не возвращаемся в Idle).

**Helper-сервис** (`nemefisto-helper.exe`) запускается с правами
SYSTEM через Windows Service Control Manager и общается с user-mode
приложением через named pipe `\\.\pipe\nemefisto-helper`. Управляет
WFP-фильтрами kill-switch, спавнит sing-box/mihomo для built-in TUN
(нужен админ для CreateAdapter WinTUN), чистит orphan-ресурсы.

---

## Релизный workflow

С версии **0.1.3** релизы выпускаются через GitHub Actions.
Подробности — [`docs/RELEASE.md`](docs/RELEASE.md).

```powershell
# Bump версии в трёх файлах: package.json, Cargo.toml, tauri.conf.json
git tag v0.X.Y -m "v0.X.Y — описание"
git push origin main --follow-tags
# CI собирает, подписывает, публикует на GitHub Releases.
# Юзеры с 0.1.3+ получают auto-update в течение 6 часов.
```

CHANGELOG в release-нотах генерируется автоматически из git log от
предыдущего тега.

---

## Roadmap

### Сделано
- ✅ sing-box миграция (0.1.2)
- ✅ Production-ready kill-switch (WFP) для обоих движков (0.1.3)
- ✅ Auto-updater + GitHub Actions CI/CD (0.1.3)
- ✅ i18n RU+EN (0.2.0)
- ✅ Drag-and-drop URL, system theme, feedback button (0.2.1)

### Запланировано
- [ ] Merge multiple subscriptions (UI-кнопка `+` + group-by-source drawer)
- [ ] EV code signing — убирает SmartScreen warning ($)
- [ ] Beta channel
- [ ] WFP per-app routing (kernel-driver, для обоих движков)
- [ ] macOS / Linux / Android / iOS порты

### Закрыто (не делаем)
- Smart auto-failover (задача провайдера через mihomo `urltest`)
- Локальная история сессий
- Speed-test (юзеры положат канал провайдеров)
- Windows Hello при запуске

---

## Приватность

Nemefisto **не собирает телеметрию**, **не отправляет crash-репорты
«домой»**, и **не имеет remote-control механизмов**. Все логи
локально на компьютере пользователя:
- `%TEMP%\NemefistoVPN\sing-box-stderr.log` / `mihomo-stderr.log`
- `C:\ProgramData\NemefistoVPN\helper.log` (kill-switch decisions)
- `C:\ProgramData\NemefistoVPN\sing-box.log` / `mihomo.log` (built-in TUN)

Deep-links и заголовки подписки имеют **строгий whitelist** — не могут
запускать процессы, читать файлы вне стандартных путей, отключать
Settings, или скрывать серверы. Подробности в [PRIVACY.md](PRIVACY.md).

---

## Лицензия

[MIT](LICENSE) — делайте что хотите, включая форк и дистрибуцию.

## Благодарности

- [SagerNet/sing-box](https://github.com/SagerNet/sing-box) — основной VPN-движок
- [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo) — второй движок (AnyTLS, Mieru)
- [WireGuard wintun](https://www.wintun.net/) — driver для TUN-адаптера
- [Loyalsoldier/v2ray-rules-dat](https://github.com/Loyalsoldier/v2ray-rules-dat) — geosite / geoip
- [Tauri](https://v2.tauri.app/) — фреймворк app
