//! Pairing URI and QR code generation.

use std::net::IpAddr;

use qrcode::{render::svg, EcLevel, QrCode};

use crate::{config::Config, crypto, protocol};

/// Interfaces that are never reachable from a phone on the same Wi-Fi.
const IGNORED_INTERFACE_PREFIXES: &[&str] = &[
    "lo", "docker", "br-", "veth", "virbr", "vboxnet", "vmnet", "tun", "tap", "wg", "tailscale",
    "zt",
];

pub fn lan_addresses() -> Vec<IpAddr> {
    let mut addrs: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| {
            !iface.is_loopback()
                && !IGNORED_INTERFACE_PREFIXES
                    .iter()
                    .any(|prefix| iface.name.starts_with(prefix))
        })
        .map(|iface| iface.ip())
        .filter(|ip| match ip {
            IpAddr::V4(v4) => v4.is_private(),
            IpAddr::V6(_) => false,
        })
        .collect();
    addrs.sort();
    addrs.dedup();
    addrs
}

pub fn pair_uri(config: &Config, pair_secret: &[u8], hosts: &[IpAddr]) -> String {
    let hosts = hosts
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "peegel://pair?v={}&id={}&name={}&h={}&p={}&s={}",
        protocol::VERSION,
        config.server_id,
        percent_encode(&config.server_name),
        hosts,
        config.port,
        crypto::b64(pair_secret),
    )
}

pub fn qr_svg(data: &str) -> String {
    match QrCode::with_error_correction_level(data, EcLevel::M) {
        Ok(code) => code
            .render::<svg::Color>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .dark_color(svg::Color("#111111"))
            .light_color(svg::Color("#ffffff"))
            .build(),
        Err(err) => {
            log::error!("could not render QR code: {err}");
            String::new()
        }
    }
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_names() {
        assert_eq!(percent_encode("Lance's PC"), "Lance%27s%20PC");
    }
}
