# VPN-клиент под Windows — контекст проекта

## О проекте
Разрабатываем VPN-клиент под Windows на базе Xray-core. Главная цель продукта — «VPN одной кнопкой» с подключением менее чем за 2 секунды и минимумом вопросов к пользователю. В планах — портирование на macOS, iOS и Android, поэтому архитектура должна это учитывать (UI отделён от системного слоя).

**Все ответы, комментарии в коде, сообщения коммитов и пояснения — на русском языке.** Технические термины (Tauri, sidecar, TUN и т.п.) оставляй как есть.

## Технологический стек
- **Фреймворк**: Tauri 2.0 (выбран ради будущей кроссплатформенности)
- **Фронтенд**: React + TypeScript + Tailwind CSS + Zustand для state
- **Бэкенд**: Rust (асинхронность через tokio)
- **VPN-ядро**: Xray-core как sidecar-процесс (xray.exe), управление через gRPC API Xray
- **Альтернативное ядро (с этапа 8.B)**: Mihomo (форк Clash Meta) как второй sidecar
- **TUN-драйвер**: WinTUN (от команды WireGuard)
- **tun2socks**: hev-socks5-tunnel или tun2socks из sing-box как sidecar
- **Безопасное хранилище**: Windows Credential Manager через `keyring-rs`
- **Логирование**: crate `tracing` с ротацией файлов

## Архитектурные принципы
1. **Долгоживущие ресурсы**: Xray sidecar и WinTUN-адаптер создаются при старте приложения и живут до его закрытия. Не пересоздаём при connect/disconnect.
2. **State machine коннекта**: Idle → Warming → Ready → Connecting → Connected → Ready (после отключения). Никогда не возвращаемся в Idle пока приложение запущено.
3. **Оптимистичный UI**: UI сразу отражает намерение пользователя, бэкенд догоняет в фоне.
4. **Умные дефолты, минимум вопросов**: При первом запуске спрашиваем только URL подписки. Всё остальное (протокол, сервер, DNS, kill switch) имеет разумные значения по умолчанию.
5. **Прогрев**: При старте приложения резолвим DNS серверов, пингуем их в фоне, готовим TUN, запускаем Xray в idle. Первый клик «Connect» — менее 500ms.
6. **Server-driven UX**: провайдер подписки может задать дефолты (тема, движок, маршрутизация, объявления) через HTTP-заголовки. Пользователь всегда может переопределить.
7. **Никакой телеметрии и remote control**: приложение не отправляет диагностические/аналитические данные. Все логи — локальные (`%TEMP%\NemefistoVPN\xray-stderr.log` + tracing-файл). Код открыт. Deep-link и заголовки подписки имеют строгий whitelist — не могут запускать процессы, читать файлы вне стандартных путей, отключать Settings, или скрывать серверы. Никаких эквивалентов «HandlerService»-style сервисов в кодовой базе.
8. **Защита от локального детекта**: сторонний процесс на машине не должен дёшево обнаружить, что VPN-клиент запущен. Защита layered: (1) **9.H** рандомизация портов inbound `[30000, 60000)` — стандартные `7890/1080/1087` не отвечают; (2) **9.G** SOCKS5 password-auth для TUN/LAN — даже если порт найден, без пароля нельзя проверить тип трафика; (3) **12.E** маскировка имени TUN-адаптера (`wlan99` / `Local Area Connection N` / `Ethernet N`) — `GetAdaptersAddresses` не выдаёт «nemefisto-». Угроза задокументирована: https://habr.com/ru/news/1020902/.

## Соглашения по коду
- **Rust**: используем `anyhow::Result` для прикладных ошибок, `thiserror` для библиотечных. Фоновые задачи через `tokio::spawn`. Публичные функции — с doc-комментариями на русском.
- **TypeScript**: strict mode включён. Валидация данных через `zod`. Компоненты функциональные, hooks-стиль.
- **Именование**: snake_case в Rust, camelCase в TS, kebab-case в файлах фронтенда.
- **Никаких `unwrap()` в продакшен-коде Rust** — только в тестах и в местах, где гарантированно невозможна паника (с комментарием почему).

## Структура проекта

```
/
├── src/                    # React фронтенд
│   ├── components/
│   ├── stores/             # Zustand stores
│   ├── lib/                # Утилиты, типы, IPC-обёртки
│   └── App.tsx
├── src-tauri/              # Rust бэкенд
│   ├── src/
│   │   ├── main.rs
│   │   ├── vpn/            # Логика VPN (state machine, tun, xray, mihomo)
│   │   ├── config/         # Парсинг подписок, конфиги, routing-профили
│   │   ├── platform/       # Windows-специфичный код (изолированно для будущего портирования)
│   │   └── ipc/            # Tauri commands
│   ├── binaries/           # xray.exe, mihomo.exe, tun2socks.exe, wintun.dll
│   └── Cargo.toml
└── CLAUDE.md
```

## Принципы работы со мной (для Claude Code)
1. **Двигайся поэтапно.** Не пытайся сделать всё за один проход. Разбивай задачи на маленькие проверяемые шаги.
2. **Перед каждым шагом** кратко объясни на русском, что собираешься сделать и почему. Дождись моего «ок» прежде чем писать большой объём кода (для мелких правок — не нужно).
3. **После каждого шага** запускай `cargo check` (для Rust) и `npm run build` или `tsc --noEmit` (для фронта), чтобы убедиться, что всё собирается. Сообщай результат.
4. **Если возникает ошибка сборки** — попробуй починить сам, максимум 3 попытки. Если не вышло — стоп, объясни проблему и спроси меня.
5. **Не выдумывай API.** Если не уверен в синтаксисе свежей библиотеки (Tauri 2, wintun-rs, конкретного crate) — попроси меня дать ссылку на документацию или проверь через web fetch если есть доступ.
6. **Никаких заглушек типа `// TODO: implement later`** в основном пути выполнения. Если что-то не реализовано — явно скажи мне об этом текстом, не прячь в коде.
7. **Перед коммитом** показывай мне краткое summary изменений (одним абзацем), не сам diff.

## Этапы разработки (roadmap)

