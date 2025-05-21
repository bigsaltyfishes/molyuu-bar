use std::collections::HashMap;

use futures_util::FutureExt;
use rusty_network_manager::{
    AccessPointProxy, DeviceProxy, NM80211ApSecurityFlags, NetworkManagerProxy,
    SettingsConnectionProxy, SettingsProxy, WirelessProxy,
    dbus_interface_types::NMDeviceStateReason,
};
use smol::{channel::Sender, stream::StreamExt};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

use super::{
    DBUS_CONNECTION, NetworkDeviceState, NetworkEvent, NetworkEventType, NetworkService,
    NetworkServiceInterEvent,
};

pub type HwAddress = String;

#[derive(Debug)]
pub struct WirelessConnectionSettings<'a> {
    pub id: Value<'a>,
    pub type_: Value<'a>,
    pub uuid: Value<'a>,
    pub autoconnect: Value<'a>,
    pub ssid: Value<'a>,
    pub bssid: Value<'a>,
    pub key_mgmt: Value<'a>,
    pub psk: Value<'a>,
    pub ipv4: Value<'a>,
    pub ipv6: Value<'a>,
    pub has_psk: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> WirelessConnectionSettings<'a> {
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
        wireless.insert("bssid", &self.bssid);

        let mut wireless_security = HashMap::new();
        wireless_security.insert("key_mgmt", &self.key_mgmt);

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

        settings
    }

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
        wireless.insert("bssid", self.bssid);

        let mut wireless_security = HashMap::new();
        wireless_security.insert("key_mgmt", self.key_mgmt);

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

        settings
    }
}

