//! 11.C — Хранилище routing-профилей + scheduler авто-обновления.
//!
//! Профили хранятся как JSON-файлы в `%LOCALAPPDATA%\NemefistoVPN\
//! routing-profiles\<id>.json`. Один профиль может быть **активным** —
//! его правила применяются при connect (см. xray_config / mihomo_config).
//!
//! Autorouting профили имеют URL-источник; фоновый scheduler каждые
//! `interval_hours` качает свежий JSON и обновляет профиль (если хеш
//! содержимого изменился). Static профили обновляются только вручную
//! (через deep-link или UI).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Notify};

use super::geofiles;
use super::routing_profile::{ProfileSource, RoutingProfile, RoutingProfileEntry};

const DIR_NAME: &str = "NemefistoVPN";
const STORE_SUBDIR: &str = "routing-profiles";
const ACTIVE_FILE: &str = "active.txt";

/// Каталог хранения профилей.
fn store_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(local).join(DIR_NAME).join(STORE_SUBDIR))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(base.join(DIR_NAME).join(STORE_SUBDIR))
    }
}

fn entry_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn active_path(dir: &Path) -> PathBuf {
    dir.join(ACTIVE_FILE)
}

/// Снимок состояния стора для UI / других модулей. Берёт snapshot
/// под mutex'ом, потом lock освобождён — async-операции (download
/// и т.п.) не держат stor заблокированным.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoutingStoreSnapshot {
    pub entries: Vec<RoutingProfileEntry>,
    pub active_id: Option<String>,
}

/// Хранилище профилей с persist'ом на диск.
///
/// Tauri-state: оборачивается в `RoutingStoreState(Arc<Mutex<Self>>)`.
/// Mutex синхронный (std::sync::Mutex) — все операции быстрые
/// (read/write одного JSON-файла), async тут не нужен.
#[derive(Debug, Default)]
pub struct RoutingStore {
    entries: Vec<RoutingProfileEntry>,
    active_id: Option<String>,
}

impl RoutingStore {
    /// Загружает все профили с диска. Используется один раз при старте app.
    pub fn load() -> Self {
        let Some(dir) = store_dir() else {
            return Self::default();
        };
        if !dir.exists() {
            return Self::default();
        }
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&p) {
                    if let Ok(e) = serde_json::from_str::<RoutingProfileEntry>(&text) {
                        entries.push(e);
                    } else {
                        eprintln!(
                            "[routing-store] skip битый профиль {}",
                            p.display()
                        );
                    }
                }
            }
        }
        let active_id = std::fs::read_to_string(active_path(&dir))
            .ok()
            .and_then(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            // Активный должен существовать в entries — иначе сбрасываем.
            .filter(|id| entries.iter().any(|e| &e.id == id));

        Self { entries, active_id }
    }

    pub fn snapshot(&self) -> RoutingStoreSnapshot {
        RoutingStoreSnapshot {
            entries: self.entries.clone(),
            active_id: self.active_id.clone(),
        }
    }

    /// Возвращает активный профиль (если есть и id'тся в списке).
    pub fn active(&self) -> Option<&RoutingProfileEntry> {
        let id = self.active_id.as_ref()?;
        self.entries.iter().find(|e| &e.id == id)
    }

    /// Добавить новую запись. Возвращает её id.
    pub fn add(&mut self, profile: RoutingProfile, source: ProfileSource) -> Result<String> {
        let mut entry = RoutingProfileEntry::new(profile, source);
        // Если это autorouting — last_fetched_at = сейчас (мы только что
        // скачали JSON извне).
        if matches!(entry.source, ProfileSource::Autorouting { .. }) {
            entry.last_fetched_at = unix_now();
        }
        let id = entry.id.clone();
        self.entries.push(entry);
        self.persist_entry(&id)?;
        Ok(id)
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == before {
            return Err(anyhow!("профиль с id {id} не найден"));
        }
        if self.active_id.as_deref() == Some(id) {
            self.active_id = None;
            self.persist_active()?;
        }
        let dir = store_dir().ok_or_else(|| anyhow!("нет path"))?;
        let _ = std::fs::remove_file(entry_path(&dir, id));
        Ok(())
    }

    pub fn set_active(&mut self, id: Option<&str>) -> Result<()> {
        match id {
            Some(i) => {
                if !self.entries.iter().any(|e| e.id == i) {
                    return Err(anyhow!("профиль {i} не найден"));
                }
                self.active_id = Some(i.to_string());
            }
            None => {
                self.active_id = None;
            }
        }
        self.persist_active()
    }

    /// Обновить содержимое профиля (например после refresh autorouting).
    pub fn update_profile(
        &mut self,
        id: &str,
        new_profile: RoutingProfile,
    ) -> Result<()> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("профиль {id} не найден"))?;
        entry.profile = new_profile;
        entry.last_fetched_at = unix_now();
        self.persist_entry(id)
    }

    fn persist_entry(&self, id: &str) -> Result<()> {
        let dir = store_dir().ok_or_else(|| anyhow!("нет path"))?;
        std::fs::create_dir_all(&dir).context("create store dir")?;
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("entry {id} not found"))?;
        let json = serde_json::to_string_pretty(entry).context("serialize entry")?;
        std::fs::write(entry_path(&dir, id), json).context("write entry")?;
        Ok(())
    }

    fn persist_active(&self) -> Result<()> {
        let dir = store_dir().ok_or_else(|| anyhow!("нет path"))?;
        std::fs::create_dir_all(&dir).context("create store dir")?;
        let path = active_path(&dir);
        match self.active_id.as_deref() {
            Some(id) => std::fs::write(path, id).context("write active")?,
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Обёртка для Tauri State.
pub struct RoutingStoreState {
    pub inner: Arc<Mutex<RoutingStore>>,
    /// Notify используется чтобы сразу разбудить scheduler-loop при
    /// добавлении/изменении autorouting профиля (без ожидания tick'а).
    pub wake: Arc<Notify>,
}

impl RoutingStoreState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RoutingStore::load())),
            wake: Arc::new(Notify::new()),
        }
    }
}

