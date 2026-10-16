//! Read the OS IPv4 default gateway; never guess a router from the host subnet.
use std::net::Ipv4Addr;

fn usable(value: &str) -> Option<Ipv4Addr> {
    let ip: Ipv4Addr = value.parse().ok()?;
    (!ip.is_unspecified() && !ip.is_loopback() && !ip.is_multicast() && !ip.is_broadcast())
        .then_some(ip)
}

pub(crate) async fn default_gateway() -> Option<Ipv4Addr> {
    tokio::time::timeout(
        std::time::Duration::from_millis(700),
        tokio::task::spawn_blocking(read_gateway),
    )
    .await
    .ok()?
    .ok()?
}

#[cfg(target_os = "macos")]
fn read_gateway() -> Option<Ipv4Addr> {
    let output = std::process::Command::new("/sbin/route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return None;
    }
    parse_macos(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos(text: &str) -> Option<Ipv4Addr> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("gateway:")
            .and_then(|value| usable(value.trim()))
    })
}

#[cfg(windows)]
fn read_gateway() -> Option<Ipv4Addr> {
    use std::os::windows::process::CommandExt;
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let path = std::path::PathBuf::from(system)
        .join("System32")
        .join("route.exe");
    let output = std::process::Command::new(path)
        .args(["print", "-4", "0.0.0.0"])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return None;
    }
    parse_windows(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(windows, test))]
fn parse_windows(text: &str) -> Option<Ipv4Addr> {
    text.lines()
        .filter_map(|line| {
            let columns: Vec<_> = line.split_whitespace().collect();
            if columns.len() != 5 || columns[0] != "0.0.0.0" || columns[1] != "0.0.0.0" {
                return None;
            }
            let gateway = usable(columns[2])?;
            let _: Ipv4Addr = columns[3].parse().ok()?;
            Some((columns[4].parse::<u32>().ok()?, gateway))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, gateway)| gateway)
}

#[cfg(target_os = "linux")]
fn read_gateway() -> Option<Ipv4Addr> {
    let text = std::fs::read_to_string("/proc/net/route").ok()?;
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let c: Vec<_> = line.split_whitespace().collect();
            if c.len() < 8 || c[1] != "00000000" || c[7] != "00000000" {
                return None;
            }
            let flags = u32::from_str_radix(c[3], 16).ok()?;
            if flags & 3 != 3 {
                return None;
            }
            let gateway = Ipv4Addr::from(u32::from_str_radix(c[2], 16).ok()?.to_le_bytes());
            let metric = c[6].parse::<u32>().ok()?;
            usable(&gateway.to_string()).map(|gateway| (metric, gateway))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, gateway)| gateway)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn read_gateway() -> Option<Ipv4Addr> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_only_real_default_gateway_rows() {
        assert_eq!(
            parse_macos("route to: default\ngateway: 192.168.5.1\ninterface: en0"),
            Some(Ipv4Addr::new(192, 168, 5, 1))
        );
        assert_eq!(parse_macos("gateway: link#4"), None);
        assert_eq!(
            parse_windows(
                "0.0.0.0 0.0.0.0 192.168.1.1 192.168.1.2 45\n0.0.0.0 0.0.0.0 10.0.0.1 10.0.0.2 20\n0.0.0.0 0.0.0.0 On-link 10.0.0.3 1"
            ),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(parse_windows("0.0.0.0 0.0.0.0 127.0.0.1 127.0.0.2 1"), None);
        assert_eq!(parse_windows("0.0.0.0 0.0.0.0 192.168.1.1 5"), None);
    }
}
