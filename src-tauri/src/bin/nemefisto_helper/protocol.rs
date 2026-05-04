//! JSON-RPC протокол между helper-сервисом и Tauri-приложением.
//!
//! Каждое сообщение — одна строка JSON, заканчивается `\n`. Helper читает
//! строку, парсит как `Request`, выполняет, отвечает `Response` (тоже одна
//! строка JSON + `\n`). Соединение остаётся открытым — клиент может слать
//! несколько команд подряд.

use serde::{Deserialize, Serialize};

/// Версия wire-протокола helper-сервиса. Бампается каждый раз когда
/// меняется набор полей в `Request` / `Response` так, что старый helper
/// не сможет корректно обработать запрос от нового клиента.
///
/// История:
/// - 1: исходный набор (TunStart без extra_server_hosts).
/// - 2 (0.1.2): добавлено `TunStart.extra_server_hosts` — для
///   mihomo-passthrough bypass на все ноды подписки. Старый helper
///   игнорит поле и добавляет bypass только на primary, что приводит
///   к петле для других нод.
/// - 3 (0.1.2 / 13.L): добавлены `MihomoStart` / `MihomoStop` для
///   SYSTEM-spawn'а mihomo. Built-in TUN-режим mihomo требует админа
///   на `CreateAdapter` WinTUN — без помощи helper'а Tauri-main
///   (user-level) не может его запустить.
/// - 4 (0.1.2 / debug): bump для форсирования reinstall'а helper'а
///   на dev-машинах. Wire-формат не менялся, но мы добавили в
///   helper-tun.rs диагностический writeln в tun2socks.log который
///   показывает реальные значения socks-auth полей. Без bump'а
///   `ensure_running` не переустанавливает старый helper.
/// - 5/6 (0.1.2 / debug): итеративные bump'ы во время диагностики
///   tun2proxy auth (см. git log). Wire-формат не менялся.
/// - 7 (0.1.2 / sing-box миграция): добавлены `SingBoxStart` /
///   `SingBoxStop` для SYSTEM-spawn'а sing-box (по аналогии с
///   `MihomoStart`/`MihomoStop` из v3). Built-in TUN-режим sing-box
///   требует админа на `CreateAdapter` WinTUN — без помощи helper'а
///   Tauri-main (user-level) не может его запустить.
///
/// - 8 (0.1.2 / sing-box миграция Phase 5): выпилены `TunStart` /
///   `TunStop` — клиент больше не использует tun2proxy pipeline (TUN
///   делает sing-box или mihomo через built-in inbound). Helper-side
///   функции `start()`/`stop()` для tun2proxy тоже удалены, остался
///   только `cleanup_orphan_resources` для очистки legacy-адаптеров.
///
/// - 9 (0.1.3 / kill-switch fix): добавлено `KillSwitchEnable.expect_tun`.
///   Без него helper не знал нужен ли retry-поиск WinTUN-адаптера → в
///   TUN-режиме kill-switch поднимался без TUN allow-фильтра и блокировал
///   user-трафик. Bump форсит апгрейд helper'а на 0.1.3 — старый helper
///   v8 поднимет kill-switch БЕЗ TUN allow (старая версия игнорирует
///   новое поле, и `current_tun_interface_index` стабом возвращает None).
///
/// - 10 (14.D / IPv6 leak protection): добавлено
///   `KillSwitchEnable.force_disable_ipv6`. Если `true` — пропускаются
///   все v6 allow-фильтры (LAN, server, app-allow, TUN-interface), а
///   базовый block-all v6 остаётся → весь IPv6 outbound блокируется
///   пока VPN активен. Защита от утечек на dual-stack ISP. Bump чтобы
///   старый helper v9 не молча игнорил флаг (он поднял бы kill-switch
///   с дефолтными v6 allow'ами, и leak бы остался).
///
/// - 11 (0.3.1 / installer file-lock fix): добавлен `ShutdownHelper`.
///   Helper graceful self-stop через SCM (`SERVICE_CONTROL_STOP`).
///   Используется auto-updater'ом перед запуском NSIS installer'а:
///   helper закрывает свой `.exe`-handle, файл становится перезаписываемым
///   без админ-прав. Старый helper v10 не понимает команду — bump форсит
///   reinstall через UAC, после чего обновления станут гладкими.
///
/// Tauri-main сравнивает с `Response::Version.protocol_version` при
/// `ensure_running()` — если получил `<` (или 0 от helper'а без поля)
/// форсит uninstall+install через UAC, чтобы пользователь получил
/// помощь с дев-сборки или релиз-апгрейда без ручных шагов.
pub const PROTOCOL_VERSION: u32 = 11;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Health-check. Helper отвечает `Response::Pong`.
    Ping,
    /// Версия helper-а.
    Version,
    /// Включить kill switch (этап 13.D — настоящий WFP).
    ///
    /// `server_ips` — список IP-адресов VPN-сервера (Tauri-main делает
    /// DNS-резолв перед вызовом, потому что после включения kill-switch'а
    /// DNS-запросы вне VPN заблокированы).
    ///
    /// `allow_lan` — пускать ли локальную сеть (10/8, 172.16/12,
    /// 192.168/16, 169.254/16, fe80::/10, ff00::/8).
    ///
    /// `allow_app_paths` — абсолютные пути к нашим бинарям, которым
    /// разрешён исходящий трафик (sing-box.exe, mihomo.exe, helper.exe,
    /// vpn-client.exe). Без этого VPN-движок не сможет соединиться
    /// даже если IP сервера есть в server_ips.
    KillSwitchEnable {
        #[serde(default)]
        server_ips: Vec<String>,
        #[serde(default)]
        allow_lan: bool,
        #[serde(default)]
        allow_app_paths: Vec<String>,
        /// DNS leak protection (этап 13.D step B): блокировать весь
        /// :53/UDP+TCP трафик кроме явно разрешённых IP.
        #[serde(default)]
        block_dns: bool,
        /// IPv4 адреса VPN-DNS которые остаются разрешены при `block_dns`.
        /// В TUN-mode обычно [`198.18.0.1`] (наш TUN gateway).
        #[serde(default)]
        allow_dns_ips: Vec<String>,
        /// 13.S strict mode: НЕ давать общий allow_app для VPN-движков
        /// (xray/mihomo). Они смогут соединяться только на server_ips
        /// (через add_filter_allow_v4_addr_port_proto, который добавляется
        /// в любом случае). Direct outbound xray по `geosite:ru` будет
        /// блокирован — это и есть смысл strict mode.
        #[serde(default)]
        strict_mode: bool,
        /// 0.1.3 kill-switch fix: нужен ли retry-поиск активного
        /// WinTUN-адаптера для TUN allow-фильтра. `true` в TUN-режиме
        /// (sing-box/mihomo built-in TUN), `false` в proxy-режиме —
        /// чтобы не задерживать `enable()` на 5с впустую.
        #[serde(default)]
        expect_tun: bool,
        /// 14.D — принудительно блокировать весь IPv6-трафик пока
        /// VPN активен. Защита от утечек на dual-stack ISP, где часть
        /// трафика идёт по нативному v6 минуя v4-туннель. Если `true`,
        /// helper пропускает все v6 allow-фильтры (LAN, server, app,
        /// TUN-interface), оставляя только базовый block-all v6.
        /// Loopback `::1` остаётся разрешён — он не уходит в сеть.
        #[serde(default)]
        force_disable_ipv6: bool,
    },
    /// Выключить kill switch — drop'ает WFP DYNAMIC engine,
    /// все наши фильтры удаляются автоматически.
    KillSwitchDisable,
    /// Heartbeat для kill-switch watchdog (этап 13.D).
    /// Tauri-main шлёт каждые ~20 сек пока активен kill-switch. Если
    /// helper не получит heartbeat 60+ секунд — фильтры автоматически
    /// снимаются (страховка от зависания main-процесса).
    KillSwitchHeartbeat,
    /// Emergency cleanup всех WFP-фильтров с нашим provider GUID.
    /// Используется UI-кнопкой «аварийный сброс» — даже если main
    /// сейчас не имеет активного kill-switch state, удалит всё что
    /// потенциально зависло от прошлых сессий.
    KillSwitchForceCleanup,
    /// Cleanup orphan TUN-адаптеров (`nemefisto-*`) и half-default
    /// routes через `198.18.0.1`. Используется UI-кнопкой
    /// «восстановить сеть» когда видимо, что что-то осталось от
    /// упавшей сессии. Безопасно вызывать только когда VPN не активен.
    OrphanCleanup,
    /// 14.E: read-only проверка остатков WFP-фильтров от прошлой
    /// сессии. Возвращает `Response::WfpOrphan { has_orphan }` —
    /// фронт показывает в crash-recovery диалоге если true.
    /// Не destructive: только читает существование sublayer'а с
    /// нашим GUID.
    WfpQueryOrphan,
    /// 13.L: запустить mihomo как SYSTEM-процесс. Нужно для built-in
    /// TUN-режима — `CreateAdapter` WinTUN требует админа, и Tauri-main
    /// (user-level) не может его поднять напрямую.
    ///
    /// `mihomo_exe_path` / `config_path` / `data_dir` — абсолютные пути.
    /// Helper не знает где у Tauri лежат sidecar-binaries, поэтому
    /// получает их явно.
    ///
    /// stdout/stderr процесса перенаправляются в
    /// `C:\ProgramData\NemefistoVPN\mihomo.log` (помощник имеет туда
    /// SYSTEM-доступ; Tauri-main как user тоже может читать для
    /// диагностики).
    MihomoStart {
        config_path: String,
        mihomo_exe_path: String,
        data_dir: String,
    },
    /// 13.L: остановить SYSTEM-spawned mihomo. Идемпотентно: если
    /// helper не запускал mihomo — no-op.
    MihomoStop,
    /// sing-box миграция (v7): запустить sing-box как SYSTEM-процесс.
    /// Нужно для built-in TUN-режима — `CreateAdapter` WinTUN требует
    /// админа, и Tauri-main (user-level) не может его поднять напрямую.
    /// Семантически зеркалит `MihomoStart`.
    ///
    /// `singbox_exe_path` / `config_path` / `data_dir` — абсолютные пути.
    /// stdout/stderr процесса перенаправляются в
    /// `C:\ProgramData\NemefistoVPN\sing-box.log`.
    SingBoxStart {
        config_path: String,
        singbox_exe_path: String,
        data_dir: String,
    },
    /// sing-box миграция (v7): остановить SYSTEM-spawned sing-box.
    /// Идемпотентно: если helper не запускал sing-box — no-op.
    SingBoxStop,
    /// 0.3.1 / installer file-lock fix: graceful self-shutdown.
    /// Helper отвечает `Ok`, потом в фоновой задаче после короткой
    /// задержки (чтобы клиент успел получить ответ) сам себя стопит
    /// через SCM `SERVICE_CONTROL_STOP`. Helper работает под SYSTEM,
    /// имеет SERVICE_STOP rights на свой же сервис. После shutdown
    /// `.exe`-файл становится перезаписываемым без admin-прав → NSIS
    /// installer может обновить helper.exe в auto-update'е.
    ShutdownHelper,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Pong,
    Version {
        version: String,
        /// `PROTOCOL_VERSION` помощника. Если поле отсутствует в JSON
        /// (старый helper до 0.1.2) — десериализуется в 0, и
        /// Tauri-main триггерит reinstall.
        #[serde(default)]
        protocol_version: u32,
    },
    /// Успешный результат операции без полезной нагрузки.
    Ok,
    /// 14.E: ответ на `WfpQueryOrphan`. `has_orphan` — есть ли
    /// sublayer с нашим GUID в persistent WFP-store.
    WfpOrphan { has_orphan: bool },
    /// Ошибка с описанием.
    Error { message: String },
}

