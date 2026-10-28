//! mDNS / DNS-SD advertisement so phones can find this computer.

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::{config::Config, protocol};

pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertiser {
    pub fn start(config: &Config) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let host = gethostname::gethostname().to_string_lossy().into_owned();
        let instance = format!("{} ({})", config.server_name, &config.server_id[..6]);
        let version = protocol::VERSION.to_string();
        let properties = [("id", config.server_id.as_str()), ("v", version.as_str())];
        let service = ServiceInfo::new(
            protocol::SERVICE_TYPE,
            &instance,
            &format!("{host}.local."),
            "",
            config.port,
            &properties[..],
        )?
        .enable_addr_auto();
        let fullname = service.get_fullname().to_string();
        daemon.register(service)?;
        log::info!("advertising {fullname}");
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}
