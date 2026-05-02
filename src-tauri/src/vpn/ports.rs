//! Утилиты выбора свободных TCP-портов для inbound'ов VPN-движков.
//!
//! Перенесены из удалённого `vpn/xray.rs` после миграции на sing-box —
//! используются обоими движками (sing-box + Mihomo) для random-port
//! пика inbound'ов (этап 9.H — защита от детекта VPN-клиента
//! сторонним процессом сканированием стандартных SOCKS-портов).

use std::net::TcpListener;

/// Найти первый свободный TCP-порт начиная со `start` (вверх до 65535).
/// Используем `bind` в loopback-only с `SO_REUSEADDR=0` (Tokio default)
/// — если bind проходит, порт точно свободен; если нет — следующий.
///
/// Возвращает `start` как fallback если ничего не нашли (теоретически
/// невозможно, но не паникуем — мы потом получим bind-error при реальном
/// запуске inbound'а).
pub fn find_free_port(start: u16) -> u16 {
    for p in start..=65535u16 {
        if TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
    }
    start
}

/// Сгенерировать псевдослучайный порт в диапазоне `[30000, 60000)` —
/// для рандомизации inbound'ов (этап 9.H).
///
/// Использует наносекунды текущего времени как энтропию — этого достаточно
/// для unpredictability внешним процессам, которым нужен полный port-scan
/// 30k-портов чтобы найти наш SOCKS. Криптостойкость не требуется.
pub fn random_high_port() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    30_000 + (nanos % 30_000) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_free_port_returns_in_range() {
        let p = find_free_port(40000);
        assert!(p >= 40000);
    }

    #[test]
    fn random_high_port_is_in_expected_range() {
        for _ in 0..10 {
            let p = random_high_port();
            assert!((30_000..60_000).contains(&p));
        }
    }
}