- **Этап 0**: Настройка проекта Tauri 2 + React + TypeScript, проверка что пустой шаблон собирается и запускается.
- **Этап 1**: Скачивание Xray-core, помещение xray.exe в sidecar, минимальный запуск/остановка Xray из Rust по кнопке в UI. Проверка через локальный SOCKS5-прокси в браузере.
- **Этап 2**: Парсинг подписки (формат vless:// и base64-список). UI: импорт URL подписки, отображение списка серверов.
- **Этап 3**: Конфигурация Xray по выбранному серверу. SOCKS5 inbound + VLESS/VMess/Trojan outbound.
- **Этап 4**: WinTUN-интеграция и tun2socks. Системный трафик идёт через VPN, а не только браузер.
- **Этап 5**: State machine коннекта, прогрев при старте, оптимистичный UI.
- **Этап 6**: Безопасное хранение конфигов (Credential Manager), автозапуск, kill switch, обработка смены сети.
- **Этап 7**: Шлифовка UX — анимации кнопки, пинги серверов, автовыбор лучшего, обработка ошибок понятным языком.

---

## Этап 8 — Двухядерная архитектура и server-driven config

**Цель**: уникальная фича приложения — два VPN-движка на выбор + конфигурация
дефолтов через HTTP-заголовки подписки + per-process routing.

### Архитектура движков

- **Xray** — текущий движок. Сильные стороны: REALITY / Vision / XHTTP /
  HTTPUpgrade, низколатентный обход DPI. С 1.8.16+ **поддерживает
  Hysteria2 outbound**, с 1.8.6+ — **WireGuard outbound**. То есть для
  большинства современных подписок Mihomo не обязателен.
- **Mihomo** (форк Clash Meta) — добавляется. Уникальная зона: **TUIC**,
  **AnyTLS**, гибкий routing с native `PROCESS-NAME` matcher (per-process
  без WFP). Дублирует поддержку других протоколов с Xray.

Один пользователь использует **одно ядро на сессию**. Выбор:
1. Из заголовка `X-Nemefisto-Engine` подписки;
2. Иначе из настроек пользователя;
3. По дефолту Xray.

Серверы из подписки помечаются полем `engine_compat`. Если выбранное ядро
несовместимо с сервером — UI показывает предупреждение и предлагает
переключить движок.

### Корректная таблица совместимости протоколов

| Протокол / транспорт | Xray | Mihomo |
|---|---|---|
| VLESS, VMess, Trojan, SS, SOCKS5 | ✅ | ✅ |
| **Hysteria2** | ✅ (1.8.16+) | ✅ |
| **WireGuard** | ✅ (1.8.6+) | ✅ |
| **TUIC** | ❌ | ✅ |
| **AnyTLS** | ❌ | ✅ |
| Transport: TCP, WS, gRPC, h2 | ✅ | ✅ |
| Transport: **XHTTP** | ✅ (1.8.18+) | ✅ (1.18+) |
| Transport: **HTTPUpgrade** | ✅ | ✅ |
| Security: TLS, REALITY | ✅ | ✅ |
| Vision (XTLS) | ✅ | ✅ |

### Универсальный парсер подписок

`src-tauri/src/config/subscription.rs` распознаёт:
- base64-список ссылок (vless / vmess / trojan / ss / hysteria2 / tuic / wireguard / socks);
- raw текстовый список ссылок (по строке);
- готовый Xray JSON-массив (Marzban-style — UA `Happ/2.7.0`);
- готовый Xray JSON-объект (одиночный полный конфиг с inbounds/outbounds/routing);
- готовый Mihomo YAML-конфиг;
- mixed формат (base64 со смесью + спец-строки маршрутизации);
- спец-строки в теле подписки (см. раздел deep-links в этапе 11).

Любая запись приводится к единому `ProxyEntry` с маркером совместимости
с движками.

### Server-driven config (HTTP-заголовки)

При запросе подписки сервер может вернуть заголовки, которые задают
**defaults** для клиента. Все заголовки опциональны, клиент игнорирует
неизвестные ключи.

#### 1. Стандартные заголовки подписок (де-факто индустриальный стандарт — 3x-ui / Marzban / x-ui / sing-box)

| Заголовок | Формат | Назначение |
|---|---|---|
| `subscription-userinfo` | `upload=X;download=Y;total=Z;expire=T` | Статистика трафика и unix-timestamp срока истечения. UI показывает «использовано X из Y, истекает через N дней» |
| `profile-title` | текст или `base64:...` | Имя подписки (≤25 символов). Используется вместо URL в UI |
| `profile-description` | текст или `base64:...` | Описание подписки |
| `profile-update-interval` | число (часы) | Интервал автообновления подписки. Перекрывает наш `autoRefreshHours`, если пользователь не менял вручную |
| `support-url` | URL | Ссылка на поддержку. UI показывает кнопку «поддержка» в карточке подписки |
| `profile-web-page-url` | URL | Ссылка на сайт подписки. Заменяет нашу захардкоженную «личный кабинет» |
| `premium-url` | URL | Ссылка на премиум. UI показывает кнопку «премиум» если задана |
| `announce` | текст или `base64:...` | Текстовое объявление от провайдера (≤200 символов). Показывается баннером сверху |
| `announce-url` | URL | Кликабельная ссылка для объявления |
| `content-disposition` | `attachment; filename="..."` | Fallback для имени подписки если `profile-title` не задан |
| `sort-order` | `ping` \| `name` \| `none` | Сортировка серверов по умолчанию |

#### 2. Заголовки Nemefisto (наше расширение для тонкой настройки UX)

| Заголовок | Значение |
|---|---|
| `X-Nemefisto-Engine` | `xray` \| `mihomo` |
| `X-Nemefisto-Mode` | `proxy` \| `tun` |
| `X-Nemefisto-Theme` | `dark` \| `light` \| `midnight` \| `sunset` \| `sand` |
| `X-Nemefisto-Background` | `crystal` \| `tunnel` \| `globe` \| `particles` |
| `X-Nemefisto-Button-Style` | `glass` \| `flat` \| `neon` \| `metallic` |
| `X-Nemefisto-Preset` | `none` \| `fluent` \| `cupertino` \| `vice` \| `arcade` \| `glacier` |
| `X-Nemefisto-Routes` | base64-encoded JSON с domain/ip-правилами |
| `X-Nemefisto-App-Rules` | base64-encoded JSON с per-process правилами |

#### Заголовки запроса (что мы отправляем)

```
User-Agent: Nemefisto/<version>/<platform>
Accept: */*
Accept-Language: ru-RU
x-app-version: <semver>
x-device-locale: <язык>
x-client: Nemefisto
```

Если включена отправка HWID:

```
x-hwid: <hwid>
x-device-os: Windows | macOS | Linux | iOS | Android
x-ver-os: <версия ОС>
x-device-model: <модель устройства>
```

#### Override-логика

```
effective[key] = userOverride[key] ?? subscriptionHints[key] ?? defaults[key]
```

- Если пользователь не трогал настройку — используется значение из заголовков.
- Если пользователь явно переключил — используется его выбор (override).
- Если заголовков нет — поведение как сейчас, всё ручками.

В UI рядом с настройками показывается badge «из подписки» когда значение
пришло из заголовков и не переопределено пользователем.

#### Безопасность заголовков

- **Только whitelist ключей.** Любые другие заголовки игнорируются.
- Заголовки **не могут**: запускать процессы, читать/писать файлы вне
  стандартных путей приложения, отключать Settings, скрывать серверы из
  списка, изменять URL подписки.
- Заголовки **могут**: задавать UI-настройки, выбирать движок и режим,
  предоставлять правила routing'а (которые потом проверяются и
  применяются к Xray/Mihomo конфигу).

### Per-process routing

**Правила вида `<exe-name> → PROXY | DIRECT | BLOCK`.**

Реализация:
- **Mihomo**: нативно через matcher `PROCESS-NAME` (требует
  `find-process-mode: always` в YAML). Просто конвертируем `appRules`
  в правила Mihomo при генерации конфига.
- **Xray**: на Windows нативно не поддерживается. Если выбран Xray и
  заданы appRules — UI показывает предупреждение «правила приложений
  работают только с Mihomo».