/// 11.B+11.C интеграция: скачивает routing-JSON по URL и парсит в
/// RoutingProfile. Используется при `add_url` и при scheduler-tick.
pub async fn fetch_profile_from_url(url: &str) -> Result<RoutingProfile> {
    let url = canonicalize_github_blob(url);
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .user_agent(format!("Nemefisto/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("reqwest")?;
    let resp = client.get(&url).send().await.context("HTTP")?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} для {url}", resp.status());
    }
    let text = resp.text().await.context("read body")?;
    RoutingProfile::parse_json(&text)
}

/// Конвертация GitHub `blob/...` URL в raw.githubusercontent (чтобы получить
/// содержимое файла, а не HTML-страницу с подсветкой).
pub fn canonicalize_github_blob(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        if let Some((repo_path, file_path)) = rest.split_once("/blob/") {
            return format!("https://raw.githubusercontent.com/{repo_path}/{file_path}");
        }
    }
    url.to_string()
}

/// Запустить фоновый scheduler авто-обновления autorouting профилей и
/// geofiles. Возвращает `oneshot::Sender<()>` — отправь в него `()`
/// при exit чтобы корректно shutdown'нуть.
///
/// Тик scheduler'а:
/// 1. Перебираем все autorouting записи стора.
/// 2. Для каждой: если `now - last_fetched_at >= interval_hours * 3600` —
///    качаем свежий JSON, обновляем профиль.
/// 3. Если в активном профиле есть geofile URLs — обновляем geofiles
///    (с .sha256 оптимизацией).
///
/// Спит между тиками до ближайшего deadline. По sigwake (Notify) — сразу.
pub fn spawn_scheduler(state: Arc<Mutex<RoutingStore>>, wake: Arc<Notify>) -> oneshot::Sender<()> {
    let (tx, mut rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        eprintln!("[routing-scheduler] started");
        loop {
            // Шаг: сделать тик и собрать метаданные о следующем deadline'е.
            let next_sleep = run_tick(&state).await;

            // Минимум — раз в час. Максимум — час (иначе зависнем надолго,
            // а пользователь например только что добавил новый autorouting
            // через UI — Notify его разбудит).
            let sleep_secs = next_sleep.clamp(300, 3600);
            eprintln!("[routing-scheduler] sleep {sleep_secs}s");

            // Параллельно ждём deadline ИЛИ wake-уведомление ИЛИ shutdown.
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {}
                _ = wake.notified() => {
                    eprintln!("[routing-scheduler] woken by notify");
                }
                _ = &mut rx => {
                    eprintln!("[routing-scheduler] shutdown");
                    return;
                }
            }
        }
    });
    tx
}

/// Один тик scheduler: обновляет просроченные autorouting профили и
/// geofiles активного. Возвращает «через сколько секунд минимально
/// нужно проснуться следующий раз» — для умного sleep'а.
async fn run_tick(state: &Arc<Mutex<RoutingStore>>) -> u64 {
    // 1. Снимаем snapshot чтобы не держать lock пока качаем.
    let snapshot = {
        let g = state.lock().unwrap();
        g.snapshot()
    };

    let now = unix_now();
    let mut min_next: u64 = 3600; // дефолт час

    // 2. Прогон по autorouting профилям.
    for entry in &snapshot.entries {
        let ProfileSource::Autorouting { url, interval_hours } = &entry.source else {
            continue;
        };
        let interval_secs = (*interval_hours as u64).max(1) * 3600;
        let next_due = entry
            .last_fetched_at
            .saturating_add(interval_secs)
            .saturating_sub(now);
        if next_due > 0 {
            min_next = min_next.min(next_due);
            continue;
        }
        eprintln!("[routing-scheduler] refresh {} (overdue)", entry.id);
        match fetch_profile_from_url(url).await {
            Ok(profile) => {
                let mut g = state.lock().unwrap();
                if let Err(e) = g.update_profile(&entry.id, profile) {
                    eprintln!("[routing-scheduler] update {} failed: {e}", entry.id);
                }
                min_next = min_next.min(interval_secs);
            }
            Err(e) => {
                eprintln!("[routing-scheduler] fetch {} failed: {e}", entry.id);
                // Retry через час — не вечно сразу повторять при network error.
                min_next = min_next.min(3600);
            }
        }
    }

    // 3. Geofiles активного профиля.
    if let Some(active) = snapshot
        .active_id
        .as_ref()
        .and_then(|id| snapshot.entries.iter().find(|e| &e.id == id))
    {
        let geoip = active.profile.geoip_url.clone();
        let geosite = active.profile.geosite_url.clone();
        if !geoip.is_empty() || !geosite.is_empty() {
            let report = geofiles::update_geofiles_if_changed(&geoip, &geosite).await;
            if !report.errors.is_empty() {
                eprintln!(
                    "[routing-scheduler] geofiles errors: {}",
                    report.errors.join("; ")
                );
            }
        }
    }

    min_next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_blob_canonicalization() {
        let in_ = "https://github.com/user/repo/blob/main/profile.json";
        let out = canonicalize_github_blob(in_);
        assert_eq!(
            out,
            "https://raw.githubusercontent.com/user/repo/main/profile.json"
        );
    }

    #[test]
    fn non_github_url_unchanged() {
        let url = "https://example.com/file.json";
        assert_eq!(canonicalize_github_blob(url), url);
    }
}
