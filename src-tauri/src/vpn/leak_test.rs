//! Проверка утечек IP/DNS (этапы 13.B и 13.H).
//!
//! Делает два HTTP-запроса:
//! 1. `https://ipapi.co/json/` — публичный IP клиента + страна/город.
//!    После connect должно вернуться значение VPN-сервера. Если
//!    после connect видим свой ISP-IP — туннель не поднялся.
//! 2. `https://1.1.1.1/dns-query?name=whoami.cloudflare&type=TXT` —
//!    DoH-запрос к Cloudflare. TXT-запись `whoami.cloudflare`
//!    возвращает IP резолвера, который **сделал** этот запрос.
//!    Если IP совпадает с публичным IP пользователя или принадлежит
//!    ISP — у нас DNS leak (запросы идут через системный DNS, а не
//!    через VPN).
//!
//! Оба запроса идут через `socks_port` если задан (proxy-mode), либо
//! напрямую (tun-mode — там system route уже через VPN).
//!
//! ВНИМАНИЕ: в tun-mode без proxy reqwest пойдёт через system DNS,
//! который мы насильно меняем на DoH через Mihomo `external-controller`
//! либо через Xray fakedns. То есть IP в DoH-ответе зависит от того
//! как настроен DNS внутри VPN-туннеля.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Результат комбинированной проверки. Все поля опциональны кроме `ok`:
/// если внешний сервис отдал не то что ожидали (rate-limit, no internet),
/// просто оставляем `None` и фронт показывает «—» в этой строке.
#[derive(Debug, Clone, Serialize)]
pub struct LeakTestResult {
    /// Публичный IP по версии ipapi.co (то что видит мир).
    pub ip: Option<String>,
    /// Двухбуквенный код страны (`DE`, `US`, `RU`).
    pub country_code: Option<String>,
    /// Полное имя страны (`Germany`).
    pub country_name: Option<String>,
    /// Город (если ipapi.co определил).
    pub city: Option<String>,
    /// IP DoH-резолвера. Сравнивается с `ip`: если равны — значит DNS
    /// идёт мимо VPN через системный/ISP DNS-сервер.
    pub dns_resolver: Option<String>,
    /// `true` если DNS-резолвер отличается от публичного IP. Это не
    /// 100%-доказательство отсутствия утечки (резолвер может быть
    /// просто другим VPN-узлом), но обратное — гарантия leak'а.
    pub dns_clean: bool,
    /// 14.D: IPv6 leak detection. Если v6-only endpoint отвечает —
    /// значит трафик IPv6 идёт мимо VPN (наш WinTUN/proxy покрывают
    /// только v4). `None` если v6 endpoint недоступен (как и должно
    /// быть при чистом туннеле). `Some(ip)` — утечка.
    pub ipv6_leak: Option<String>,
}

/// Промежуточный тип, в который собираем результат от любого
/// IP-сервиса. Не все провайдеры отдают city/country_name — поля
/// опциональны, фронт показывает «—» где пусто.
struct IpInfo {
    ip: Option<String>,
    country_code: Option<String>,
    country_name: Option<String>,
    city: Option<String>,
}

#[derive(Deserialize)]
struct IpwhoIsResponse {
    ip: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
    /// ipwho.is возвращает `success: false` при rate-limit / ошибке.
    success: Option<bool>,
}

/// Cloudflare DoH JSON формат — упрощённый, нам нужен только TXT-data
/// первого ответа.
#[derive(Deserialize)]
struct DohResponse {
    #[serde(rename = "Answer", default)]
    answer: Vec<DohAnswer>,
}

#[derive(Deserialize)]
struct DohAnswer {
    data: String,
}

/// Собрать reqwest::Client с опциональным SOCKS5-прокси.
///
/// `socks_port` — наш локальный SOCKS5 inbound (proxy-mode). В tun-mode
/// передаётся `None` и клиент использует system route (которая в TUN
/// уже идёт через VPN).
fn build_client(socks_port: Option<u16>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .danger_accept_invalid_certs(false)
        // Browser-like User-Agent: ipapi.co и DoH-серверы могут
        // блокировать пустой/дефолтный reqwest UA.
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/130.0 Safari/537.36",
        );

    if let Some(port) = socks_port {
        let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}"))
            .context("неверный socks5 URL")?;
        builder = builder.proxy(proxy);
    }

    builder.build().context("не удалось собрать reqwest::Client")
}

/// Получить публичный IP + GeoIP с fallback-цепочкой провайдеров.
///
/// Порядок:
/// 1. **Cloudflare cdn-cgi/trace** — text-формат `ip=X\nloc=DE\n...`.
///    Самый надёжный (один из самых живучих CDN на планете), но
///    отдаёт только country_code, без city/country_name.
/// 2. **ipwho.is** — JSON с полным GeoIP (city, country_name).
///    Используется если cloudflare упал или для дозаполнения city.
///
/// Если оба сервиса легли — возвращаем `IpInfo` с пустыми полями;
/// тост покажет «не удалось получить IP».
async fn fetch_public_ip(client: &reqwest::Client) -> IpInfo {
    let mut info = IpInfo {
        ip: None,
        country_code: None,
        country_name: None,
        city: None,
    };

    // 1. Cloudflare trace — основной источник IP + country code.
    if let Ok(resp) = client
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(text) = resp.text().await {
                for line in text.lines() {
                    if let Some(v) = line.strip_prefix("ip=") {
                        info.ip = Some(v.trim().to_string());
                    } else if let Some(v) = line.strip_prefix("loc=") {
                        info.country_code = Some(v.trim().to_uppercase());
                    }
                }
            }
        }
    }

    // 2. ipwho.is — за city + country_name. Если cloudflare уже дал
    //    IP и country_code, мы пропустим ipwho.is только когда инфы
    //    «достаточно» — но всё-таки делаем, чтобы получить city.
    //    Если cloudflare упал, ipwho.is становится единственным.
    if info.city.is_none() {
        if let Ok(resp) = client.get("https://ipwho.is/").send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<IpwhoIsResponse>().await {
                    if body.success.unwrap_or(true) {
                        if info.ip.is_none() {
                            info.ip = body.ip;
                        }
                        if info.country_code.is_none() {
                            info.country_code = body.country_code;
                        }
                        info.country_name = body.country;
                        info.city = body.city;
                    }
                }
            }
        }
    }

    info
}