Хранение в settings:

```ts
appRules: Array<{
  exe: string;          // "telegram.exe"
  action: "proxy" | "direct" | "block";
  comment?: string;
}>
```

UI: Settings → раздел «правила приложений» → список + кнопка «добавить»
с file-picker'ом для выбора exe.

### Этапы реализации

- **8.A** — универсальный парсер подписок (vmess / trojan / ss / hy2 / tuic / wireguard / socks + Mihomo YAML + полные Xray JSON).
- **8.A.1** — *(срочный hotfix, см. ниже)* завершение Xray-поддержки: hy2/wireguard outbounds + xhttp/httpupgrade transports + правка `engine_compat` для hy2/wireguard.
- **8.B** — Mihomo как второй sidecar; UI-селект движка; helper-coordination для TUN с любым ядром. **Уникальная зона Mihomo сократилась до TUIC + AnyTLS + native per-process** — но всё ещё нужен.
- **8.C** — заголовки подписки (стандартные + Nemefisto) + override-логика + UI-бейджи «из подписки» + UI для `subscription-userinfo` / `announce` / `support-url` / `premium-url`.
- **8.D** — per-process routing (Mihomo-only через PROCESS-NAME) с UI-редактором правил. Альтернативная реализация через WFP (Windows-native, для обоих движков) — см. этап 13.G.
- **8.E** — релизный NSIS-installer (см. ниже).

### 8.A.1 — Завершение поддержки протоколов и транспортов

**Срочный hotfix** к коммиту `6fcb4d9` (этап 8.A): ошибочно маркировал
hy2 и wireguard как Mihomo-only, тогда как современный Xray умеет оба.
Также не были добавлены два важных Xray-транспорта.

Изменения в коде:

1. **`config/subscription.rs`** — `engine_compat` для парсеров:
   - `parse_hysteria2()` → `engines_both()` (было `engines_mihomo_only()`);
   - `parse_wireguard()` → `engines_both()` (было `engines_mihomo_only()`);
   - функция `engines_mihomo_only` остаётся — теперь только для **TUIC**
     и **AnyTLS** (yaml_proxy_to_entry helper).

2. **`config/xray_config.rs`** — добавить новые `build_*` функции:
   - `build_hysteria2(entry)` — VLESS-style outbound с `protocol:
     "hysteria2"`, settings включают `password` + `obfs` (если
     задано в raw) + `serverName` + `alpn: ["h3"]`.
   - `build_wireguard(entry)` — `protocol: "wireguard"`, settings:
     `secretKey`, `address` (массив `"10.0.0.2/32"`), `peers` с
     `publicKey`, `endpoint` = server:port, `mtu`, `reserved`
     (если есть).
   - Подключить в `build_outbound()`: убрать `bail!` для hy2/wireguard.

3. **`config/xray_config.rs`** — расширить `build_stream()` новыми
   transport-ами:
   - `"xhttp"` →
     ```rust
     let path = raw["path"].as_str().unwrap_or("/");
     let host = raw["host"].as_str().unwrap_or("");
     let mode = raw["mode"].as_str().unwrap_or("auto");
     // mode: auto | packet-up | stream-up | stream-one
     let mut x = json!({ "path": path, "mode": mode });
     if !host.is_empty() { x["host"] = host.into(); }
     s["xhttpSettings"] = x;
     ```
   - `"httpupgrade"` →
     ```rust
     let path = raw["path"].as_str().unwrap_or("/");
     let host = raw["host"].as_str().unwrap_or("");
     let mut hu = json!({ "path": path });
     if !host.is_empty() { hu["host"] = host.into(); }
     s["httpupgradeSettings"] = hu;
     ```

После 8.A.1 пользователи смогут подключаться к hy2/wireguard
серверам в Xray-only клиенте, **без необходимости 8.B (Mihomo)**.
Это убирает блокирующее «требуется Mihomo» сообщение для большинства
современных подписок.

**Время реализации**: ~30–40 минут. Должен быть **первым делом
следующей сессии** (горячие следы, простые правки).

### 8.E — Релизный NSIS-installer

Цель: один setup.exe который пользователь скачивает с сайта,
дважды кликает, и приложение готово к работе.

- Все sidecar (xray, mihomo, tun2socks, wintun.dll) добавляются в
  `tauri.conf.json` через `externalBin` или `resources`.
- `nemefisto-helper.exe` собирается отдельно release-сборкой и
  включается в bundle.
- `helper_bootstrap.rs` ищет helper в `<install-dir>/` или
  в `<install-dir>/resources/`, не только в exe-dir.
- `webviewInstallMode: "downloadBootstrapper"` — auto-install
  WebView2 при отсутствии (Win10 без обновлений).
- Кастомная иконка и метаданные NSIS (название, описание, версия,
  издатель).
- Опциональная страница «Запустить Nemefisto после установки» в
  wizard.
- Output: `Nemefisto_<version>_x64-setup.exe` в
  `src-tauri/target/release/bundle/nsis/`.

---

## Этап 9 — Защита от конфликтов с другими VPN-клиентами

**Цель**: приложение не падает и не оставляет систему в сломанном
состоянии когда параллельно активен другой VPN, заняты порты, остались
orphan-ресурсы от прошлых сессий.

### 9.A — Авто-выбор свободных портов (готово)
- `find_free_port(start)` сканит вверх до первого свободного.
- Дополнительно: команда `get_port_conflict_info()` возвращает имя
  процесса, занявшего стандартный порт — UI показывает в логах.
- Стартовая точка с этапа 9.H — псевдослучайный порт `[30000, 60000)`,
  не фиксированные `1080/1087`.

### 9.B — Детект известных VPN-клиентов
При старте приложения и при connect перебираем процессы. Знакомые
имена (Happ, OutlineClient, OpenVPNGUI, wireguard, nordvpn, ExpressVPN,
ProtonVPN, mullvad, v2rayN, Clash, Hiddify, INCY, и др.) — показываем
неблокирующий warning-banner.

Implementation: `EnumProcesses` Win32 API в `platform/processes.rs`.

### 9.C — Детект конфликтов routing-таблицы
Перед спавном tun2socks helper проверяет наличие сторонних TUN-адаптеров
с активными half-default или 0.0.0.0/0 маршрутами. Если найдено —
bail с сообщением «отключите другой VPN». Опциональный force-mode для
продвинутых пользователей.

### 9.D — System proxy backup/restore
При connect (mode=proxy) сохранять предыдущие значения registry-keys
`ProxyEnable` / `ProxyServer` / `ProxyOverride` в
`%LOCALAPPDATA%\NemefistoVPN\proxy_backup.json`. При disconnect —
восстанавливать. На случай краша — детект backup-файла на старте app
с предложением восстановить.

### 9.E — Cleanup orphan-ресурсов на старте
- Helper-сервис при старте: удалить все WinTUN-адаптеры с префиксом
  `nemefisto-` (best-effort через `Remove-NetAdapter`); вычистить
  routing-rules с нашим NextHop=198.18.0.1.
- Main app при старте: detect proxy_backup.json и предложить restore.

