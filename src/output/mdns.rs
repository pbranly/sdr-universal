//! mDNS / DNS-SD advertisement of the rtl_tcp server (`_rtl_tcp._tcp`).
//!
//! Clients such as NyxScope list the rtl_tcp servers of the local network
//! automatically; advertising the gateway makes it appear in that list without
//! typing an address. Failure to advertise is never fatal: the server keeps
//! working and clients can still connect by address.

use anyhow::{anyhow, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;
use std::time::Duration;

/// DNS-SD service type used by rtl_tcp servers.
pub const SERVICE_TYPE: &str = "_rtl_tcp._tcp.local.";

/// Longest DNS label, in bytes (the instance name is one label).
const MAX_LABEL_BYTES: usize = 63;

/// Whether the gateway should advertise itself, and why not if it should not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Advertise,
    /// Turned off with `--no-mdns` or `SDR_MDNS=0`.
    Disabled,
    /// Listening on the loopback interface only: nobody else could connect.
    Loopback,
}

pub fn decide(bind_ip: IpAddr, disabled: bool) -> Decision {
    if disabled {
        Decision::Disabled
    } else if bind_ip.is_loopback() {
        Decision::Loopback
    } else {
        Decision::Advertise
    }
}

/// Host name usable in a `.local.` name: lowercase letters, digits and
/// hyphens, without the domain part.
pub fn sanitize_hostname(raw: &str) -> String {
    let short = raw.trim().split('.').next().unwrap_or("");

    let cleaned: String = short
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    let cleaned = cleaned.trim_matches('-');

    if cleaned.is_empty() {
        "sdr-universal".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Name of this machine, from the kernel, `/etc/hostname` or `$HOSTNAME`.
pub fn local_hostname() -> String {
    let raw = ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();

    sanitize_hostname(&raw)
}

/// Default instance name: "SDR Universal on <host>".
pub fn default_instance_name(hostname: &str) -> String {
    truncate_label(&format!("SDR Universal on {}", hostname))
}

/// Cuts a name to the longest DNS label, on a character boundary.
pub fn truncate_label(name: &str) -> String {
    let name = name.trim();

    if name.len() <= MAX_LABEL_BYTES {
        return name.to_string();
    }

    let mut end = MAX_LABEL_BYTES;

    while !name.is_char_boundary(end) {
        end -= 1;
    }

    name[..end].trim_end().to_string()
}

/// A registered advertisement; dropped with [`Advertisement::stop`].
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    /// Registers the service. All interface addresses are advertised when the
    /// server listens on every interface; only `bind_ip` otherwise.
    pub fn start(instance: &str, bind_ip: IpAddr, port: u16, backend: &str) -> Result<Self> {
        let host = format!("{}.local.", local_hostname());
        let properties = [
            ("software", "sdr-universal"),
            ("version", env!("CARGO_PKG_VERSION")),
            ("backend", backend),
        ];

        let info = if bind_ip.is_unspecified() {
            ServiceInfo::new(SERVICE_TYPE, instance, &host, "", port, &properties[..])
                .map_err(|e| anyhow!("{}", e))?
                .enable_addr_auto()
        } else {
            ServiceInfo::new(
                SERVICE_TYPE,
                instance,
                &host,
                bind_ip,
                port,
                &properties[..],
            )
            .map_err(|e| anyhow!("{}", e))?
        };

        let fullname = info.get_fullname().to_string();
        let daemon = ServiceDaemon::new().map_err(|e| anyhow!("{}", e))?;

        daemon.register(info).map_err(|e| anyhow!("{}", e))?;

        Ok(Self { daemon, fullname })
    }

    /// Withdraws the service (so that clients drop it at once) and stops the
    /// responder.
    pub fn stop(self) {
        if let Ok(status) = self.daemon.unregister(&self.fullname) {
            let _ = status.recv_timeout(Duration::from_secs(1));
        }

        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn decision_follows_the_bind_address_and_the_switch() {
        let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);

        assert_eq!(decide(any, false), Decision::Advertise);
        assert_eq!(decide(lan, false), Decision::Advertise);
        assert_eq!(
            decide(IpAddr::V6(Ipv6Addr::UNSPECIFIED), false),
            Decision::Advertise
        );
        assert_eq!(decide(loopback, false), Decision::Loopback);
        assert_eq!(
            decide(IpAddr::V6(Ipv6Addr::LOCALHOST), false),
            Decision::Loopback
        );
        assert_eq!(decide(any, true), Decision::Disabled);
        assert_eq!(decide(loopback, true), Decision::Disabled);
    }

    #[test]
    fn hostnames_are_made_dns_safe() {
        assert_eq!(
            sanitize_hostname("pbranly-NUC13ANHI7\n"),
            "pbranly-nuc13anhi7"
        );
        assert_eq!(sanitize_hostname("my host.example.com"), "my-host");
        assert_eq!(sanitize_hostname("  --odd_name--  "), "odd-name");
        assert_eq!(sanitize_hostname(""), "sdr-universal");
        assert_eq!(sanitize_hostname("..."), "sdr-universal");
        assert_eq!(sanitize_hostname("é"), "sdr-universal");
    }

    #[test]
    fn instance_names_fit_in_a_dns_label() {
        assert_eq!(default_instance_name("nuc"), "SDR Universal on nuc");

        let long = "x".repeat(200);
        assert!(default_instance_name(&long).len() <= MAX_LABEL_BYTES);
        assert!(truncate_label(&"é".repeat(100)).len() <= MAX_LABEL_BYTES);
        assert!(truncate_label(&"é".repeat(100)).chars().all(|c| c == 'é'));
        assert_eq!(truncate_label("  short  "), "short");
    }
}