impl Response {
    pub fn err(msg: impl Into<String>) -> Self {
        Self::Error { message: msg.into() }
    }
}

pub const PIPE_NAME: &str = r"\\.\pipe\nemefisto-helper";
pub const SERVICE_NAME: &str = "NemefistoHelper";
pub const SERVICE_DISPLAY_NAME: &str = "Nemefisto VPN Helper";
pub const SERVICE_DESCRIPTION: &str = "Управление TUN-интерфейсом и системной маршрутизацией для Nemefisto VPN.";

#[cfg(test)]
mod tests {
    use super::*;

    /// 14.E: проверка JSON-формата `WfpQueryOrphan` request — должен быть
    /// `{"cmd":"wfp_query_orphan"}`. Если случайно изменить tag/rename_all
    /// на helper-стороне — тест поймает.
    #[test]
    fn wfp_query_orphan_request_serializes() {
        let req = Request::WfpQueryOrphan;
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"cmd":"wfp_query_orphan"}"#);
    }

    /// 14.E: `WfpOrphan` response с `has_orphan: true`.
    #[test]
    fn wfp_orphan_response_serializes() {
        let resp = Response::WfpOrphan { has_orphan: true };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"result":"wfp_orphan","has_orphan":true}"#);
    }

    /// Roundtrip: Request → JSON → Request. Если serde-теги совпадают,
    /// десериализация должна вернуть тот же variant.
    #[test]
    fn wfp_query_orphan_roundtrip() {
        let req = Request::WfpQueryOrphan;
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Request::WfpQueryOrphan));
    }
}