### 9.F — Уникальное имя TUN (готово)
Каждая сессия создаёт `nemefisto-<pid>` — двойной запуск приложения
не конфликтует.

### 9.G — SOCKS5 inbound authentication

**Цель**: защита от использования нашего локального SOCKS5 прокси
сторонним процессом / устройством в LAN.

Сейчас наш Xray inbound — `auth: noauth`, что позволяет:
- любому процессу на машине в proxy/TUN-режиме гонять свой трафик
  через VPN (включая малварь);
- в LAN-режиме — любому устройству в Wi-Fi сети использовать клиент
  как открытый прокси.

Решение: при старте Xray генерируется случайный пароль (UUID v4),
inbound настраивается с `auth: password`. Пароль знает только наше
приложение и его компоненты.

Применение по режимам:
- **TUN-режим**: ставим всегда. tun2socks мы спавним сами и передаём
  ему `--proxy socks5://user:pass@127.0.0.1:port`. Прозрачно для
  пользователя.
- **LAN-режим** (`allow_lan: true`): обязательно. UI показывает
  сгенерированный логин/пароль с кнопкой копирования; LAN-клиенты
  вводят их вручную в настройках браузера.
- **Proxy-режим (loopback, default)**: оставляем `noauth`. Windows
  registry для системного прокси не поддерживает `user:pass@host:port`
  синтаксис, и браузеры будут получать 407 auth challenge на каждый
  запрос. Loopback и так только локально-доступен.

### 9.H — Рандомизация портов inbound (готово)

**Цель**: защита от локального сканирования VPN-клиента.

