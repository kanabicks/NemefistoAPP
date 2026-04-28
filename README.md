# nemefisto

> приватный канал поверх xray, замаскированный под обычный TLS.
> ноль логов · серверы по всему миру · один ключ — все устройства.

VPN-клиент под Windows на базе **Xray-core**. Главная цель — «VPN одной кнопкой»: подключение менее чем за 2 секунды, минимум вопросов к пользователю, максимум совместимости с современными протоколами.

Архитектура изначально готова к портированию на macOS, iOS и Android — UI отделён от системного слоя, всё Windows-специфичное вынесено в отдельный модуль.

![stage](https://img.shields.io/badge/stage-4_of_7-white?style=flat-square&labelColor=050505)
![platform](https://img.shields.io/badge/platform-windows-white?style=flat-square&labelColor=050505)
![tauri](https://img.shields.io/badge/tauri-2.0-white?style=flat-square&labelColor=050505)
![xray](https://img.shields.io/badge/xray--core-26.x-white?style=flat-square&labelColor=050505)

---

## / 01 — возможности

- **Подключение одной кнопкой** — большая круглая power-кнопка, статус через индикатор, минимум интерфейса.
- **Xray-core под капотом** — VLESS, VMess, Trojan, Shadowsocks, REALITY, XHTTP.
- **REALITY-маскировка** — трафик внешне неотличим от обращения к легитимному TLS-сайту.
- **Marzban / RemnaWave-подписки** — UA `Happ/2.7.0` + `x-hwid`, парсер массива готовых Xray-конфигов.
- **Локальный balancer** — `burstObservatory` + `leastLoad`, авто-выбор самого быстрого сервера из списка.
- **Системный прокси** — настройка Windows Internet Settings через реестр, без админских прав.
- **HWID на основе железа** — `MachineGuid` из `HKLM\SOFTWARE\Microsoft\Cryptography`. Переустановка приложения не меняет идентификатор.
- **Стиль one-button VPN** — узкая центральная колонка, монохром, brutalist tech.

---

## / 02 — стек

| слой           | технология                                |
| -------------- | ----------------------------------------- |
| фреймворк      | Tauri 2.0                                 |
| фронтенд       | React 18 · TypeScript · Tailwind · Zustand |
| бэкенд         | Rust 2021 · tokio · anyhow                 |
| VPN-ядро       | Xray-core (sidecar)                        |
| HTTP           | reqwest (rustls)                           |
| реестр Windows | winreg                                     |
| шрифты         | Space Grotesk · Inter Tight · JetBrains Mono |

---

## / 03 — запуск из исходников

**Требования:**

- Windows 10/11
- Node.js 20+
- Rust stable (`rustup install stable`)
- Tauri CLI: `npm i -g @tauri-apps/cli`

**Шаги:**

```bash
# 1. клонировать репо (бинарники xray / tun2socks / wintun.dll уже включены в src-tauri/binaries/)
git clone https://github.com/kanabicks/NemefistoAPP.git
cd NemefistoAPP

# 2. поставить зависимости фронта
npm install

# 3. dev-режим (с hot reload)
npm run tauri dev

# 4. release-сборка
npm run tauri build
```

**Для TUN-режима** (этап 4) дополнительно нужно установить helper-сервис от админа:

```powershell
# admin PowerShell после первой `cargo build`
cd src-tauri\target\debug
.\nemefisto-helper.exe install
```

Helper стартует автоматически при логине Windows и слушает named pipe `\\.\pipe\nemefisto-helper`. Управляет TUN-интерфейсом и системной маршрутизацией от SYSTEM, чтобы основное приложение не требовало UAC при каждом запуске. Удалить — `.\nemefisto-helper.exe uninstall`.

После первого запуска приложение покажет сгенерированный **HWID** в секции «настройки → hwid устройства». Скопируй его и отправь в Telegram-бот провайдера для добавления устройства в подписку.

---

## / 04 — структура

```
.
├── src/                              react фронтенд
│   ├── App.tsx                       однокнопочный UI
│   ├── App.css                       стили в духе brutalist tech
│   └── stores/                       zustand-stores (vpn, subscription)
│
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs                    точка входа Tauri
│   │   ├── ipc/commands.rs           Tauri-команды (connect, disconnect, ...)
│   │   ├── config/
│   │   │   ├── hwid.rs               чтение MachineGuid
│   │   │   ├── subscription.rs       парсер подписки (Xray JSON, base64, ...)
│   │   │   ├── xray_config.rs        генератор / patch конфигов Xray
│   │   │   └── server.rs             ProxyEntry
│   │   ├── vpn/xray.rs               управление sidecar Xray
│   │   └── platform/proxy.rs         windows system proxy через реестр
│   │
│   ├── binaries/                     xray.exe (требуется положить вручную)
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── public/                           статические ассеты (logo.png)
├── index.html                        подключение шрифтов google fonts
├── CLAUDE.md                         инструкции для Claude Code
└── README.md                         этот файл
```

---

## / 05 — дорожная карта

- [x] **этап 0** — базовый шаблон Tauri 2 + React + TS + Tailwind + Zustand
- [x] **этап 1** — Xray sidecar: запуск/остановка по кнопке, тест через SOCKS5
- [x] **этап 2** — парсинг подписок (base64 URI / Xray JSON / plain / Clash YAML)
- [x] **этап 3** — конфиги Xray, REALITY, balancer + burstObservatory, system proxy
- [x] **этап 4** — TUN-режим: WinTUN + tun2socks через привилегированный helper-сервис
- [ ] **этап 5** — state machine, прогрев при старте, оптимистичный UI
- [x] **этап 6** *(частично)* — HWID на MachineGuid; осталось: Credential Manager, автозапуск, kill switch
- [x] **этап 7** *(предварительно)* — UI в стиле nemefisto.online; осталось: пинги серверов, флаги, авто-выбор, плавные анимации

---

## / 06 — известные проблемы

- **`xray.exe` не подписан** — Microsoft Defender может ругаться при первом запуске. Добавь папку `src-tauri/binaries/` в исключения, либо `Unblock-File` через PowerShell.
- **TUN-режим пока не реализован** — переключатель `tun` есть в UI, но при выборе вернёт ошибку. Будет добавлено в этапе 4.
- **HWID-whitelist у провайдера** — если получаешь заглушку «приложение не поддерживается», скорее всего твой `MachineGuid` ещё не зарегистрирован в подписке. Скопируй HWID из настроек и отправь в бот провайдера.

---

## / 07 — лицензия

Проект приватный. Все права защищены. © 2026 Nemefisto.

---

## / 08 — контакты

- Сайт: [nemefisto.online](https://nemefisto.online)
- Telegram-бот: [@nemefistovpn_bot](https://t.me/nemefistovpn_bot)

```
─────────────────────────────────────────────────
NO-LOGS · XRAY · VLESS · REALITY · XHTTP
BUILT FOR THE OPEN INTERNET
─────────────────────────────────────────────────
```
