use std::collections::HashMap;

use super::ap::{AccessPoint, AccessPointConnectResult};
use crate::service::network::endpoints::inter::NetworkServiceInterEvent;
use crate::service::network::{
    NetworkService, DBUS_CONNECTION
};
use futures_util::StreamExt;
use rusty_network_manager::{ActiveProxy, DeviceProxy, NetworkManagerProxy, SettingsConnectionProxy, SettingsProxy};
use rusty_network_manager::dbus_interface_types::{NMActiveConnectionState, NMActiveConnectionStateReason};
use smol::channel::Sender;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};


#[derive(Debug)]
/// Represents the settings for a wireless network connection.
pub struct WirelessConnectionSettings<'a> {
    pub id: Value<'a>,
    pub type_: Value<'a>,
    pub uuid: Value<'a>,
    pub autoconnect: Value<'a>,
    pub ssid: Value<'a>,
    pub key_mgmt: Value<'a>,
    pub psk: Value<'a>,
    pub ipv4: Value<'a>,
    pub ipv6: Value<'a>,
    pub has_psk: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> WirelessConnectionSettings<'a> {
    /// Converts the wireless connection settings into a HashMap suitable for D-Bus communication.
    /// This method returns a map of references to the original values.
    pub fn map_ref(&'a self) -> HashMap<&'a str, HashMap<&'a str, &'a Value<'a>>> {
        let mut settings = HashMap::new();
        let mut connection = HashMap::new();
        connection.insert("id", &self.id);
        connection.insert("type", &self.type_);
        connection.insert("uuid", &self.uuid);
        connection.insert("autoconnect", &self.autoconnect);
        settings.insert("connection", connection);

        let mut wireless = HashMap::new();
        wireless.insert("ssid", &self.ssid);

        let mut wireless_security = HashMap::new();
        wireless_security.insert("key-mgmt", &self.key_mgmt);

        if self.has_psk {
            wireless_security.insert("psk", &self.psk);
        }
        settings.insert("802-11-wireless", wireless);
        settings.insert("802-11-wireless-security", wireless_security);

        let mut ipv4 = HashMap::new();
        ipv4.insert("method", &self.ipv4);
        settings.insert("ipv4", ipv4);

        let mut ipv6 = HashMap::new();
        ipv6.insert("method", &self.ipv6);
        settings.insert("ipv6", ipv6);

        settings
    }

    /// Converts the wireless connection settings into a HashMap suitable for D-Bus communication.
    /// This method consumes the settings and returns a map of owned values.
    pub fn into_map(self) -> HashMap<&'a str, HashMap<&'a str, Value<'a>>> {
        let mut settings = HashMap::new();
        let mut connection = HashMap::new();
        connection.insert("id", self.id);
        connection.insert("type", self.type_);
        connection.insert("uuid", self.uuid);
        connection.insert("autoconnect", self.autoconnect);
        settings.insert("connection", connection);

        let mut wireless = HashMap::new();
        wireless.insert("ssid", self.ssid);

        let mut wireless_security = HashMap::new();
        wireless_security.insert("key-mgmt", self.key_mgmt);

        if self.has_psk {
            wireless_security.insert("psk", self.psk);
        }
        settings.insert("802-11-wireless", wireless);
        settings.insert("802-11-wireless-security", wireless_security);

        let mut ipv4 = HashMap::new();
        ipv4.insert("method", self.ipv4);
        settings.insert("ipv4", ipv4);

        let mut ipv6 = HashMap::new();
        ipv6.insert("method", self.ipv6);
        settings.insert("ipv6", ipv6);

        settings
    }
}

