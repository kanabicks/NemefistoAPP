# nemefisto

> приватный VPN-клиент под Windows на двух ядрах (Xray-core и Mihomo)
> с защитой от DPI, утечек и локального детекта.
> ноль телеметрии · открытый код · одна кнопка.

«VPN одной кнопкой»: подключение менее чем за 2 секунды, минимум
вопросов к пользователю, максимум совместимости с современными
протоколами обхода блокировок. Архитектура изначально готова к
портированию на macOS, iOS и Android — UI отделён от системного
слоя, всё Windows-специфичное вынесено в `platform/`.

![tauri](https://img.shields.io/badge/tauri-2.0-white?style=flat-square&labelColor=050505)
![xray](https://img.shields.io/badge/xray--core-26.x-white?style=flat-square&labelColor=050505)
![mihomo](https://img.shields.io/badge/mihomo-1.19-white?style=flat-square&labelColor=050505)
![license](https://img.shields.io/badge/license-MIT-white?style=flat-square&labelColor=050505)
![telemetry](https://img.shields.io/badge/telemetry-zero-white?style=flat-square&labelColor=050505)

---

## / 01 — что это умеет

### Подключение
- **Подключение в один клик** с прогревом sidecar-процессов при
  старте — первое нажатие < 500 мс.
- **Два режима**: системный прокси (HKCU registry, без админа) и
  TUN (весь трафик через WinTUN-адаптер, нужен helper-сервис).
- **Optimistic UI**: интерфейс мгновенно реагирует на намерение,
  бэкенд догоняет в фоне.

### Протоколы и транспорты

|              | Xray | Mihomo |
|---|---|---|
| VLESS, VMess, Trojan, SS, SOCKS5 | ✓ | ✓ |
| Hysteria2 (с obfs salamander)    | ✓ | ✓ |
| WireGuard                        | ✓ | ✓ |
| TUIC                             | – | ✓ |
| AnyTLS                           | – | ✓ |
| TCP / WS / gRPC / h2 / xhttp / httpupgrade | ✓ | ✓ |
| TLS / REALITY / Vision (XTLS)    | ✓ | ✓ |

Один пользователь использует одно ядро на сессию. Выбор движка
автоматический (заголовок `X-Nemefisto-Engine` подписки), либо
ручной (Settings → «движок»).

### Подписки

- **Универсальный парсер**: base64-список ссылок, raw-список,
  Marzban-style xray-json (UA `Happ/2.7.0`), полный Xray JSON,
  Mihomo YAML (UA `clash-verge/v2.0.0`), mixed-формат.
- **Server-driven UX**: провайдер подписки задаёт через HTTP-заголовки
  тему, фон, движок, режим, anti-DPI, объявления, ссылку на премиум,
  правила маршрутизации. Пользователь всегда может переопределить —
  бейдж «из подписки» в UI рядом с такими настройками.
- **Per-engine UA**: при смене движка автоматический re-fetch с
  правильным UA, чтобы получить ровно тот формат, который панель
  поддерживает для этого ядра.
- **Стандартные заголовки** (3x-ui / Marzban / x-ui): трафик,
  срок, имя, поддержка, премиум, объявления.

### Anti-DPI обвязка
- TCP-фрагментация TLS ClientHello (Xray freedom-fragment outbound).
- Шумовые UDP-пакеты (`noises`).
- Server-address-resolve через DoH (минуя системный DNS).
- Hysteria2 obfs `salamander` — пакеты маскируются под мусор.
- Все три механизма управляются из Settings или подписки.

### Защита от утечек
- **Kill switch на WFP** (Windows Filtering Platform): фильтры на уровне
  ядра, DYNAMIC session — авто-снимаются если процесс падает,
  cleanup orphan-фильтров на старте helper'а, Service Recovery
  через SCM. **Strict mode** — даже direct-маршруты xray
  блокируются (для пользователей со split-routing «всё через VPN»).
- **DNS leak protection**: `:53` блокируется кроме нашего VPN-DNS.
- **Leak-test**: после connect — `cdn-cgi/trace.cloudflare` +
  `ipwho.is` через туннель, toast «твой IP сейчас X (страна)».
- **IPv6 leak protection**: блок-all v6 в WFP + опция принудительного
  отключения IPv6 на интерфейсе.

### Защита от локального детекта
- **Рандомизация портов** inbound: `[30000, 60000)` вместо
  стандартных `1080/1087` — сторонний сканер не найдёт за миллисекунды.
- **SOCKS5 password-auth** в TUN/LAN-режимах: даже если порт найден,
  без пароля проверить тип трафика нельзя.
- **Маскировка имени TUN-адаптера**: `wlan99` / `Local Area Connection N` /
  `Ethernet N` вместо `nemefisto-<pid>`. `GetAdaptersAddresses` не
  выдаёт VPN-клиент.
- **TUN-only strict mode**: можно полностью убрать proxy-режим из UI.

### Маршрутизация
- **Routing-профили** (Marzban-формат с `DirectSites/ProxyIp/BlockSites`).
- **Geofiles**: `geoip.dat` + `geosite.dat` от Loyalsoldier с
  `.sha256`-оптимизацией (не качаем повторно если хеш не сменился).
- **Static и Autorouting** (с автообновлением каждые 12ч/24ч/3д/7д).
- **Per-process правила** через Mihomo `PROCESS-NAME` (на Windows нужен
  `find-process-mode: always`).
- **13.Q minimal-RU template** — auto-fallback `geosite:ru → DIRECT`,
  ads → BLOCK, если подписка не задаёт routing.

### UX
- **Системный трей** с цветным статусом и быстрым меню.
- **Bandwidth-меттер** (1 Гц через `GetIfTable2`) и **floating window**
  (мини-окно alwaysOnTop с скоростью).
- **Глобальные горячие клавиши** (`Ctrl+Shift+V` toggle VPN и др.).
- **Trusted Wi-Fi**: автоматически выключать VPN в доверенных сетях
  (по SSID).
- **Конфликт-детект**: при старте находим сторонние VPN-клиенты,
  routing-конфликты и orphan-ресурсы прошлой сессии — предлагаем
  починить.
- **Crash recovery**: если приложение упало — следующий старт чистит
  оставленные WFP-фильтры, прокси-настройки, TUN-адаптеры.
- **Темы** (5 штук) + **3D-фоны** (4 штуки) + **стили кнопки** + **5
  готовых пресетов**.

### Безопасность хранилища
- URL подписки и HWID-override → Windows Credential Manager
  (`keyring-rs`), не в `localStorage`.
- Helper-сервис слушает named pipe с access-check'ами от SYSTEM.
- Deep-link и заголовки подписки имеют **строгий whitelist** —
  не могут запускать процессы, читать файлы вне стандартных путей,
  отключать Settings, скрывать серверы.

---

## / 02 — стек

| слой           | технология                                        |
| -------------- | ------------------------------------------------- |
| фреймворк      | Tauri 2.0                                         |
| фронтенд       | React 19 · TypeScript · Tailwind 4 · Zustand 5    |
| бэкенд         | Rust 2021 · tokio · anyhow · thiserror             |
| VPN-ядра       | Xray-core · Mihomo (sidecar)                       |
| TUN            | WinTUN + tun2socks (sing-box / hev-socks5-tunnel)   |
| HTTP           | reqwest (rustls)                                   |
| WFP            | windows-sys (FwpmEngineOpen / FwpmFilterAdd / DYNAMIC session) |
| хранилище      | winreg + keyring-rs (Credential Manager)           |
| logs           | tracing + ротация файлов                           |
| геобазы        | sha2 (verify) + reqwest no-proxy                   |

---

## / 03 — запуск из исходников

**Требования:**
- Windows 10/11
- Node.js 20+
- Rust stable (`rustup install stable`)
- Tauri CLI: `npm i -g @tauri-apps/cli`

```bash
git clone https://github.com/kanabicks/NemefistoAPP.git
cd NemefistoAPP
npm install
npm run tauri dev          # dev с hot reload
npm run tauri:bundle       # release NSIS-installer
```

**Helper-сервис** (для TUN-режима и kill switch) устанавливается
автоматически при первом включении соответствующей опции из UI.
Ручная установка:

```powershell
# admin PowerShell, из папки с собранным exe
.\nemefisto-helper.exe install
```

Helper работает с правами SYSTEM, слушает только локальный named
pipe `\\.\pipe\nemefisto-helper`. Удалить — `.\nemefisto-helper.exe uninstall`.

---

## / 04 — структура

```
.
├── src/                          react фронтенд
│   ├── App.tsx                   главное окно
│   ├── components/               Settings, ServerSelector, и др.
│   ├── stores/                   zustand: vpn / subscription / settings / toast
│   └── lib/                      deepLinks, hooks, leakTest, constants
│
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs                точка входа Tauri 2
│   │   ├── ipc/commands.rs       все Tauri-команды
│   │   ├── config/               подписки, Xray/Mihomo конфиги, routing-профили, geofiles
│   │   ├── platform/             Windows-специфика (proxy, network, tray, wifi, ...)
│   │   ├── vpn/                  state machine, xray, mihomo, tun-helper-coord
│   │   └── bin/nemefisto_helper/ helper-сервис: WFP, TUN, routing, named-pipe
│   ├── binaries/                 xray.exe, mihomo.exe, tun2socks.exe, wintun.dll
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── PRIVACY.md                    политика конфиденциальности
├── LICENSE                       MIT
├── CLAUDE.md                     внутренние roadmap и архитектурные принципы
└── README.md                     этот файл
```

---

## / 05 — приватность

**Никакой телеметрии.** Приложение не собирает, не передаёт и не хранит
на сторонних серверах данные о пользователе или его активности.
Все логи — локальные. Нет аналитики, A/B-тестов, crash-репортинга «домой».

Подробности в [PRIVACY.md](PRIVACY.md).

Список сетевых запросов которые приложение делает:
- HTTP к URL подписки (который вписал сам пользователь);
- скачивание `geoip.dat`/`geosite.dat` с GitHub Releases (в обход VPN);
- routing-профили из URL по расписанию (если активна autorouting-подписка);
- опциональный leak-test после connect (Cloudflare cdn-trace + ipwho.is);
- сам VPN-трафик через выбранный сервер.

Всё. Никаких других серверов.

---

## / 06 — лицензия

[MIT](LICENSE) · открытый исходный код, форки и модификации
разрешены без ограничений. Нужно только сохранить notice о copyright.

---

## / 07 — контакты

- Сайт: [web.nemefisto.online](https://web.nemefisto.online)
- Telegram: [@nemefistovpn_bot](https://t.me/nemefistovpn_bot)
- GitHub: [kanabicks/NemefistoAPP](https://github.com/kanabicks/NemefistoAPP)

---

## / 08 — благодарности

- [Xray-core](https://github.com/XTLS/Xray-core) — низколатентное
  обходное ядро с REALITY и Vision.
- [Mihomo](https://github.com/MetaCubeX/mihomo) — форк Clash Meta
  с TUIC, AnyTLS и нативной per-process маршрутизацией.
- [Tauri](https://tauri.app) — кроссплатформенный фреймворк, на
  котором всё держится.
- [WinTUN](https://www.wintun.net) — userspace TUN-драйвер от команды
  WireGuard.
- [Loyalsoldier/v2ray-rules-dat](https://github.com/Loyalsoldier/v2ray-rules-dat) —
  geoip/geosite базы.

```
─────────────────────────────────────────────────
NO-TELEMETRY · XRAY + MIHOMO · WFP · OPEN-SOURCE
─────────────────────────────────────────────────
```
