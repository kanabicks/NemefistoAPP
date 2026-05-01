//! 11.B — Менеджер geofiles (`geoip.dat` / `geosite.dat`).
//!
//! Скачиваем v2ray-rules-dat файлы (Loyalsoldier) с GitHub Release. Чтобы
//! не качать одно и то же при каждом обновлении подписки — сначала
//! берём `.sha256` (64 hex-символа, ≤100 байт) и сравниваем с
//! сохранённым. Если совпадает — пропускаем `.dat` (5-15 МБ экономии).
//!
//! Файлы кешируются в `%LOCALAPPDATA%\NemefistoVPN\geofiles\`. Xray
//! находит их через env-var `XRAY_LOCATION_ASSET` (выставляется в
//! `vpn::xray::spawn` перед стартом sidecar). Mihomo получает прямой
//! путь через `geox-url:` в YAML-конфиге.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;

const DIR_NAME: &str = "NemefistoVPN";
const GEOFILES_SUBDIR: &str = "geofiles";

/// Размер response-body, выше которого мы считаем файл подозрительным
/// и обрываем download. v2ray-rules-dat обычно 5-15 МБ; ставим 50 МБ
/// с запасом, но останавливаемся если кто-то пытается засосать ГБ.
const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// Timeout на одно скачивание. `.dat` файлы ~10 МБ, на медленном
/// канале (1 МБит/с) загрузятся за ~1.5 минуты — поэтому 90 сек.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(90);

/// Каталог хранения geofiles (`%LOCALAPPDATA%\NemefistoVPN\geofiles\`).
pub fn geofiles_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(local).join(DIR_NAME).join(GEOFILES_SUBDIR))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(base.join(DIR_NAME).join(GEOFILES_SUBDIR))
    }
}

/// Состояние конкретного файла на диске.
#[derive(Debug, Serialize, Clone)]
pub struct GeofileStatus {
    /// Имя файла (например `"geoip.dat"`).
    pub filename: String,
    /// Существует ли файл.
    pub present: bool,
    /// Размер в байтах (0 если нет).
    pub size_bytes: u64,
    /// Известный sha256 (если сохранён в `.sha256` рядом).
    pub sha256: Option<String>,
}

/// Снимок состояния всех geofiles + путь.
#[derive(Debug, Serialize, Clone)]
pub struct GeofilesStatus {
    pub directory: String,
    pub geoip: GeofileStatus,
    pub geosite: GeofileStatus,
}

/// Результат `update_geofiles_if_changed` для UI / логов.
#[derive(Debug, Serialize, Clone)]
pub struct UpdateReport {
    pub geoip_updated: bool,
    pub geoip_skipped_unchanged: bool,
    pub geosite_updated: bool,
    pub geosite_skipped_unchanged: bool,
    pub errors: Vec<String>,
}

/// Прочитать текущее состояние без обращения к сети.
pub fn status() -> GeofilesStatus {
    let dir = geofiles_dir().unwrap_or_else(|| PathBuf::from("."));
    let geoip = file_status(&dir, "geoip.dat");
    let geosite = file_status(&dir, "geosite.dat");
    GeofilesStatus {
        directory: dir.to_string_lossy().into_owned(),
        geoip,
        geosite,
    }
}

fn file_status(dir: &Path, name: &str) -> GeofileStatus {
    let path = dir.join(name);
    let (present, size) = match std::fs::metadata(&path) {
        Ok(m) => (m.is_file(), m.len()),
        Err(_) => (false, 0),
    };
    let sha_path = dir.join(format!("{name}.sha256"));
    let sha256 = std::fs::read_to_string(sha_path)
        .ok()
        .and_then(|s| {
            let trimmed = s.trim().to_lowercase();
            // sha256 файлы Loyalsoldier'а имеют формат `<hex>  <filename>`,
            // нам нужен только первый whitespace-separated токен.
            trimmed
                .split_whitespace()
                .next()
                .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
                .map(String::from)
        });
    GeofileStatus {
        filename: name.to_string(),
        present,
        size_bytes: size,
        sha256,
    }
}

/// Обновить оба geofile'а если их `.sha256` изменился относительно
/// сохранённого. Если сохранённого нет — качает `.dat` всегда.
///
/// Не падает на одной ошибке — каждый файл независимый, ошибки
/// аккумулируются в `UpdateReport.errors`.
///
/// `geoip_url` и `geosite_url` могут быть пустыми — тогда соответствующий
/// файл просто не трогаем (нечего качать).
///
/// Reqwest без proxy: всегда **direct** через physic. Это критично —
/// если URL пытаются скачать через VPN-туннель с автообновлением
/// заблокированного в стране ресурса, мы попадаем в loop. Direct
/// обходит проблему.
pub async fn update_geofiles_if_changed(geoip_url: &str, geosite_url: &str) -> UpdateReport {
    let mut report = UpdateReport {
        geoip_updated: false,
        geoip_skipped_unchanged: false,
        geosite_updated: false,
        geosite_skipped_unchanged: false,
        errors: Vec::new(),
    };

    let dir = match geofiles_dir() {
        Some(d) => d,
        None => {
            report.errors.push("не удалось определить путь geofiles".to_string());
            return report;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        report.errors.push(format!("create_dir_all: {e}"));
        return report;
    }

    // Reqwest клиент без системного прокси.
    let client = match build_no_proxy_client() {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!("reqwest client: {e}"));
            return report;
        }
    };

    if !geoip_url.is_empty() {
        match update_one(&client, geoip_url, &dir, "geoip.dat").await {
            Ok(true) => report.geoip_updated = true,
            Ok(false) => report.geoip_skipped_unchanged = true,
            Err(e) => report.errors.push(format!("geoip: {e}")),
        }
    }
    if !geosite_url.is_empty() {
        match update_one(&client, geosite_url, &dir, "geosite.dat").await {
            Ok(true) => report.geosite_updated = true,
            Ok(false) => report.geosite_skipped_unchanged = true,
            Err(e) => report.errors.push(format!("geosite: {e}")),
        }
    }

    report
}

