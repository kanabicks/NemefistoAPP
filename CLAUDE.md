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

- **Xray** — текущий движок. Сильные стороны: Reality / Vision / XHTTP,
  низколатентный обход DPI.
- **Mihomo** (форк Clash Meta) — добавляется. Сильные стороны: Hysteria2 /
  Tuic / AnyTLS / WireGuard, гибкий routing, нативный per-process matcher.

Один пользователь использует **одно ядро на сессию**. Выбор:
1. Из заголовка `X-Nemefisto-Engine` подписки;
2. Иначе из настроек пользователя;
3. По дефолту Xray.

Серверы из подписки помечаются полем `engine_compat`. Если выбранное ядро
несовместимо с сервером — UI показывает предупреждение и предлагает
переключить движок.

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

- **8.A** — универсальный парсер подписок (vmess / trojan / ss / hy2 / tuic / wireguard + Mihomo YAML + полные Xray JSON).
- **8.B** — Mihomo как второй sidecar; UI-селект движка; helper-coordination для TUN с любым ядром.
- **8.C** — заголовки подписки (стандартные + Nemefisto) + override-логика + UI-бейджи «из подписки» + UI для `subscription-userinfo` / `announce` / `support-url` / `premium-url`.
- **8.D** — per-process routing (Mihomo-only) с UI-редактором правил.
- **8.E** — релизный NSIS-installer (см. ниже).

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