Любое приложение на машине без админ-прав может за миллисекунды
просканировать стандартные SOCKS-порты (`7890`, `1080`, `1087`) и
обнаружить запущенный VPN-клиент — без какого-либо доступа к нашему
процессу. Это активно применяется для детекта VPN-пользователей
(см. https://habr.com/ru/news/1020902/, идеи позаимствованы у dropweb).

Решение: при подключении inbound'ы Xray стартуют с псевдослучайных
портов в диапазоне `[30000, 60000)`, выбираемых из наносекунд
системных часов. От запуска к запуску значение разное и для
стороннего сканера непредсказуемо без полного скана 30 000 портов.

Реализация: `vpn::xray::random_high_port()` → передаётся в
`find_free_port` как стартовая точка для SOCKS и HTTP inbound'ов.
В связке с **9.G** (SOCKS5 auth для TUN/LAN) даёт двойную защиту:
сторонний сканер не найдёт порт, а если найдёт — не сможет его
использовать без пароля.

⚠️ Loopback proxy-режим всё ещё `noauth` (ограничение Windows registry
из 9.G), но порт уже не предсказуем. Параноикам — TUN-режим, который
в этой связке полностью закрыт.

---

## Этап 10 — Anti-DPI обвязка Xray

**Цель**: повысить процент успешных подключений в условиях агрессивного
DPI (Россия, Иран, Китай). Все три механизма опциональны и управляются
из настроек или заголовков подписки.

### 10.A — TCP-фрагментация

Параметр Xray `outbounds[].streamSettings.sockopt.tcpFastOpen` + freedom-fragment
outbound. Делит TLS ClientHello на куски, мешая DPI собрать его обратно.

**HTTP-заголовки подписки:**

| Заголовок | Формат | Значение |
|---|---|---|
| `fragmentation-enable` | `0` \| `1` | Включить/выключить |
| `fragmentation-packets` | `tlshello` \| `1-3` \| `all` | Какие пакеты фрагментировать |
| `fragmentation-length` | `min-max` (байты) | Размер фрагмента |
| `fragmentation-interval` | `min-max` (мс) | Задержка между фрагментами |

**Дефолты** при `fragmentation-enable: 1` и отсутствии остальных:
- `packets: tlshello`, `length: 10-20`, `interval: 10-20`.

**Settings UI**: переключатель «фрагментация TCP» + три текстовых поля для тонкой настройки.

### 10.B — Шумовые пакеты (noises)

Xray `freedom` outbound с фейковыми UDP-пакетами для запутывания DPI.

**HTTP-заголовки:**

| Заголовок | Формат | Значение |
|---|---|---|
| `noises-enable` | `0` \| `1` | Включить/выключить |
| `noises-type` | `rand` \| `str` \| `hex` | Тип содержимого |
| `noises-packet` | строка или `min-max` | Содержимое или размер |
| `noises-delay` | `min-max` (мс) | Задержка между пакетами |

### 10.C — Server-address-resolve через DoH

Перед подключением к серверу VPN резолвим его адрес через DoH (минуя
системный DNS, который может быть отравлен/заблокирован).

**HTTP-заголовки:**

| Заголовок | Формат | Значение |
|---|---|---|
| `server-address-resolve-enable` | `0` \| `1` | Включить |
| `server-address-resolve-dns-domain` | URL | DoH endpoint (например `https://cloudflare-dns.com/dns-query`) |
| `server-address-resolve-dns-ip` | IP | Bootstrap IP для самого DoH-сервера |

**Settings UI**: «обход DNS-блокировок» + поле DoH-сервера + поле bootstrap-IP.

### Этапы реализации

- **10.A** — TCP-фрагментация: парсинг заголовков, поле в настройках, генерация freedom-fragment outbound в Xray-конфиге.
- **10.B** — Noises: парсинг + UI + генерация конфига.
- **10.C** — DoH-resolve: реализация через `hickory-resolver` или `reqwest`, кеш результата на сессию.

---

## Этап 11 — Routing-профили, geofiles и расширенные deep-links

**Цель**: пользователь может импортировать профиль маршрутизации одним
кликом из ссылки, профиль автоматически обновляется по расписанию,
правила применяются и к Xray, и к Mihomo.

### 11.A — Формат routing-профиля

JSON-документ с правилами маршрутизации, совместимый с типовыми панелями:

```json
{
  "Name": "RoscomVPN",
  "GlobalProxy": "true",
  "LastUpdated": "1700000000",
  "DomainStrategy": "IPIfNonMatch",

  "RemoteDNSType": "DoH",
  "RemoteDNSDomain": "https://cloudflare-dns.com/dns-query",
  "RemoteDNSIP": "1.1.1.1",
  "DomesticDNSType": "DoH",
  "DomesticDNSDomain": "https://dns.google/dns-query",
  "DomesticDNSIP": "8.8.8.8",
  "DnsHosts": { "example.com": "1.2.3.4" },
  "FakeDNS": "false",

  "DirectSites": ["geosite:ru"],
  "DirectIp":    ["geoip:ru", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"],
  "ProxySites":  [],
  "ProxyIp":     [],
  "BlockSites":  ["geosite:category-ads-all"],
  "BlockIp":     [],

  "Geoipurl":   "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat",
  "Geositeurl": "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat",
  "useChunkFiles": false
}
```

**Поля:**
- `GlobalProxy` — весь трафик через прокси (`true`) или только по правилам ProxySites/ProxyIp (`false`).
- `DomainStrategy` — `AsIs` (без резолва) / `IPIfNonMatch` (резолв если домен не сматчился) / `IPOnDemand` (всегда резолвить в IP).
- `DirectSites` / `DirectIp` / `ProxySites` / `ProxyIp` / `BlockSites` / `BlockIp` — массивы правил. Поддерживаются `geosite:XX`, `geoip:XX`, конкретные домены, IP/CIDR.
- `RemoteDNS*` — DNS для проксированного трафика.
- `DomesticDNS*` — DNS для прямого трафика (split DNS).
- `DnsHosts` — статические DNS-записи (например, чтобы DoH-сервер сам не резолвился через себя же).
- `FakeDNS` — виртуальные IP для доменов (Mihomo only).

### 11.B — Geofiles с оптимизацией через .sha256

Скачиваем `geoip.dat` и `geosite.dat` с GitHub (Loyalsoldier/v2ray-rules-dat).
Кладём в `%LOCALAPPDATA%\NemefistoVPN\geofiles\`.

**Алгоритм обновления:**
1. Скачиваем `geoip.dat.sha256` (64 hex-символа).
2. Сравниваем с сохранённым хешем.
3. Если совпадает — пропускаем скачивание `.dat` (экономия трафика 5–15 МБ).
4. Если нет — качаем `.dat`, сохраняем новый хеш.
5. Fallback: если `.sha256` недоступен, сравниваем `LastUpdated` из профиля.

**Опция `useChunkFiles: true`** (в перспективе для мобильных, на десктопе игнорируется): парсим protobuf-файл и оставляем только упомянутые в правилах теги. Хеш пересчитывается локально.

### 11.C — Autorouting vs Routing (два режима)

- **Routing** — статический профиль. Передаётся либо как base64 в заголовке `routing`, либо как ссылка `nemefisto://routing/onadd/{base64}`. Обновляется только при ручном перезапросе подписки.
- **Autorouting** — URL-источник, профиль скачивается отдельно и обновляется автоматически по интервалу. `sourceURL` сохраняется. В UI помечается иконкой облака.

**Заголовки подписки:**

| Заголовок | Формат | Назначение |
|---|---|---|
| `routing` | base64 / URL | Статический профиль маршрутизации |
| `autorouting` | URL | URL-источник профиля с периодическим обновлением |

**Интервалы автообновления (на выбор)**: 12 ч / 24 ч (default) / 3 дня / 7 дней.

**Приоритет источников** (если задано несколько):
1. Заголовок `autorouting`
2. Body-строка `://autorouting/...`
3. Заголовок `routing`
4. Body-строка `://routing/...` (base64)

### 11.D — Расширенные deep-links

Расширяем существующий обработчик `nemefisto://` командами:

#### Управление VPN
| Команда | Действие |
|---|---|
| `nemefisto://connect` или `nemefisto://open` | Подключить VPN |
| `nemefisto://disconnect` или `nemefisto://close` | Отключить VPN |
| `nemefisto://toggle` | Переключить состояние |
| `nemefisto://status` | Открыть приложение, показать статус |

#### Импорт конфигураций
| Команда | Что делает |
|---|---|
| `nemefisto://import/{data}` | Auto-detect: URL подписки или одиночная протокольная ссылка |
| `nemefisto://add/{url}` | Добавить подписку напрямую |
| `nemefisto://onadd/{url}` | Сокращённая форма (без автообновления) |

#### Маршрутизация
| Команда | Действие |
|---|---|
| `nemefisto://routing/add/{base64}` | Добавить routing-профиль |
| `nemefisto://routing/onadd/{base64}` | Добавить и сразу активировать |
| `nemefisto://routing/onadd/{url}` | Скачать одноразово (без автообновления) |
| `nemefisto://autorouting/add/{url}` | Скачать с автообновлением (не активирует) |
| `nemefisto://autorouting/onadd/{url}` | Скачать, активировать, включить автообновление |

**Query-параметр `?data={base64}`** поддерживается как альтернатива path-сегменту (для длинных payload'ов и совместимости).

**GitHub-конвертация**: `https://github.com/.../blob/main/...` автоматически переписывается на `https://raw.githubusercontent.com/.../main/...` чтобы получить сырой контент.

### 11.E — Спец-строки в теле подписки

Универсальный парсер дополнительно распознаёт строки вида:

```
://autorouting/onadd/https://example.com/profile.json
://autorouting/add/https://example.com/profile.json
://routing/onadd/https://example.com/profile.json
://routing/onadd/{base64}
://routing/add/{base64}
#announce: текст объявления
#announce: base64:...
#profile-title: имя
#support-url: https://...
#profile-web-page-url: https://...
#announce-url: https://...
#profile-update-interval: 6
```

Это позволяет панели с примитивным API (только тело без заголовков) всё равно
управлять клиентом.

### 11.F — Применение правил к движкам

- **Mihomo**: маппим напрямую — `DirectSites/DirectIp` → `DIRECT`, `ProxySites/ProxyIp` → выбранная proxy-group, `BlockSites/BlockIp` → `REJECT`. `geosite:`/`geoip:` нативно поддерживаются.
- **Xray**: транслируем в `routing.rules[]` с `outboundTag: "direct" | "proxy" | "block"`. `geosite:`/`geoip:` загружаются из локальных `.dat` файлов через `assets`-каталог.

### Этапы реализации

- **11.A** — модель `RoutingProfile` (Rust + TS типы), парсинг JSON, валидация.
- **11.B** — менеджер geofiles: скачивание, кеширование, проверка `.sha256`, фоновые обновления.
- **11.C** — стор для routing-профилей, разделение routing vs autorouting, scheduler автообновления.
- **11.D** — расширение deep-link обработчика всеми новыми командами.
- **11.E** — расширение парсера подписок спец-строками.
- **11.F** — генерация правил маршрутизации в конфигах Xray/Mihomo.
- **11.G** — UI: вкладка «маршрутизация» в Settings, импорт/удаление профилей, индикатор «обновлено N часов назад», кнопка ручного refresh.

---

## Этап 12 — Полировка UX (предложения сообщества)

**Цель**: серия мелких UX-улучшений, отобранных из обратной связи
пользователей INCY/Happ/v2rayTun. Каждый пункт — независимый,
реализуется быстро (15–60 минут), повышает повседневный комфорт.

### 12.A — Сброс настроек без удаления подписки
В Settings → внизу две раздельные кнопки:
- «сбросить настройки» — `settingsStore.reset()`, не трогает
  `subscriptionStore` (URL подписки и кеш серверов остаются);
- «удалить всё» (с двойным confirm) — settings + subscription +
  выбранный сервер.

### 12.B — Дата последнего обновления подписки
В `subscriptionStore` сохранять `lastFetchedAt: number` (unix-ts) после
успешного `fetchSubscription`, персистить в localStorage. Показывать
в `SubscriptionMeta`-плашке: «обновлено 4 ч. назад» рядом с трафиком.
Используем относительный формат («5 мин назад», «2 ч назад», «3 дн
назад», «давно» если >7 дней).

### 12.C — Фильтр серверов в drawer
Поисковая строка сверху drawer + чипы протоколов (vless / vmess /
trojan / hy2 / tuic / wg / socks). Клик по чипу — show only этот
тип. Клик ещё раз — снимает фильтр. Поиск по `name` (case-insensitive,
по подстроке). Фильтрация на клиенте, без бэкенда. Сильно нужно при
подписках с >50 серверами.

### 12.D — Backup/restore настроек через deep-link
Закрывает реальную боль: «настроил себе → отправил жене ссылкой» /
«переехал на новый комп».

Реализация:
- `nemefisto://export` — открывает file-save диалог, сохраняет JSON
  с settings + URL подписки + appRules (без кеша серверов и HWID).
- `nemefisto://import-from-url/{url}` — скачать JSON по ссылке.
- `nemefisto://import/{base64}` — импорт из inline base64 (для
  коротких ссылок).
- Перед применением — модалка с превью изменений (что заменится),
  кнопки «применить» / «отмена».
- Whitelist полей: тема, фон, пресет, button-style, autoRefresh*,
  refresh/ping/connectOnOpen, sort, allowLan, anti-DPI группы,
  app-rules, URL подписки. **Без HWID, без localStorage-флагов
  туторила, без dismissed-set объявлений.**

### 12.E — Маскировка имени TUN-адаптера
В Windows имя адаптера (`Get-NetAdapter`) видно сторонним
приложениям через `GetAdaptersAddresses`. Шпионы типа МАХ / ВК /
Госуслуги / OZON / WB детектят VPN по имени `nemefisto-<pid>` или
по диапазону `198.18.0.0/15`.

В Settings → toggle «маскировка TUN» (off по умолчанию). Если on —
имя адаптера выбирается случайно из набора:
`wlan{99..199}` / `Local Area Connection {N}` / `Ethernet {N}`.
В payload запоминаем настоящее имя для своего lookup.

⚠️ Это первый layer защиты. Имя — самый дешёвый детект-вектор;
шпионы серьёзнее смотрят ещё на IP-диапазон TUN-интерфейса
(сейчас `198.18.0.1/15`). Полная маскировка потребует ещё рандомного
IP-range, но начинаем с имени.

### Этапы реализации

- **12.A** — две кнопки в Settings, тривиально (~15 мин).
- **12.B** — поле в store + персистенс + UI-строка (~20 мин).
- **12.C** — search-input + chips в drawer + filter-логика (~30–45 мин).
- **12.D** — экспорт/импорт + deep-link обработчик + модалка превью
  (~1 час).
- **12.E** — генератор имён + интеграция с tun2socks/helper +
  Settings-toggle (~30 мин).

---

## Этап 13 — Что отличает «крепкий клиент» от «топового»

**Цель**: фичи которые формируют разницу между «работающим VPN-клиентом»
и «приложением которое хочется рекомендовать». Все пункты независимы и
могут реализовываться параллельно с основным roadmap. Расположены по
value/effort.

### 13.A — Системный трей + автоминимизация

**Must-have** для VPN-клиента, ожидаемое поведение.

- Иконка в трее с цвет-статусом (red = stopped/error, yellow = busy,
  green = running). Анимация при переходных состояниях.
- Контекстное меню трея: connect/disconnect, быстрая смена сервера
  (топ-5 по пингу), открыть main, выход.
- Закрытие окна (X) → сворачиваем в трей, не выходим из приложения.
  Опционально настраивается («close button: minimize / quit»).
- Двойной клик по иконке трея → восстановить главное окно.

Реализация: **`tauri-plugin-tray`** (в Tauri 2 — встроено в core API
через `app.tray()`). Win32-специфичных вызовов не нужно.

### 13.B — Leak-test после connect

После успешного `connect` (или по кнопке в Settings) делаем HTTP-запрос
к `https://api.ipify.org?format=json` через системный прокси, парсим
IP, опционально через GeoIP-API получаем страну. Показываем toast:
«твой IP сейчас: 203.0.113.x — 🇩🇪 Германия».

Зачем: подтверждает что VPN реально работает. Без этого пользователь
полагается на веру. Ставит планку доверия к клиенту.

Опционально: до/после диалог при первом подключении («твой IP был:
X.X.X.X (РФ) → стал: Y.Y.Y.Y (DE)»).

### 13.C — Smart auto-failover

Во время сессии мониторим выбранный сервер: пинг каждые 30 сек, или
ловим TCP-fail в логах Xray. Если пинг > 3000мс на 30 сек подряд или
TCP-disconnect → автоматически переключаемся на следующий по пингу.
Toast: «сервер DE-Fast не отвечает, переключился на NL-Stable».

Не работает если пользователь явно выбрал конкретный сервер (опция
«не переключать автоматически» в settings). Включается только при
выборе сервера через «авто-выбор лучшего» (этап 7-хвост).

### 13.D — Kill switch (часть этапа 6)

WFP-фильтр (Windows Filtering Platform) блокирует весь не-VPN трафик
когда VPN disconnect. Защита от утечек при reconnect / краше Xray /
смене сети.

- Опция в Settings (off по умолчанию).
- Whitelist для LAN (можно выключить блокировку 192.168.*).
- Реализация через `windivert` или native WFP API (через crate
  `windows-rs`, `Win32::NetworkManagement::WindowsFilteringPlatform`).
- Helper-сервис должен поднимать/убирать WFP-правила (нужны admin-
  права).

### 13.E — История сессий

Локальный лог connect/disconnect события: timestamp, сервер, режим
(proxy/tun), длительность сессии, причина disconnect (user / failover
/ error). Хранится в SQLite файле `%LOCALAPPDATA%\NemefistoVPN\
history.db` (можно через `rusqlite`).

UI: вкладка «история» в Settings или отдельный экран. Сортировка по
времени, фильтр по серверу. Полезно для диагностики и просто
интересно пользователю.

### 13.F — Speed-test встроенный

Кнопка в Settings → «измерить скорость через VPN». Скачивает 5–10 МБ
с известного быстрого CDN (Cloudflare speedtest endpoints или
`speed.cloudflare.com`), показывает Mbps.

Опционально: автоматически раз в неделю на всех серверах подписки
(в фоне) для smart-сортировки. Полученные значения сохраняются
вместе с пингами в `subscriptionStore`.

### 13.G — Per-app routing через WFP (Windows-native, без Mihomo)

Mihomo на Windows реализует `PROCESS-NAME` через `find-process-mode:
always` — это **полл текущих процессов раз в N секунд**, медленно и не
ловит короткоживущие процессы.

Альтернатива: **WFP callout-driver** перехватывает соединения по
`process-id` напрямую от ядра Windows. Точно, мгновенно, работает с
обоими движками (Xray и Mihomo).

- Реализация серьёзная (~1 неделя): нужен kernel-mode driver или
  user-mode WFP filter с callout. Crate `windivert-rs` упрощает но
  требует подписи драйвера.
- Альтернативный путь: использовать готовый `WinDivert` который уже
  подписан Microsoft.
- Серьёзно отличает приложение от конкурентов на Windows.

После 8.D считается «достаточно хорошо», 13.G — «идеально».

### 13.H — WebRTC + DNS leak protection

**DNS leak**: monitor DNS-traffic на интерфейсах (через `pktmon` или
WFP), assert что все DNS-запросы идут только через VPN. Иначе toast
с предупреждением + ссылкой «как починить DNS» (Settings → DNS
override).

**WebRTC**: на странице первого запуска / в Settings секция «утечки
браузера»: текстовая инструкция + deep-link на `about:flags` /
`chrome://flags` для отключения WebRTC. Браузерное расширение мы
делать не будем — это вне scope нативного клиента.

### 13.I — Bandwidth-метр в реальном времени

Маленький график (или текстовый индикатор) в верхнем углу окна:
↑ 1.2 МБ/с / ↓ 5.4 МБ/с. Обновление 1 Гц. Получаем через Windows
Performance Counters (`PdhCollectQueryData` для нашего interface)
или из tun2socks логов.

Не ставит планку (есть в любом VPN), но добавляет ощущение «живого»
приложения. Низкий effort.

### 13.J — Session passcode (Windows Hello)

Опция: при запуске приложения требовать аутентификацию через
**Windows Hello** (face / pin / fingerprint). Crate `windows-rs`,
`UserConsentVerifier`. Полезно для общих компьютеров.

Toggle в Settings → «требовать аутентификацию при запуске».

### 13.K — Hysteria2 obfs salamander (anti-DPI прямо в протоколе)

Hysteria2 поддерживает obfuscation `salamander` с паролем — пакеты
маскируются под случайный мусор, DPI не может определить QUIC. Это
встраивается в outbound:

```json
{
  "protocol": "hysteria2",
  "settings": {
    "password": "...",
    "obfs": { "type": "salamander", "password": "..." }
  }
}
```

Парсер уже сохраняет `obfs` / `obfs-password` из URI → достаточно
учесть в `build_hysteria2()` (после 8.A.1).

### 13.L — Mihomo built-in TUN-mode (альтернатива tun2socks)

Mihomo имеет встроенный TUN-режим с собственным userspace network
stack (gVisor). Не нужен отдельный tun2socks-процесс — Mihomo сам
поднимает TUN-интерфейс через WinTUN.

Плюсы: одна цепочка процессов вместо двух (Mihomo вместо
Xray + tun2socks), меньше точек отказа, проще архитектура.
Минусы: только когда выбран Mihomo (для Xray всё равно нужен tun2socks).

Альтернативный путь реализации в рамках 8.B. Решение принимается
при разработке 8.B.

### 13.M — SSID-based auto-mode (от koala-clash)

**Уникальная фича для путешественников.** Пользователь добавляет
«доверенные» Wi-Fi сети (домашний/рабочий) в whitelist; при подключении
к ним VPN автоматически выключается или переходит в `direct`-режим.
Уехал из дома — снова включается.

Реализация:
- Расширяем `network_watcher.rs`: помимо имени интерфейса читаем SSID
  через `netsh wlan show interfaces` (Windows) — парсим строку
  `SSID                   : <name>`. На macOS/Linux — отдельные команды,
  пока пропускаем (готовимся к портированию).
- При смене SSID emit-им событие `wifi-changed`, фронт принимает решение:
  если новый SSID в `trustedSsids` — disconnect (или `direct`); если
  ушли с trusted на unknown — auto-reconnect (если `autoConnectOnLeave`).
- Settings → секция «доверенные Wi-Fi» → список с add/remove + dropdown
  «при подключении к доверенной сети: ничего / отключить VPN /
  только заблокированные сайты».
- Effort: ~1 ч. Value: ⭐⭐⭐⭐⭐.

### 13.N — Global shortcuts (от koala-clash)

Системные горячие клавиши через `tauri-plugin-global-shortcut`:
- `Ctrl+Shift+V` — toggle connect/disconnect;
- `Ctrl+Shift+T` — переключить proxy↔TUN режим;
- `Ctrl+Shift+M` — показать/скрыть главное окно.

Каждая клавиша конфигурируется в Settings (input для записи комбинации,
toggle on/off). Effort: ~30 мин. Value: ⭐⭐⭐.

### 13.O — Floating window (от koala-clash)

Опциональное мини-окно 120×42 px, прозрачное, alwaysOnTop, skipTaskbar.
Показывает значок статуса + текущую скорость ↑/↓. Включается toggle'ом
в Settings. Drag-handle для перетаскивания, позиция персистится в
localStorage. Хорошо работает в паре с **13.I** bandwidth-метром
(одна реализация — два места отображения).

Реализация: второе Tauri-окно с `decorations: false, transparent: true,
always_on_top: true, skip_taskbar: true`. Effort: ~1.5–2 ч. Value: ⭐⭐⭐.

### 13.P — Слияние нескольких подписок (от Prizrak-Box)

Пользователь может добавить 2-5 подписок одновременно; клиент сливает
все серверы в один список с тегом источника. Полезно тем, у кого
запасные подписки на случай блокировки основной.

- `subscriptionStore` хранит массив `Subscription[]` вместо одной;
- При импорте новой — добавляем, не заменяем;
- Каждый сервер помечается `source: <subscription-id>`;
- В UI server-list — group by source с заголовками-разделителями.

Реализация: средний рефакторинг store. Effort: ~3 ч. Value: ⭐⭐⭐.
Имеет смысл после 8.B (Mihomo) — у кого подписки на разные движки.

### 13.Q — Auto-grouping правил для пустых подписок (от Prizrak-Box)

Если подписка не задаёт routing (нет заголовка `routing`/`autorouting`,
нет шаблона X-Nemefisto-Routes), применяем встроенный «минимальный»
шаблон: `geosite:ru` + `geoip:ru` → DIRECT, всё остальное → PROXY,
рекламные домены → BLOCK. Опция в Settings → «авто-применять
минимальные правила РФ» (off по умолчанию для совместимости).

Effort: ~1.5 ч (после этапа 11). Value: ⭐⭐⭐.

### 13.R — TUN-only «strict mode» (от dropweb)

Toggle в Settings скрывает выбор proxy-режима, оставляет только TUN.
Для параноиков, которые не хотят оставлять SOCKS-прокси на loopback
(пусть и с рандомным портом). UX-минимализм + чуть строже
безопасность. Effort: ~30 мин. Value: ⭐⭐.

### Приоритет внутри этапа 13

| Пункт | Value | Effort | Когда |
|---|---|---|---|
| 13.A системный трей | ⭐⭐⭐⭐⭐ | средний | сразу после 12 |
| 13.B leak-test | ⭐⭐⭐⭐⭐ | низкий | вместе с 13.H (анти-leak блок) |
| **13.M SSID auto-mode** | ⭐⭐⭐⭐⭐ | средний | quick-win |
| 13.C smart failover | ⭐⭐⭐⭐ | средний | после 13.A/B |
| 13.D kill switch | ⭐⭐⭐⭐ | высокий | в этапе 6 |
| **13.N global shortcuts** | ⭐⭐⭐ | низкий | quick-win |
| **13.O floating window** | ⭐⭐⭐ | средний | UX-полировка |
| **13.P слияние подписок** | ⭐⭐⭐ | высокий | после 8.B |
| **13.Q auto-grouping rules** | ⭐⭐⭐ | средний | после 11 |
| 13.K hy2 salamander | ⭐⭐⭐ | низкий | после 8.A.1 (готово) |
| 13.L Mihomo TUN | ⭐⭐⭐ | средний | в этапе 8.B |
| 13.E история | ⭐⭐⭐ | низкий | в любой момент |
| 13.F speed-test | ⭐⭐⭐ | средний | после 7-хвоста |
| 13.I bandwidth | ⭐⭐ | низкий | UX-полировка |
| 13.J Windows Hello | ⭐⭐ | низкий | UX-полировка |
| **13.R TUN-only strict** | ⭐⭐ | низкий | UX-полировка |
| 13.G WFP per-app | ⭐⭐⭐⭐ | очень высокий | долгосрочно |
| 13.H DNS/WebRTC leak | ⭐⭐⭐ | средний | после 13.D |

---

## Дорожная карта по сессиям

### Готово
- **8.A** + **8.A.1** — универсальный парсер подписок + hotfix Xray
  (hy2/wg/xhttp/httpupgrade).
- **8.B** — Mihomo как второй sidecar (TUIC/AnyTLS/Mieru-ready).
  YAML-конфиг через `config::mihomo_config::build()`, `mixed-port`
  один на SOCKS5+HTTP, DNS включён всегда против leak'ов. UI: Settings
  → секция «движок» с server-driven override-бейджем; engine-бейджи
  X/M на server-cards для эксклюзивных протоколов; предупреждение в
  anti-DPI секции что фрагментация/шумы — Xray-only.
  - **Per-engine UA + auto-refetch + smart-reconnect** при смене движка:
    Xray шлёт `Happ/2.7.0` (xray-json с routing), Mihomo шлёт
    `clash-verge/v2.0.0` (clash YAML с rules). Смена движка =
    disconnect → fetchSubscription → reconnect атомарно.
  - **Нормализация xray-json** для совместимости подписок: если в JSON
    нет custom routing.rules — выдаём обычный ProxyEntry с `engine_compat = both`.
    Если есть routing.rules — оставляем как `xray-json` (engine_compat=xray),
    через `patch_xray_json` сохраняем split-routing.
- **8.D** — per-process routing через Mihomo `PROCESS-NAME` matcher
  с `find-process-mode: always`. UI: Settings → секция «правила
  приложений» с add/remove + цветные бейджи proxy/direct/block.
  Xray-ветка игнорирует правила (на Windows нет нативной поддержки —
  будет в 13.G через WFP).
- **9.D** + **9.F** + **9.G** + **9.H** — proxy-backup/restore,
  уникальное имя TUN, SOCKS5 inbound auth для TUN/LAN, рандомизация
  портов inbound `[30000, 60000)`.
- **10** — anti-DPI обвязка (фрагментация, шумы, DoH-resolve).
- **12.E** — маскировка имени TUN-адаптера.
- **13.K** — Hysteria2 obfs salamander.
- **Этап 6** — Credential Manager + autostart (Task Scheduler) +
  network watcher + kill switch (firewall-вариант, 13.D).
- **Этап 8.C** — server-driven UX (X-Nemefisto-* + стандартные заголовки
  подписки) с override-логикой и UI-бейджами «из подписки».

### Следующая сессия — **Variant A (~2.5 ч)** [рекомендация]
- **13.A** системный трей (~1.5 ч) — иконка статуса, контекстное меню,
  «закрытие → свернуть в трей», double-click → восстановить окно.
  `tauri-plugin-tray` встроен в core API через `app.tray()`.
- **13.B** leak-test после connect (~45 мин) — HTTP-запрос к
  `api.ipify.org` через системный прокси, опциональный GeoIP-резолв,
  toast «твой IP сейчас X (страна)». Ставит планку доверия к клиенту.

После этого приложение перестаёт ощущаться прототипом — есть трей и
визуальное подтверждение работы VPN.

### Альтернативы

**Variant B — UX-полировка из этапа 12 (~2.5 ч)**:
- **12.A** сброс настроек без удаления подписки (~15 мин);
- **12.C** фильтр серверов в drawer (~45 мин);
- **12.D** backup/restore через deep-link (~1 ч);
- **9.B** детект конкурирующих VPN-клиентов (~45 мин).

**Variant C — Mihomo-движок (~3.5 ч)** [готово]:
- **8.B** Mihomo как второй sidecar + UI-селект движка ✅;
- **8.D** per-process routing через PROCESS-NAME matcher ✅;
- опционально **13.L** Mihomo built-in TUN — отложено: требует
  рефакторинга helper-сервиса под запуск Mihomo как SYSTEM (WinTUN на
  Windows требует админ для CreateAdapter). Текущая схема Mihomo+tun2socks
  через helper работает прекрасно.

**Variant D — Routing-профили (~4–5 ч, 2 сессии)**:
- **11.A**…**11.G** — geofiles, autorouting, deep-links, UI.

**Variant E — quick wins (~30–45 мин)**:
- одна из коротких задач: 13.B leak-test, или 12.A+9.B, или 7-хвост
  «авто-выбор лучшего сервера».

**Variant F — фишки от конкурентов (~2.5 ч)** [новое]:
- **13.M** SSID auto-mode (~1 ч) — уникальная фича для путешественников;
- **13.N** global shortcuts (~30 мин) — Ctrl+Shift+V toggle VPN;
- **13.B** leak-test (~45 мин) — toast «твой IP сейчас X (страна)».

**Variant G — Floating window (~2 ч)**:
- **13.O** мини-окно поверх всего со статусом и скоростью.

### Долгосрочно (когда дойдут руки)
- **8.D** per-process routing UI (нужен 8.B);
- **9.B / 9.C / 9.E** — закрытие conflict-protection остатков;
- **13.C** smart auto-failover, **13.E** история сессий, **13.F**
  speed-test, **13.I** bandwidth-метр, **13.J** Windows Hello;
- **13.G** WFP per-app routing — большой проект, серьёзный отрыв от
  конкурентов;
- **13.H** DNS/WebRTC leak protection;
- полноценный WFP-кill switch (13.D) вместо текущего firewall-варианта.

### Долги
- TUN 15-секундная задержка первого запроса.

### Идеи из сравнения с другими клиентами

**dropweb** (форк FlClashX, mihomo-only):
- Рандомизация портов — взяли (9.H, готово).
- TUN-only «strict mode» — записан как **13.R** (низкий приоритет).
- Mihomo-only — не наш путь (теряем REALITY/XHTTP оптимизации Xray).

**koala-clash** (Electron + mihomo, 551 ⭐):
- SSID-based auto-mode — записан как **13.M** (высокий приоритет).
- Global shortcuts — записан как **13.N** (низкий effort).
- Floating window — записан как **13.O** (средний приоритет).
- Multiple cores (stable+alpha) — будет в рамках 8.B.

**Prizrak-Box** (Vue + Wails, mihomo-only, 229 ⭐):
- Слияние нескольких подписок — записан как **13.P**.
- Auto-grouping правил для пустых подписок — записан как **13.Q**.
- Mieru протокол — после 8.B (только Mihomo).
- DNS rewrite forced — частично закрыт нашим DoH-resolve (10.C).