fn build_no_proxy_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(format!("Nemefisto/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("сборка reqwest client")
}

/// Скачивает один файл если sha256 изменился.
/// Возвращает `Ok(true)` если файл был обновлён, `Ok(false)` если sha
/// не изменился (skip).
async fn update_one(
    client: &reqwest::Client,
    dat_url: &str,
    dir: &Path,
    filename: &str,
) -> Result<bool> {
    let sha_url = format!("{dat_url}.sha256sum");
    let sha_url_alt = format!("{dat_url}.sha256");
    let dat_path = dir.join(filename);
    let sha_path = dir.join(format!("{filename}.sha256"));

    // Шаг 1: качаем .sha256(.sum). Loyalsoldier release использует
    // суффикс `.sha256sum`, но некоторые миррорят как `.sha256` —
    // пробуем оба.
    let remote_sha = match fetch_text(client, &sha_url).await {
        Ok(s) => Some(s),
        Err(_) => fetch_text(client, &sha_url_alt).await.ok(),
    };

    let remote_sha_clean = remote_sha.as_ref().and_then(|s| {
        s.trim()
            .split_whitespace()
            .next()
            .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|h| h.to_lowercase())
    });

    // Шаг 2: сравним с сохранённым.
    let local_sha: Option<String> = std::fs::read_to_string(&sha_path)
        .ok()
        .and_then(|s| {
            s.trim()
                .split_whitespace()
                .next()
                .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
                .map(|h| h.to_lowercase())
        });

    if let (Some(remote), Some(local)) = (remote_sha_clean.as_ref(), local_sha.as_ref()) {
        if remote == local && dat_path.is_file() {
            return Ok(false); // unchanged → skip
        }
    }

    // Шаг 3: скачиваем .dat. Если файл подозрительно большой — bail.
    let bytes = fetch_bytes(client, dat_url, MAX_DOWNLOAD_BYTES).await?;

    // Шаг 4: проверяем sha256 если был получен с сервера.
    if let Some(expected) = &remote_sha_clean {
        let actual = sha256_hex(&bytes);
        if actual.to_lowercase() != *expected {
            bail!(
                "{filename}: sha256 mismatch — server: {expected}, downloaded: {actual}"
            );
        }
    }

    // Шаг 5: атомарная запись через временный файл + rename.
    let tmp_path = dat_path.with_extension("dat.tmp");
    std::fs::write(&tmp_path, &bytes)
        .with_context(|| format!("write tmp {filename}"))?;
    std::fs::rename(&tmp_path, &dat_path)
        .with_context(|| format!("rename tmp → {filename}"))?;

    if let Some(remote) = &remote_sha_clean {
        let _ = std::fs::write(&sha_path, format!("{remote}\n"));
    } else {
        // Сервер sha не прислал — посчитаем сами и сохраним для
        // следующих сравнений (если апстрим начнёт отдавать sha).
        let computed = sha256_hex(&bytes);
        let _ = std::fs::write(&sha_path, format!("{computed}\n"));
    }

    Ok(true)
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await.context("HTTP")?;
    if !resp.status().is_success() {
        bail!("HTTP {} для {url}", resp.status());
    }
    resp.text().await.context("read body").map(|s| s)
}

async fn fetch_bytes(client: &reqwest::Client, url: &str, max: u64) -> Result<Vec<u8>> {
    let resp = client.get(url).send().await.context("HTTP")?;
    if !resp.status().is_success() {
        bail!("HTTP {} для {url}", resp.status());
    }
    // Pre-check через Content-Length: если сервер заявил больше max — bail
    // не качая. Если Content-Length нет (chunked transfer) — качаем целиком,
    // потом проверяем размер post-fact.
    if let Some(len) = resp.content_length() {
        if len > max {
            bail!(
                "файл {url}: Content-Length {len} > max {max} — отказ"
            );
        }
    }
    let bytes = resp.bytes().await.context("read body")?;
    if (bytes.len() as u64) > max {
        bail!("файл {url}: размер {} > max {max}", bytes.len());
    }
    Ok(bytes.to_vec())
}

/// SHA-256 хеш в lowercase hex. Используем только для verify-after-download
/// (не security-critical: это integrity check скачанного файла).
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let h = sha256_hex(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn empty_url_does_nothing() {
        // unit-тест без сети: пустые URL → empty UpdateReport (oba false).
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(update_geofiles_if_changed("", ""));
        assert!(!r.geoip_updated);
        assert!(!r.geosite_updated);
        assert!(!r.geoip_skipped_unchanged);
        assert!(!r.geosite_skipped_unchanged);
        assert!(r.errors.is_empty());
    }
}
