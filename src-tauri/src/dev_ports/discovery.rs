use crate::models::{ListenerExposure, ListenerProtocol};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawListenerRecord {
    pub pid: u32,
    pub command: String,
    pub uid: Option<u32>,
    pub port: u16,
    pub bind_address: String,
    pub exposure: ListenerExposure,
    pub protocol: ListenerProtocol,
}

/// Parses the machine-readable `-F0pcuLn` output from `lsof`.
pub fn parse_lsof_output(raw_bytes: &[u8]) -> Vec<RawListenerRecord> {
    let mut records = Vec::new();
    let mut current_pid: Option<u32> = None;
    let mut current_cmd = String::new();
    let mut current_uid: Option<u32> = None;

    // Split on NUL bytes
    for chunk in raw_bytes.split(|&b| b == 0) {
        if chunk.is_empty() {
            continue;
        }

        let Ok(field_str) = std::str::from_utf8(chunk) else {
            continue;
        };

        let trimmed = field_str.trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ');
        if trimmed.is_empty() {
            continue;
        }

        let prefix = &trimmed[..1];
        let value = &trimmed[1..];

        match prefix {
            "p" => {
                if let Ok(pid) = value.parse::<u32>() {
                    current_pid = Some(pid);
                    current_cmd.clear();
                    current_uid = None;
                }
            }
            "c" => {
                current_cmd = value.to_string();
            }
            "u" => {
                current_uid = value.parse::<u32>().ok();
            }
            "n" => {
                if let Some(pid) = current_pid {
                    if let Some((bind_address, port, exposure)) = parse_endpoint(value) {
                        records.push(RawListenerRecord {
                            pid,
                            command: current_cmd.clone(),
                            uid: current_uid,
                            port,
                            bind_address,
                            exposure,
                            protocol: ListenerProtocol::Tcp,
                        });
                    }
                }
            }
            _ => {
                // Ignore other fields like 'f', 'L', 't', etc.
            }
        }
    }

    deduplicate_listeners(records)
}

/// Parses an endpoint name from `lsof` (e.g. `*:5173`, `127.0.0.1:5173`, `[::1]:5173`, `:::5173`).
pub fn parse_endpoint(name: &str) -> Option<(String, u16, ListenerExposure)> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Bracketed IPv6 case: [::1]:5173 or [::]:5173
    if let Some(close_bracket) = trimmed.find(']') {
        if trimmed.starts_with('[') && close_bracket > 1 {
            let host_part = &trimmed[1..close_bracket];
            let rest = &trimmed[close_bracket + 1..];
            if let Some(port_str) = rest.strip_prefix(':') {
                if let Ok(port) = port_str.parse::<u16>() {
                    if port == 0 {
                        return None;
                    }
                    let (norm_addr, exposure) = classify_address(host_part);
                    return Some((norm_addr, port, exposure));
                }
            }
        }
    }

    // Split on the last ':'
    let last_colon = trimmed.rfind(':')?;
    let host_part = &trimmed[..last_colon];
    let port_part = &trimmed[last_colon + 1..];

    let port = port_part.parse::<u16>().ok()?;
    if port == 0 {
        return None;
    }

    let (norm_addr, exposure) = classify_address(host_part);
    Some((norm_addr, port, exposure))
}

fn classify_address(host: &str) -> (String, ListenerExposure) {
    let host = host.trim();
    if host == "*" || host == "0.0.0.0" || host.is_empty() {
        ("0.0.0.0".to_string(), ListenerExposure::AllInterfaces)
    } else if host == "::" || host == ":::" {
        ("::".to_string(), ListenerExposure::AllInterfaces)
    } else if host == "localhost" || host == "127.0.0.1" || host.starts_with("127.") {
        (
            if host == "localhost" {
                "127.0.0.1".to_string()
            } else {
                host.to_string()
            },
            ListenerExposure::Loopback,
        )
    } else if host == "::1" {
        ("::1".to_string(), ListenerExposure::Loopback)
    } else {
        (host.to_string(), ListenerExposure::Network)
    }
}