/// 14.D: проверка IPv6-утечки. Делает HTTP к v6-only endpoint
/// (`https://api6.ipify.org`) с коротким таймаутом. Если запрос
/// прошёл и вернул IP — значит v6-трафик идёт мимо VPN: наш WinTUN
/// и SOCKS5 inbound покрывают только v4, физический NIC с v6
/// доступностью отвечает напрямую.
///
/// Возвращает `None` при любой ошибке (timeout, DNS fail, HTTP 4xx) —
/// это считается «v6 чистый» (нет route в v6 интернет, либо kill-switch
/// блокирует, либо ISP без v6).
async fn fetch_ipv6_leak(client: &reqwest::Client) -> Option<String> {
    // Таймаут короче основного: при правильно настроенном туннеле
    // запрос должен фейлиться быстро. Если v6 reachable — он мгновенный.
    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        client.get("https://api6.ipify.org/?format=text").send(),
    )
    .await
    .ok()?
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    let trimmed = text.trim();
    // ipify возвращает голый IP. Защита от мусора: должен содержать «:»
    // (любой v6 имеет хотя бы один колон). v4-only ipify endpoint
    // сюда не должен попадать (мы запросили api6.*), но проверка не
    // лишняя на случай если ipify за NAT64 вернёт IPv4-mapped.
    if trimmed.is_empty() || !trimmed.contains(':') {
        return None;
    }
    Some(trimmed.to_string())
}

/// DoH-запрос к Cloudflare для TXT `whoami.cloudflare`. Возвращает IP
/// резолвера — то что видит Cloudflare DNS как «кто только что задал
/// этот вопрос».
async fn fetch_dns_resolver(client: &reqwest::Client) -> Result<String> {
    let resp = client
        .get("https://1.1.1.1/dns-query?name=whoami.cloudflare&type=TXT")
        .header("accept", "application/dns-json")
        .send()
        .await
        .context("cloudflare DoH недоступен")?;
    if !resp.status().is_success() {
        return Err(anyhow!("DoH статус {}", resp.status()));
    }
    let body: DohResponse = resp.json().await.context("DoH bad json")?;
    let answer = body
        .answer
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("DoH без ответа"))?;
    // Cloudflare обрамляет TXT в кавычки: `"203.0.113.42"` → срезаем.
    let trimmed = answer.data.trim().trim_matches('"').to_string();
    Ok(trimmed)
}

/// Запустить комбинированную проверку. Запросы идут параллельно через
/// `tokio::join!` — общая длительность ~max(t1, t2), не сумма.
pub async fn run(socks_port: Option<u16>) -> Result<LeakTestResult> {
    let client = build_client(socks_port)?;

    let (ip_data, dns_res, ipv6_leak) = tokio::join!(
        fetch_public_ip(&client),
        fetch_dns_resolver(&client),
        fetch_ipv6_leak(&client),
    );

    let dns_resolver = dns_res.ok();

    let ip = ip_data.ip.clone();
    let country_code = ip_data.country_code.clone();
    let country_name = ip_data.country_name.clone();
    let city = ip_data.city.clone();

    // DNS «чистый» если резолвер не совпадает с public IP. Если хотя
    // бы одно из значений неизвестно — дефолтно true (нет данных для
    // паники), фронт показывает «не определено».
    let dns_clean = match (ip.as_deref(), dns_resolver.as_deref()) {
        (Some(a), Some(b)) => a != b,
        _ => true,
    };

    Ok(LeakTestResult {
        ip,
        country_code,
        country_name,
        city,
        dns_resolver,
        dns_clean,
        ipv6_leak,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 14.D: serialization-test нового поля `ipv6_leak`. Защищает от
    /// случайных правок serde-имени (frontend ожидает snake_case).
    #[test]
    fn result_serializes_ipv6_leak_field() {
        let r = LeakTestResult {
            ip: Some("1.2.3.4".into()),
            country_code: None,
            country_name: None,
            city: None,
            dns_resolver: None,
            dns_clean: true,
            ipv6_leak: Some("2606:4700::1".into()),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            json.contains(r#""ipv6_leak":"2606:4700::1""#),
            "JSON: {json}"
        );
    }

    #[test]
    fn result_serializes_ipv6_leak_null() {
        let r = LeakTestResult {
            ip: None,
            country_code: None,
            country_name: None,
            city: None,
            dns_resolver: None,
            dns_clean: true,
            ipv6_leak: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""ipv6_leak":null"#), "JSON: {json}");
    }
}
