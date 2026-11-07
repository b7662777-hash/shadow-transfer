pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_speed(bytes_per_second: f64) -> String {
    if !bytes_per_second.is_finite() || bytes_per_second <= 0.0 {
        "0 B/s".to_string()
    } else {
        format!("{}/s", format_bytes(bytes_per_second.round() as u64))
    }
}

pub fn format_eta(seconds: Option<u64>) -> String {
    match seconds {
        None => "—".to_string(),
        Some(0) => "0s".to_string(),
        Some(secs) if secs < 60 => format!("{secs}s"),
        Some(secs) if secs < 3600 => format!("{}m {}s", secs / 60, secs % 60),
        Some(secs) => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

pub fn short_fingerprint(fingerprint: &str) -> String {
    if fingerprint.len() <= 16 {
        fingerprint.to_string()
    } else {
        format!(
            "{}…{}",
            &fingerprint[..12],
            &fingerprint[fingerprint.len() - 4..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(format_eta(Some(75)), "1m 15s");
        assert_eq!(format_eta(None), "—");
    }
}