#[derive(Debug, Default)]
/// Builder for creating `WirelessConnectionSettings`.
pub struct WirelessConnectionSettingsBuilder<'a> {
    id: Option<String>,
    ssid: Option<String>,
    key_mgmt: Option<String>,
    psk: Option<String>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> WirelessConnectionSettingsBuilder<'a> {
    /// Creates a new `WirelessConnectionSettingsBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the ID for the wireless connection.
    pub fn id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the SSID for the wireless connection.
    pub fn ssid(mut self, ssid: String) -> Self {
        self.ssid = Some(ssid);
        self
    }

    /// Sets the key management type for the wireless connection.
    pub fn key_mgmt(mut self, key_mgmt: String) -> Self {
        self.key_mgmt = Some(key_mgmt);
        self
    }

    /// Sets the pre-shared key (PSK) for the wireless connection.
    pub fn psk(mut self, psk: String) -> Self {
        self.psk = Some(psk);
        self
    }

    /// Builds the `WirelessConnectionSettings` from the builder.
    ///
    /// # Panics
    ///
    /// Panics if `bssid` or `ssid` are not set.
    pub fn build(self) -> WirelessConnectionSettings<'a> {
        let ssid = self.ssid.unwrap();
        let id = self.id.unwrap_or(ssid.clone());
        let psk = self.psk.unwrap_or("".to_string());
        let has_psk = !psk.is_empty();

        WirelessConnectionSettings {
            id: id.into(),
            type_: "802-11-wireless".into(),
            uuid: uuid::Uuid::new_v4().to_string().into(),
            autoconnect: true.into(),
            ssid: ssid.into_bytes().into(),
            key_mgmt: self.key_mgmt.unwrap_or("wpa-psk".to_string()).into(),
            psk: psk.into(),
            ipv4: "auto".into(),
            ipv6: "auto".into(),
            has_psk,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait::async_trait]
pub(in super::super) trait WirelessConnHelperExt {
        async fn fetch_profile(
        sender: &Sender<NetworkServiceInterEvent>,
        ap: &AccessPoint,
    ) -> Option<bool> {
        let (tx, rx) = smol::channel::unbounded();
        sender
            .send(NetworkServiceInterEvent::HasAPConnectionProfile {
                ssid: ap.ssid.clone(),
                key_mgmt: ap.key_management(),
                sender: tx,
            })
            .await
            .expect("Inter Service channel closed");

        rx.recv()
            .await
            .expect("Failed to receive profile check response")
    }

    async fn connect_with_auth(
        nm: &NetworkManagerProxy<'_>,
        ap: &AccessPoint,
        device: &OwnedObjectPath,
        has_profile: Option<bool>,
        psk: Option<String>,
    ) -> Result<OwnedObjectPath, AccessPointConnectResult> {
        if let Some(true) = has_profile {
            Ok(nm
                .activate_connection(&ObjectPath::try_from("/").unwrap(), device, &ap.dbus_path)
                .await
                .expect("Failed to activate connection"))
        } else {
            let psk = psk.ok_or(AccessPointConnectResult::Failed(
                NMActiveConnectionStateReason::LOGIN_FAILED,
            ))?;
            let settings = WirelessConnectionSettingsBuilder::new()
                .id(ap.ssid.clone())
                .ssid(ap.ssid.clone())
                .key_mgmt(ap.key_management().try_into().unwrap())
                .psk(psk)
                .build();

            let settings_proxy = SettingsProxy::new(&DBUS_CONNECTION)
                .await
                .expect("Failed to create settings proxy");
            let conn_path = settings_proxy
                .add_connection(settings.into_map())
                .await
                .expect("Failed to add connection");

            eprintln!("Connection added: {:?}", conn_path);
            Ok(nm
                .activate_connection(&conn_path, device, &ap.dbus_path)
                .await
                .expect("Failed to activate connection"))
        }
    }

    async fn connect_without_auth(
        nm: &NetworkManagerProxy<'_>,
        ap: &AccessPoint,
        device: &OwnedObjectPath,
    ) -> OwnedObjectPath {
        let settings = WirelessConnectionSettingsBuilder::new()
            .id(ap.ssid.clone())
            .ssid(ap.ssid.clone())
            .key_mgmt(ap.key_management().try_into().unwrap())
            .build();

        let (_conn_settings, path) = nm
            .add_and_activate_connection(settings.into_map(), device, &ap.dbus_path)
            .await
            .expect("Failed to add and activate connection");
        path
    }