#[derive(Debug, Default)]
pub struct WirelessConnectionSettingsBuilder<'a> {
    id: Option<String>,
    ssid: Option<String>,
    bssid: Option<String>,
    key_mgmt: Option<String>,
    psk: Option<String>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> WirelessConnectionSettingsBuilder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }

    pub fn ssid(mut self, ssid: String) -> Self {
        self.ssid = Some(ssid);
        self
    }

    pub fn bssid(mut self, bssid: String) -> Self {
        self.bssid = Some(bssid);
        self
    }

    pub fn key_mgmt(mut self, key_mgmt: String) -> Self {
        self.key_mgmt = Some(key_mgmt);
        self
    }

    pub fn psk(mut self, psk: String) -> Self {
        self.psk = Some(psk);
        self
    }

    pub fn build(self) -> WirelessConnectionSettings<'a> {
        let bssid = self.bssid.unwrap();
        let ssid = self.ssid.unwrap();
        let id = self.id.unwrap_or(format!("{} Connection", ssid));
        let psk = self.psk.unwrap_or("".to_string());
        let has_psk = !psk.is_empty();

        WirelessConnectionSettings {
            id: id.into(),
            type_: "802-11-wireless".into(),
            uuid: uuid::Uuid::new_v4().to_string().into(),
            autoconnect: true.into(),
            ssid: ssid.into(),
            bssid: bssid.into(),
            key_mgmt: self.key_mgmt.unwrap_or("wpa-psk".to_string()).into(),
            psk: psk.into(),
            ipv4: "auto".into(),
            ipv6: "auto".into(),
            has_psk,
            _marker: std::marker::PhantomData,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessPointSecurity {
    None,
    WPA,
    WPA3,
    Unsupported,
}

impl AccessPointSecurity {
    /// Converts the security flags to an AccessPointSecurity enum.
    /// Returns None if the flags do not match any known security type or is not supported.
    /// Currently, only WPA3 and WPA/WPA2 Personal are supported.
    pub fn from_flags(flags: u32) -> Self {
        NM80211ApSecurityFlags::from_bits(flags)
            .map_or(None, |security| {
                security
                    .contains(NM80211ApSecurityFlags::NONE)
                    .then(|| Self::None)
                    .or_else(|| {
                        security
                            .contains(NM80211ApSecurityFlags::KEY_MGMT_SAE)
                            .then(|| Self::WPA3)
                    })
                    .or_else(|| {
                        security
                            .contains(NM80211ApSecurityFlags::KEY_MGMT_PSK)
                            .then(|| Self::WPA)
                    })
            })
            .unwrap_or(Self::Unsupported)
    }
}

impl TryInto<String> for AccessPointSecurity {
    type Error = ();
    fn try_into(self) -> Result<String, Self::Error> {
        match self {
            AccessPointSecurity::None => Ok("none".to_string()),
            AccessPointSecurity::WPA => Ok("wpa-psk".to_string()),
            AccessPointSecurity::WPA3 => Ok("sae".to_string()),
            AccessPointSecurity::Unsupported => Err(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AccessPointConnectError {
    AuthentiationRequired,
}

#[derive(Clone, Debug)]
pub struct AccessPoint {
    pub ssid: String,
    pub flags: u32,
    pub wpa_flags: u32,
    pub rsn_flags: u32,
    pub mode: u32,
    pub bssid: HwAddress,
    pub frequency: u32,
    pub signal_strength: u8,
    pub last_seen: i32,
    pub dbus_path: OwnedObjectPath,
}

impl AccessPoint {
    pub async fn try_from_path(path: String) -> Option<Self> {
        let path = ObjectPath::try_from(path.clone()).ok()?;
        let access_point = AccessPointProxy::new_from_path(path.clone().into(), &DBUS_CONNECTION)
            .await
            .ok()?;

        let flags = access_point.flags().await.ok()?;
        let wpa_flags = access_point.wpa_flags().await.ok()?;
        let rsn_flags = access_point.rsn_flags().await.ok()?;
        let ssid = access_point.ssid().await.ok()?;
        let ssid_str = String::from_utf8_lossy(&ssid).to_string();
        let freq = access_point.frequency().await.ok()?;
        let mode = access_point.mode().await.ok()?;
        let bssid = access_point.hw_address().await.ok()?;
        let signal_strength = access_point.strength().await.ok()?;
        let last_seen = access_point.last_seen().await.ok()?;

        Some(Self {
            ssid: ssid_str,
            flags,
            wpa_flags,
            rsn_flags,
            mode,
            bssid,
            frequency: freq,
            signal_strength,
            last_seen,
            dbus_path: path.into(),
        })
    }

    pub fn is_hidden(&self) -> bool {
        self.ssid.is_empty()
    }

    pub fn authentication_required(&self) -> bool {
        self.flags & 0x1 != 0
    }

    pub fn key_management(&self) -> AccessPointSecurity {
        let mut ret = AccessPointSecurity::from_flags(self.rsn_flags);
        if ret == AccessPointSecurity::Unsupported {
            ret = AccessPointSecurity::from_flags(self.wpa_flags);
        }
        ret
    }
}

impl NetworkService {
    pub(super) async fn request_scan(device_path: OwnedObjectPath) {
        let wireless = WirelessProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create wireless proxy for {:?}", device_path).as_str());
        wireless
            .request_scan(HashMap::new())
            .await
            .expect(format!("Failed to request scan for device: {}", device_path).as_str());
    }

    pub(super) async fn sync_connections(sender: Sender<NetworkServiceInterEvent>) {
        let settings_proxy = SettingsProxy::new(&DBUS_CONNECTION)
            .await
            .expect("Failed to create connection proxy");

        let connections = settings_proxy
            .list_connections()
            .await
            .expect("Failed to list connections");

        let filter_and_send = async move |connections: Vec<OwnedObjectPath>| {
            let mut connection_map = HashMap::new();
            for connection_path in connections {
                let connection_proxy = SettingsConnectionProxy::new_from_path(
                    connection_path.clone(),
                    &DBUS_CONNECTION,
                )
                .await
                .expect("Failed to create connection proxy");
                let settings = connection_proxy
                    .get_settings()
                    .await
                    .expect("Failed to get settings");
                if let Some(connection) = settings.get("connection") {
                    if let Some(connection_type) = connection.get("type") {
                        let connection_type = connection_type.to_string();
                        if connection_type == "802-11-wireless" {
                            if let Some(bssid) =
                                settings.get("802-11-wireless").and_then(|s| s.get("bssid"))
                            {
                                connection_map
                                    .insert(bssid.to_string(), connection_path.to_string());
                            }
                        }
                    }
                }
            }

            if !connection_map.is_empty() {
                sender
                    .send(NetworkServiceInterEvent::RefreshAPConnections {
                        map: connection_map,
                    })
                    .await
                    .expect("Failed to send refresh connections event");
            }
        };

        filter_and_send(connections).await;

        let mut connections_changed_stream = settings_proxy.receive_connections_changed().await;

        while let Some(signal) = connections_changed_stream.next().await {
            if let Ok(connections) = signal.get().await {
                filter_and_send(connections).await;
            }
        }
    }

    pub(super) async fn request_connect(
        inter_sender: Sender<NetworkServiceInterEvent>,
        ap: AccessPoint,
        device_path: OwnedObjectPath,
        psk: Option<String>,
    ) -> Result<(), AccessPointConnectError> {
        let nm = NetworkManagerProxy::new(&DBUS_CONNECTION)
            .await
            .expect("Failed to create NetworkManager proxy");

        let (profile_sender, profile_receiver) = smol::channel::unbounded();
        inter_sender
            .send(NetworkServiceInterEvent::GetAPConnectionProfile {
                hw_addr: ap.bssid.clone(),
                sender: profile_sender,
            })
            .await
            .expect("Failed to send GetAPConnectionProfile event");
        let (dbus_path, validation) = profile_receiver
            .recv()
            .await
            .expect("Failed to receive connection profile");

        if ap.authentication_required() {
            if let Some(psk) = psk {
                let settings = WirelessConnectionSettingsBuilder::new()
                    .id(format!("{} Connection", ap.ssid))
                    .ssid(ap.ssid.clone())
                    .bssid(ap.bssid.clone())
                    .key_mgmt(
                        ap.key_management()
                            .try_into()
                            .expect("Failed to convert key management"),
                    )
                    .psk(psk)
                    .build();

                if !dbus_path.is_empty() {
                    let connection_proxy = SettingsConnectionProxy::new_from_path(
                        ObjectPath::try_from(dbus_path.clone()).unwrap().into(),
                        &DBUS_CONNECTION,
                    )
                    .await
                    .expect(
                        format!("Failed to create connection proxy for {:?}", dbus_path).as_str(),
                    );

                    connection_proxy.update(settings.map_ref()).await.expect(
                        format!("Failed to update connection settings for {:?}", dbus_path)
                            .as_str(),
                    );

                    nm.activate_connection(
                        &ObjectPath::try_from(dbus_path).unwrap(),
                        &device_path,
                        &ap.dbus_path,
                    )
                    .await
                    .expect("Failed to activate connection");
                } else {
                    nm.add_and_activate_connection(
                        settings.into_map(),
                        &device_path,
                        &ap.dbus_path,
                    )
                    .await
                    .expect(
                        format!(
                            "Failed to add and activate connection for {:?}",
                            device_path
                        )
                        .as_str(),
                    );
                }
            } else if !validation {
                return Err(AccessPointConnectError::AuthentiationRequired);
            }
        } else {
            let settings = WirelessConnectionSettingsBuilder::new()
                .id(format!("{} Connection", ap.ssid))
                .ssid(ap.ssid.clone())
                .bssid(ap.bssid.clone())
                .key_mgmt(
                    ap.key_management()
                        .try_into()
                        .expect("Failed to convert key management"),
                )
                .build();

            if !dbus_path.is_empty() {
                let connection_proxy = SettingsConnectionProxy::new_from_path(
                    ObjectPath::try_from(dbus_path.clone()).unwrap().into(),
                    &DBUS_CONNECTION,
                )
                .await
                .expect(format!("Failed to create connection proxy for {:?}", dbus_path).as_str());

                connection_proxy.update(settings.map_ref()).await.expect(
                    format!("Failed to update connection settings for {:?}", dbus_path).as_str(),
                );

                nm.activate_connection(
                    &ObjectPath::try_from(dbus_path).unwrap(),
                    &device_path,
                    &ap.dbus_path,
                )
                .await
                .expect("Failed to activate connection");
            } else {
                nm.add_and_activate_connection(settings.into_map(), &device_path, &ap.dbus_path)
                    .await
                    .expect(
                        format!(
                            "Failed to add and activate connection for {:?}",
                            device_path
                        )
                        .as_str(),
                    );
            }
        }

        Ok(())
    }

    pub(super) async fn wifi_watch_dog(
        sender: Sender<NetworkServiceInterEvent>,
        device_path: OwnedObjectPath,
    ) {
        let device = DeviceProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create device proxy for {:?}", device_path).as_str());
        let wireless = WirelessProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create wireless proxy for {:?}", device_path).as_str());
        let device_interface = device
            .interface()
            .await
            .expect(format!("Failed to get interface for {:?}", device_path).as_str());

        // Send initial state
        let initial_state = device
            .state()
            .await
            .expect(format!("Failed to get state for {:?}", device_path).as_str());
        if let Ok(state_enum) = NetworkDeviceState::try_from(initial_state) {
            let reason = device
                .state_reason()
                .await
                .expect(format!("Failed to get state reason for {:?}", device_path).as_str())
                .1;
            eprintln!(
                "NetworkService::ethernet_watch_dog - Initial device state: {:?} for interface {}, reason: {:?}",
                state_enum, device_interface, reason
            );

            let ret = sender
                .send(NetworkServiceInterEvent::SendMessage {
                    event_type: NetworkEventType::DeviceStateChanged,
                    event: NetworkEvent::DeviceStateChanged {
                        interface: device_interface.clone(),
                        state: state_enum,
                        reason: NMDeviceStateReason::try_from(reason)
                            .unwrap_or(NMDeviceStateReason::UNKNOWN),
                    },
                })
                .await;

            if ret.is_err() {
                return;
            }
        }

        // TODO: Detect scan finished via LastScan signal
        let mut device_state_changed_stream = device.receive_state_changed().await;
        let mut active_access_point_changed_stream =
            wireless.receive_active_access_point_changed().await;
        let mut access_points_changed_stream = wireless.receive_access_points_changed().await;
        loop {
            futures_util::select_biased! {
                device_state_changed_signal = device_state_changed_stream.next().fuse() => {
                    if let Some(signal) = device_state_changed_signal {
                        if let Ok(state) = signal.get().await {
                            if let Ok(state_enum) = NetworkDeviceState::try_from(state) {
                                let reason = device.state_reason().await.expect(
                                    format!("Failed to get state reason for {:?}", device_path).as_str()
                                ).1;
                                eprintln!(
                                    "NetworkService::wifi_watch_dog - Device state changed: {:?} for interface {}, reason: {:?}",
                                    state_enum, device_interface, reason
                                );

                                let ret = sender
                                    .send(NetworkServiceInterEvent::SendMessage {
                                        event_type: NetworkEventType::DeviceStateChanged,
                                        event: NetworkEvent::DeviceStateChanged {
                                            interface: device_interface.clone(),
                                            state: state_enum,
                                            reason: NMDeviceStateReason::try_from(reason)
                                                .unwrap_or(NMDeviceStateReason::UNKNOWN),
                                        },
                                    })
                                    .await;

                                if ret.is_err() {
                                    break;
                                }
                            }
                        }
                    } else {
                        eprintln!(
                            "NetworkService::wifi_watch_dog - Device state changed stream closed for {:?}. Watchdog terminating.",
                            device_path
                        );
                        break;
                    }
                }
                active_access_point_changed_signal = active_access_point_changed_stream.next().fuse() => {
                    if let Some(signal) = active_access_point_changed_signal {
                        if let Ok(access_point) = signal.get().await {
                            eprintln!(
                                "NetworkService::wifi_watch_dog - Access point added: {:?} for interface {}",
                                access_point, device_interface
                            );
                        }
                    } else {
                        eprintln!(
                            "NetworkService::wifi_watch_dog - AccessPointAdded stream closed for {:?}. Watchdog terminating.",
                            device_path
                        );
                        break;
                    }
                }
                access_points_changed_signal = access_points_changed_stream.next().fuse() => {
                    if let Some(signal) = access_points_changed_signal {
                        if let Ok(access_points) = signal.get().await {
                            let mut access_points_map = HashMap::<HwAddress, AccessPoint>::new();
                            for ap in access_points {
                                if let Some(access_point) = AccessPoint::try_from_path(ap.to_string()).await {
                                    access_points_map.insert(access_point.bssid.clone(), access_point);
                                }
                            }
                            eprintln!(
                                "NetworkService::wifi_watch_dog - Access points changed for interface {}",
                                device_interface
                            );
                            let ret = sender.send(NetworkServiceInterEvent::RefreshAccessPoints { interface: device_interface.clone(), access_points: access_points_map }).await;
                            if ret.is_err() {
                                break;
                            }
                        }
                    } else {
                        eprintln!(
                            "NetworkService::wifi_watch_dog - AccessPointsChanged stream closed for {:?}. Watchdog terminating.",
                            device_path
                        );
                        break;
                    }
                }
            }
        }
    }
}