/// Deduplicates duplicate socket descriptors for the same (pid, port, protocol),
/// giving preference to more specific bind addresses (e.g. Loopback/Network over AllInterfaces).
fn deduplicate_listeners(records: Vec<RawListenerRecord>) -> Vec<RawListenerRecord> {
    let mut map: HashMap<(u32, u16), RawListenerRecord> = HashMap::new();

    for record in records {
        let key = (record.pid, record.port);
        match map.get(&key) {
            None => {
                map.insert(key, record);
            }
            Some(existing) => {
                // If the new one is Loopback or Network and existing is AllInterfaces, replace it
                if existing.exposure == ListenerExposure::AllInterfaces
                    && record.exposure != ListenerExposure::AllInterfaces
                {
                    map.insert(key, record);
                }
            }
        }
    }

    let mut result: Vec<RawListenerRecord> = map.into_values().collect();
    // Sort stably by port, then pid
    result.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.pid.cmp(&b.pid)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_nul_delimited_fixture() {
        let fixture = b"p620\0crapportd\0u501\0Lapple\0\nf10\0n*:65491\0\nf11\0n*:65491\0\np32892\0cnode\0u501\0Lapple\0\nf28\0n127.0.0.1:5173\0";
        let records = parse_lsof_output(fixture);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].pid, 32892);
        assert_eq!(records[0].port, 5173);
        assert_eq!(records[0].bind_address, "127.0.0.1");
        assert_eq!(records[0].exposure, ListenerExposure::Loopback);
        assert_eq!(records[0].command, "node");
        assert_eq!(records[0].uid, Some(501));

        assert_eq!(records[1].pid, 620);
        assert_eq!(records[1].port, 65491);
        assert_eq!(records[1].bind_address, "0.0.0.0");
        assert_eq!(records[1].exposure, ListenerExposure::AllInterfaces);
    }

    #[test]
    fn skip_malformed_and_partial_records_safely() {
        let fixture =
            b"invalid_garbage\0pxyz\0cbad\0f1\0nnotaport\0p9999\0cvalid\0u501\0f2\0n*:3000\0";
        let records = parse_lsof_output(fixture);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pid, 9999);
        assert_eq!(records[0].port, 3000);
        assert_eq!(records[0].command, "valid");
    }

    #[test]
    fn parse_ipv4_loopback_ipv6_loopback_wildcard_and_network() {
        assert_eq!(
            parse_endpoint("127.0.0.1:5173"),
            Some(("127.0.0.1".to_string(), 5173, ListenerExposure::Loopback))
        );
        assert_eq!(
            parse_endpoint("127.0.0.2:8080"),
            Some(("127.0.0.2".to_string(), 8080, ListenerExposure::Loopback))
        );
        assert_eq!(
            parse_endpoint("localhost:3000"),
            Some(("127.0.0.1".to_string(), 3000, ListenerExposure::Loopback))
        );
        assert_eq!(
            parse_endpoint("[::1]:5173"),
            Some(("::1".to_string(), 5173, ListenerExposure::Loopback))
        );
        assert_eq!(
            parse_endpoint("::1:5173"),
            Some(("::1".to_string(), 5173, ListenerExposure::Loopback))
        );
        assert_eq!(
            parse_endpoint("*:5173"),
            Some(("0.0.0.0".to_string(), 5173, ListenerExposure::AllInterfaces))
        );
        assert_eq!(
            parse_endpoint("0.0.0.0:3000"),
            Some(("0.0.0.0".to_string(), 3000, ListenerExposure::AllInterfaces))
        );
        assert_eq!(
            parse_endpoint("[::]:8080"),
            Some(("::".to_string(), 8080, ListenerExposure::AllInterfaces))
        );
        assert_eq!(
            parse_endpoint(":::8080"),
            Some(("::".to_string(), 8080, ListenerExposure::AllInterfaces))
        );
        assert_eq!(
            parse_endpoint("192.168.1.100:8000"),
            Some(("192.168.1.100".to_string(), 8000, ListenerExposure::Network))
        );
        assert_eq!(
            parse_endpoint("10.0.0.5:4000"),
            Some(("10.0.0.5".to_string(), 4000, ListenerExposure::Network))
        );
        assert_eq!(parse_endpoint("invalid"), None);
        assert_eq!(parse_endpoint("127.0.0.1:0"), None);
    }

    #[test]
    fn deduplicate_duplicate_records_without_merging_different_owners() {
        // Same PID (100) on port 5173 listed twice (fd 10 and fd 11)
        // Different PID (200) also on port 5173 (e.g. SO_REUSEPORT or dual-stack)
        let fixture = b"p100\0cnode\0u501\0f10\0n*:5173\0\nf11\0n127.0.0.1:5173\0\np200\0cother\0u501\0f5\0n192.168.1.5:5173\0";
        let records = parse_lsof_output(fixture);

        assert_eq!(records.len(), 2);
        let pid100 = records.iter().find(|r| r.pid == 100).unwrap();
        assert_eq!(pid100.port, 5173);
        assert_eq!(pid100.bind_address, "127.0.0.1"); // preferred over wildcard

        let pid200 = records.iter().find(|r| r.pid == 200).unwrap();
        assert_eq!(pid200.port, 5173);
        assert_eq!(pid200.bind_address, "192.168.1.5");
    }
}