    async fn wait_for_active(
        conn_path: OwnedObjectPath,
        auto_update: bool,
    ) -> AccessPointConnectResult {
        let active = ActiveProxy::new_from_path(conn_path, &DBUS_CONNECTION)
            .await
            .expect("Failed to create active proxy");
        let mut stream = active
            .receive_active_state_changed()
            .await
            .expect("Failed to listen for state changes");

        while let Some(signal) = stream.next().await {
            if let Ok(args) = signal.args() {
                if let Ok(state) = NMActiveConnectionState::try_from(args.state) {
                    match state {
                        NMActiveConnectionState::ACTIVATED => {
                            return AccessPointConnectResult::Connected;
                        }
                        NMActiveConnectionState::DEACTIVATED
                        | NMActiveConnectionState::DEACTIVATING
                        | NMActiveConnectionState::UNKNOWN => {
                            if auto_update {
                                Self::cleanup_connection(active).await;
                            }
                            return AccessPointConnectResult::Failed(
                                NMActiveConnectionStateReason::try_from(args.reason)
                                    .unwrap_or(NMActiveConnectionStateReason::UNKNOWN),
                            );
                        }
                        _ => continue,
                    }
                }
            }
        }

        if auto_update {
            Self::cleanup_connection(active).await;
        }
        AccessPointConnectResult::Failed(NMActiveConnectionStateReason::UNKNOWN)
    }

    async fn cleanup_connection(active: ActiveProxy<'_>) {
        if let Ok(path) = active.connection().await {
            if let Ok(conn) = SettingsConnectionProxy::new_from_path(path, &DBUS_CONNECTION).await {
                let _ = conn.delete().await;
            }
        }
    }
}

#[async_trait::async_trait]
pub(in super::super) trait WirelessConnExt: WirelessConnHelperExt {
    /// Attempts to connect to the specified access point.
    ///
    /// Handles both connecting to known profiles and adding new ones.
    /// If authentication is required and a PSK is provided, it will use it.
    ///
    /// # Arguments
    ///
    /// * `inter_sender` - Sender for internal service events.
    /// * `ap` - The `AccessPoint` to connect to.
    /// * `device_path` - The D-Bus path of the wireless device.
    /// * `auto_update` - Flag to indicate if the connection should be automatically updated.
    /// * `psk` - Optional pre-shared key for authentication.
    ///
    /// # Errors
    ///
    /// Returns `AccessPointConnectResult::AuthentiationRequired` if authentication is needed but no PSK is provided
    /// for a new connection or an existing connection that is not validated.
    async fn request_connect(
        inter_sender: Sender<NetworkServiceInterEvent>,
        ap: AccessPoint,
        device_path: OwnedObjectPath,
        auto_update: bool,
        psk: Option<String>,
    ) -> AccessPointConnectResult {
        let nm = NetworkManagerProxy::new(&DBUS_CONNECTION)
            .await
            .expect("Failed to create NetworkManager proxy");

        // 1. Fetch or create profile indicator
        let has_profile = Self::fetch_profile(&inter_sender, &ap).await;

        // 2. Determine activation path
        let active_conn = if ap.authentication_required() {
            Self::connect_with_auth(&nm, &ap, &device_path, has_profile, psk).await
        } else {
            Ok(Self::connect_without_auth(&nm, &ap, &device_path).await)
        };

        // 3. Wait for activation or fail
        match active_conn {
            Ok(conn_path) => {
                if let AccessPointConnectResult::Failed(e) =
                    Self::wait_for_active(conn_path.clone(), auto_update).await
                {
                    if auto_update {
                        let active = ActiveProxy::new_from_path(conn_path, &DBUS_CONNECTION)
                            .await
                            .expect("Failed to create active proxy");
                        Self::cleanup_connection(active).await;
                    }
                    AccessPointConnectResult::Failed(e)
                } else {
                    AccessPointConnectResult::Connected
                }
            }
            Err(err) => err,
        }
    }

    /// Disconnects a network device.
    ///
    /// # Arguments
    /// * `device_path` - The D-Bus object path of the device to disconnect.
    async fn disconnect(device_path: OwnedObjectPath) {
        let device = DeviceProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create device proxy for {:?}", device_path).as_str());
        device.disconnect().await.expect(
            format!(
                "Failed to send disconnect request for device: {}",
                device_path
            )
            .as_str(),
        );
    }
}

impl WirelessConnHelperExt for NetworkService {}
impl WirelessConnExt for NetworkService {}