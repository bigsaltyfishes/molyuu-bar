use futures_util::{StreamExt, stream};
use rusty_network_manager::{SettingsConnectionProxy, SettingsProxy};
use smol::channel::Sender;
use std::collections::HashSet;
use zbus::zvariant::OwnedObjectPath;

use crate::service::network::{endpoints::inter::NetworkServiceInterEvent, NetworkService, DBUS_CONNECTION};

use super::ap::AccessPointSecurity;

#[async_trait::async_trait]
pub(in super::super) trait WirelessProfileHelperExt {
    async fn collect_wireless(
        paths: Vec<OwnedObjectPath>,
    ) -> HashSet<(String, AccessPointSecurity)> {
        let mut set = HashSet::new();

        for path in paths {
            if let Ok(proxy) = SettingsConnectionProxy::new_from_path(path, &DBUS_CONNECTION).await
            {
                if let Ok(cfg) = proxy.get_settings().await {
                    let is_wireless = cfg
                        .get("connection")
                        .and_then(|c| c.get("type"))
                        .map(|v| v.to_string().trim_matches('"') == "802-11-wireless")
                        .unwrap_or(false);

                    if is_wireless {
                        if let (Some(ss_raw), Some(km_raw)) = (
                            cfg.get("802-11-wireless")
                                .and_then(|w| w.get("ssid"))
                                .map(|v| v.to_string()),
                            cfg.get("802-11-wireless-security")
                                .and_then(|w| w.get("key-mgmt"))
                                .map(|v| v.to_string()),
                        ) {
                            if let (Ok(ss), Ok(sec)) = (
                                String::from_utf8(ss_raw.into_bytes()).map_err(|_| ()),
                                <AccessPointSecurity as TryFrom<&str>>::try_from(
                                    km_raw.trim_matches('"'),
                                ),
                            ) {
                                set.insert((ss, sec));
                            }
                        }
                    }
                }
            }
        }

        set
    }
}

#[async_trait::async_trait]
pub(in super::super) trait WirelessProfileSyncExt: WirelessProfileHelperExt {
    /// Synchronizes known wireless network connections with NetworkManager.
    ///
    /// It lists existing connections, filters for wireless ones, and sends an event
    /// to update the internal state. It also listens for connection changes from NetworkManager.
    async fn sync_connections(sender: Sender<NetworkServiceInterEvent>) {
        let settings = SettingsProxy::new(&DBUS_CONNECTION)
            .await
            .expect("SettingsProxy creation failed");
        let initial = settings
            .list_connections()
            .await
            .expect("Failed to get initial connections");
        let initial_set = Self::collect_wireless(initial).await;
        if !initial_set.is_empty() {
            sender
                .send(NetworkServiceInterEvent::RefreshAPConnections { map: initial_set })
                .await
                .expect("Failed to send initial refresh event");
        }
        let mut stream = settings
            .receive_connections_changed()
            .await
            .filter_map(|signal| async move { signal.get().await.ok() })
            .boxed();

        while let Some(paths) = stream.next().await {
            let set = Self::collect_wireless(paths).await;
            if !set.is_empty() {
                sender
                    .send(NetworkServiceInterEvent::RefreshAPConnections { map: set })
                    .await
                    .expect("Failed to send refresh event");
            }
        }
    }
}

impl WirelessProfileHelperExt for NetworkService {}
impl WirelessProfileSyncExt for NetworkService {}