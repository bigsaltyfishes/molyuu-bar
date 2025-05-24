use std::collections::HashMap;

use rusty_network_manager::WirelessProxy;
use zbus::zvariant::OwnedObjectPath;

use crate::service::network::{NetworkService, DBUS_CONNECTION};

#[async_trait::async_trait]
pub(in super::super) trait WirelessScanExt {
    /// Requests a scan for wireless networks on the specified device.
    async fn request_scan(device_path: OwnedObjectPath) {
        let wireless = WirelessProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create wireless proxy for {:?}", device_path).as_str());
        wireless
            .request_scan(HashMap::new())
            .await
            .expect(format!("Failed to request scan for device: {}", device_path).as_str());
    }
}

impl WirelessScanExt for NetworkService {}