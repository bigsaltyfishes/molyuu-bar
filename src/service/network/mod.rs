mod ethernet;
mod wireless;

use std::{collections::HashMap, hash::Hash, time::Duration};

use bimap::BiHashMap;
use futures_util::{FutureExt, TryFutureExt};
use num_enum::TryFromPrimitive;
use rusty_network_manager::{
    DeviceProxy, NetworkManagerProxy,
    dbus_interface_types::NMDeviceStateReason,
};
use smol::{
    channel::{Receiver, Sender},
    stream::StreamExt,
};
use smol_timeout::TimeoutExt;
use wireless::{AccessPoint, AccessPointConnectError, HwAddress};
use zbus::zvariant::ObjectPath;
use zbus::{Connection, zvariant::OwnedObjectPath};

use super::event;

lazy_static::lazy_static! {
    static ref DBUS_CONNECTION: Connection = smol::block_on(Connection::system())
        .expect("CRITICAL: Failed to connect to system bus. NetworkService cannot operate.");
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug, TryFromPrimitive)]
#[repr(u32)]
pub enum NetworkDeviceType {
    Ethernet = 1,
    WiFi = 2,
    Bluetooth = 5,
    OLPCMesh = 6,
    WiMax = 7,
    Modem = 8,
    Infiniband = 9,
    Bond = 10,
    VLAN = 11,
    ADSL = 12,
    Bridge = 13,
    Generic = 14,
    Team = 15,
    Tun = 16,
    IPTunnel = 17,
    MACVLAN = 18,
    VxLAN = 19,
    Veth = 20,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u32)]
pub enum NetworkDeviceState {
    Unknown = 0,
    Unmanaged = 10,
    Unavailable = 20,
    Disconnected = 30,
    Prepare = 40,
    Config = 50,
    NeedAuth = 60,
    IPConfig = 70,
    IPCheck = 80,
    Secondaries = 90,
    Activated = 100,
    Deactivating = 110,
    Failed = 120,
    UnknownState(u32),
}

impl TryFrom<u32> for NetworkDeviceState {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(NetworkDeviceState::Unknown),
            10 => Ok(NetworkDeviceState::Unmanaged),
            20 => Ok(NetworkDeviceState::Unavailable),
            30 => Ok(NetworkDeviceState::Disconnected),
            40 => Ok(NetworkDeviceState::Prepare),
            50 => Ok(NetworkDeviceState::Config),
            60 => Ok(NetworkDeviceState::NeedAuth),
            70 => Ok(NetworkDeviceState::IPConfig),
            80 => Ok(NetworkDeviceState::IPCheck),
            90 => Ok(NetworkDeviceState::Secondaries),
            100 => Ok(NetworkDeviceState::Activated),
            110 => Ok(NetworkDeviceState::Deactivating),
            120 => Ok(NetworkDeviceState::Failed),
            _ => Ok(NetworkDeviceState::UnknownState(value)),
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub enum NetworkEventType {
    DeviceAdded,
    DeviceRemoved,
    DeviceStateChanged,
    AccessPointScanReport,
}

#[derive(Clone, Debug)]
pub enum NetworkServiceEvent {
    // Server Side
    ServerAcceptedConnection(Sender<NetworkServiceEvent>),
    AuthentiationRequired,
    RequestAcknowledged,

    // Client Side
    WiFiConnect { ssid: String, hw_addr: HwAddress },
    AuthenticationInfo { psk: String },
}

#[derive(Debug)]
pub enum NetworkCommand {
    WiFiConnect {
        interface: String,
        channel: Sender<NetworkServiceEvent>,
    },
    WiFiScan {
        interface: String,
    },
    WiFiDisconnect {
        interface: String,
    },
}

#[derive(Clone, Debug)]
pub enum NetworkEvent {
    DeviceAdded {
        interface: String,
        device_type: NetworkDeviceType,
    },
    DeviceRemoved {
        interface: String,
    },
    DeviceStateChanged {
        interface: String,
        state: NetworkDeviceState,
        reason: NMDeviceStateReason,
    },
    AccessPointScanReport {
        interface: String,
        access_points: HashMap<HwAddress, AccessPoint>,
    },
}

#[derive(Debug)]
enum NetworkServiceInterEvent {
    SendMessage {
        event_type: NetworkEventType,
        event: NetworkEvent,
    },
    RegisterInterface {
        dbus_path: String,
        interface: String,
        device_type: NetworkDeviceType,
        task: smol::Task<()>,
    },
    UnregisterInterface {
        dbus_path: String,
    },
    RefreshAccessPoints {
        interface: String,
        access_points: HashMap<HwAddress, AccessPoint>,
    },
    GetAccessPoint {
        interface: String,
        hw_addr: HwAddress,
        sender: Sender<Option<AccessPoint>>,
    },
    GetInterfaceDBusPath {
        interface: String,
        sender: Sender<Option<String>>,
    },
    RefreshAPConnections {
        map: HashMap<HwAddress, String>,
    },
    AssignProfileValidation {
        hw_addr: HwAddress,
        is_valid: bool,
    },
    GetAPConnectionProfile {
        hw_addr: HwAddress,
        sender: Sender<(String, bool)>,
    },
    ScanNow {
        interface: String,
    },
}

#[derive(Debug, Default)]
pub struct NetworkServiceStorage {
    interfaces: HashMap<String, smol::Task<()>>,
    dbus_interface_map: BiHashMap<String, String>,
    interface_ap_map: HashMap<String, HashMap<HwAddress, AccessPoint>>,
    ap_connection_map: HashMap<HwAddress, (String, bool)>,
}

impl NetworkServiceStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_interface(
        &mut self,
        interface: String,
        dbus_path: String,
        task: smol::Task<()>,
    ) {
        self.interfaces.insert(interface.clone(), task);
        self.dbus_interface_map
            .insert(dbus_path.clone(), interface.clone());
    }

    pub fn unregister_interface(&mut self, interface: &str) -> Option<String> {
        self.interfaces.remove(interface);
        if let Some((k, _)) = self.dbus_interface_map.remove_by_right(interface) {
            self.interface_ap_map.remove(interface);
            return Some(interface.to_string());
        }
        None
    }

    pub fn unregister_interface_by_dbus_path(&mut self, dbus_path: &str) -> Option<String> {
        if let Some((interface, _)) = self.dbus_interface_map.remove_by_left(dbus_path) {
            self.interfaces.remove(&interface);
            self.interface_ap_map.remove(&interface);
            return Some(interface);
        }
        None
    }

    pub fn refresh_access_points(
        &mut self,
        interface: String,
        access_points: HashMap<HwAddress, AccessPoint>,
    ) {
        self.interface_ap_map
            .insert(interface.clone(), access_points);
    }

    pub fn try_find_access_points(
        &self,
        interface: &str,
        hw_addr: &HwAddress,
    ) -> Option<&HashMap<HwAddress, AccessPoint>> {
        self.interface_ap_map.get(interface).and_then(|map| {
            if map.contains_key(hw_addr) {
                Some(map)
            } else {
                None
            }
        })
    }

    pub fn get_dbus_path_by_interface(&self, interface: &str) -> Option<String> {
        self.dbus_interface_map.get_by_left(interface).cloned()
    }

    pub fn refresh_ap_connections(
        &mut self,
        map: HashMap<HwAddress, String>,
    ) {
        self.ap_connection_map = map.into_iter()
            .map(|(k, v)| (k, (v, true)))
            .collect();
    }

    pub fn assign_profile_validation(
        &mut self,
        hw_addr: &HwAddress,
        is_valid: bool,
    ) {
        if let Some((_, valid)) = self.ap_connection_map.get_mut(hw_addr) {
            *valid = is_valid;
        }
    }

    pub fn get_ap_connection_profile(
        &self,
        hw_addr: &HwAddress,
    ) -> Option<(String, bool)> {
        self.ap_connection_map.get(hw_addr).cloned()
    }
}

pub struct NetworkService {
    handlers: HashMap<NetworkEventType, Vec<Sender<NetworkEvent>>>,
    inter_channel: (Sender<NetworkServiceInterEvent>, Receiver<NetworkServiceInterEvent>),
    command_channel: (Sender<NetworkCommand>, Receiver<NetworkCommand>),
    storage: NetworkServiceStorage,
}

impl NetworkService {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            inter_channel: smol::channel::unbounded::<NetworkServiceInterEvent>(),
            command_channel: smol::channel::unbounded::<NetworkCommand>(),
            storage: NetworkServiceStorage::default(),
        }
    }

    pub async fn listen(&mut self) {
        // Spawn the device watcher task. If it panics, this service might become non-functional for new devices.
        smol::spawn(Self::watch_devices(self.inter_channel.0.clone())).detach();
        smol::spawn(Self::sync_connections(self.inter_channel.0.clone())).detach();
        smol::spawn(Self::command_endpoint(
            self.inter_channel.0.clone(),
            self.command_channel.1.clone(),
        )).detach();
        loop {
            let event = self.inter_channel.1.recv().await
                .expect("CRITICAL: Failed to receive from watchdog_channel in NetworkService::listen. This may indicate a fatal error in watch_devices.");

            match event {
                NetworkServiceInterEvent::SendMessage { event_type, event } => {
                    self.send_msg(event_type, event).await;
                }
                NetworkServiceInterEvent::RegisterInterface {
                    dbus_path,
                    interface,
                    device_type,
                    task,
                } => {
                    eprintln!(
                        "NetworkService::listen - Registering interface: {} ({:?})",
                        interface, device_type
                    );
                    self.storage
                        .register_interface(interface.clone(), dbus_path.clone(), task);
                    self.send_msg(
                        NetworkEventType::DeviceAdded,
                        NetworkEvent::DeviceAdded {
                            interface,
                            device_type,
                        },
                    )
                    .await;
                }
                NetworkServiceInterEvent::UnregisterInterface { dbus_path } => {
                    if let Some(interface) =
                        self.storage.unregister_interface_by_dbus_path(&dbus_path)
                    {
                        eprintln!(
                            "NetworkService::listen - Unregistered interface: {}",
                            interface
                        );
                        self.send_msg(
                            NetworkEventType::DeviceRemoved,
                            NetworkEvent::DeviceRemoved { interface },
                        )
                        .await;
                    }
                }
                NetworkServiceInterEvent::RefreshAccessPoints {
                    interface,
                    access_points,
                } => {
                    self.storage
                        .refresh_access_points(interface.clone(), access_points.clone());
                    self.send_msg(
                        NetworkEventType::AccessPointScanReport,
                        NetworkEvent::AccessPointScanReport {
                            interface: interface.clone(),
                            access_points: access_points.clone(),
                        },
                    )
                    .await;
                }
                NetworkServiceInterEvent::ScanNow { interface } => {
                    if let Some(dbus_addr) = self.storage.get_dbus_path_by_interface(&interface) {
                        smol::spawn(Self::request_scan(
                            ObjectPath::try_from(dbus_addr.clone()).unwrap().into(),
                        ))
                        .detach();
                    }
                }
                NetworkServiceInterEvent::RefreshAPConnections { map } => {
                    self.storage.refresh_ap_connections(map);
                }
                NetworkServiceInterEvent::GetAccessPoint { interface, hw_addr, sender } => {
                    let ret = if let Some(map) = self.storage.try_find_access_points(&interface, &hw_addr) {
                        sender.send(map.get(&hw_addr).cloned()).await
                    } else {
                        sender.send(None).await
                    };

                    if ret.is_err() {
                        eprintln!("NetworkService::listen - Failed to send access point.");
                    }
                }
                NetworkServiceInterEvent::AssignProfileValidation { hw_addr, is_valid } => {
                    self.storage.assign_profile_validation(&hw_addr, is_valid);
                }
                NetworkServiceInterEvent::GetAPConnectionProfile { hw_addr, sender } => {
                    let ret = if let Some((profile, is_valid)) = self.storage.get_ap_connection_profile(&hw_addr) {
                        sender.send((profile, is_valid)).await
                    } else {
                        sender.send((String::new(), false)).await
                    };

                    if ret.is_err() {
                        eprintln!("NetworkService::listen - Failed to send AP connection profile.");
                    }
                }
                NetworkServiceInterEvent::GetInterfaceDBusPath { interface, sender } => {
                    let ret = if let Some(dbus_path) = self.storage.get_dbus_path_by_interface(&interface) {
                        sender.send(Some(dbus_path)).await
                    } else {
                        sender.send(None).await
                    };

                    if ret.is_err() {
                        eprintln!("NetworkService::listen - Failed to send DBus path.");
                    }
                }
            }
        }
    }

    async fn register_device(
        sender: &Sender<NetworkServiceInterEvent>,
        dbus_path: String,
        interface: String,
        device_type: NetworkDeviceType,
        task: smol::Task<()>,
    ) {
        sender
                .send(NetworkServiceInterEvent::RegisterInterface {
                    dbus_path,
                    interface,
                    device_type: device_type,
                    task,
                })
                .await
                .expect(
                    "CRITICAL: Communication failed with NetworkService in NetworkService::register_device.",
                );
    }

    async fn add_device(sender: Sender<NetworkServiceInterEvent>, device_path: OwnedObjectPath) {
        let device_proxy = DeviceProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create device proxy for {:?}", device_path).as_str());
        let interface = device_proxy
            .interface()
            .await
            .expect(format!("Failed to get interface for {:?}", device_path).as_str());
        let device_type_u32 = device_proxy
            .device_type()
            .await
            .expect("Failed to get device type");
        if let Ok(dev_type) = NetworkDeviceType::try_from(device_type_u32) {
            match dev_type {
                NetworkDeviceType::Ethernet => {
                    Self::register_device(
                        &sender,
                        device_path.to_string(),
                        interface.clone(),
                        dev_type,
                        smol::spawn(NetworkService::ethernet_watch_dog(
                            sender.clone(),
                            device_path,
                        )),
                    )
                    .await;
                }
                NetworkDeviceType::WiFi => {
                    Self::register_device(
                        &sender,
                        device_path.to_string(),
                        interface.clone(),
                        dev_type,
                        smol::spawn(NetworkService::wifi_watch_dog(sender.clone(), device_path)),
                    )
                    .await;
                }
                _ => {
                    eprintln!(
                        "NetworkService::watch_devices - Device type {:?} is not supported for interface {}",
                        dev_type, interface
                    );
                }
            }
        } else {
            eprintln!(
                "NetworkService::watch_devices - Unknown device type value: {} for interface {}",
                device_type_u32, interface
            );
        }
    }

    async fn command_endpoint(
        inter_sender: Sender<NetworkServiceInterEvent>,
        command_receiver: Receiver<NetworkCommand>,
    ) {
        while let Ok(command) = command_receiver.recv().await {
            match command {
                NetworkCommand::WiFiConnect { interface, channel } => {
                    let inter_sender = inter_sender.clone();
                    smol::spawn(async move {
                        let (event_sender, event_receiver) = smol::channel::unbounded::<NetworkServiceEvent>();
                        channel
                            .send(NetworkServiceEvent::ServerAcceptedConnection(event_sender))
                            .await
                            .expect("Failed to send connection event.");

                        let mut ap_storage = None;
                        while let Some(Ok(event)) = event_receiver.recv().timeout(Duration::from_secs(5)).await {
                            match event {
                                NetworkServiceEvent::WiFiConnect { ssid, hw_addr } => {
                                    let (ap_sender, ap_reciver) = smol::channel::unbounded::<Option<AccessPoint>>();

                                    inter_sender
                                        .send(NetworkServiceInterEvent::GetAccessPoint {
                                            interface: interface.clone(),
                                            hw_addr,
                                            sender: ap_sender,
                                        })
                                        .await
                                        .expect("Failed to send GetAccessPoint event.");

                                    if let Ok(Some(ap)) = ap_reciver.recv().await {
                                        assert!(ap.ssid.contains(&ssid) && ap.ssid.len() == ssid.len(), 
                                            "SSID mismatch: expected {}, got {}",
                                            ssid, ap.ssid
                                        );
                                        
                                        ap_storage = Some(ap.clone());

                                        let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                                        inter_sender
                                            .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                                                interface: interface.clone(),
                                                sender: dbus_sender,
                                            })
                                            .await
                                            .expect("Failed to send GetInterfaceDBusPath event.");

                                        if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                                            if let Err(ret) = Self::request_connect(inter_sender.clone(), ap, ObjectPath::try_from(dbus_path).unwrap().into(), None).await {
                                                if ret == AccessPointConnectError::AuthentiationRequired {
                                                    channel
                                                        .send(NetworkServiceEvent::AuthentiationRequired)
                                                        .await
                                                        .expect("Failed to send AuthentiationRequired event.");
                                                } else {
                                                    eprintln!("NetworkService::command_endpoint - Failed to connect to access point: {:?}", ret);
                                                }
                                            }
                                        }
                                    }
                                }
                                NetworkServiceEvent::AuthenticationInfo { psk } => {
                                    if let Some(ap) = ap_storage.take() {
                                        let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                                        inter_sender
                                            .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                                                interface: interface.clone(),
                                                sender: dbus_sender,
                                            })
                                            .await
                                            .expect("Failed to send GetInterfaceDBusPath event.");

                                        if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                                            if let Err(ret) = Self::request_connect(inter_sender.clone(), ap, ObjectPath::try_from(dbus_path).unwrap().into(), Some(psk)).await {
                                                eprintln!("NetworkService::command_endpoint - Failed to connect to access point: {:?}", ret);
                                            }
                                            channel
                                                .send(NetworkServiceEvent::RequestAcknowledged)
                                                .await
                                                .expect("Failed to send RequestAcknowledged event.");
                                        }
                                    }
                                }
                                _ => {
                                    eprintln!("NetworkService::command_endpoint - Unhandled event: {:?}", event);
                                }
                            }
                        }
                    }).detach();
                }
                NetworkCommand::WiFiDisconnect { interface } => {
                    let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                    inter_sender
                        .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                            interface: interface.clone(),
                            sender: dbus_sender,
                        })
                        .await
                        .expect("Failed to send GetInterfaceDBusPath event.");
                    if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                        smol::spawn(Self::disconnect(ObjectPath::try_from(dbus_path).unwrap().into())).detach();
                    }
                }
                NetworkCommand::WiFiScan { interface } => {
                    let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                    inter_sender
                        .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                            interface: interface.clone(),
                            sender: dbus_sender,
                        })
                        .await
                        .expect("Failed to send GetInterfaceDBusPath event.");
                    if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                        smol::spawn(Self::request_scan(ObjectPath::try_from(dbus_path).unwrap().into())).detach();
                    }
                }
            }
        }
    }

    async fn watch_devices(sender: Sender<NetworkServiceInterEvent>) {
        let nm = NetworkManagerProxy::new(&DBUS_CONNECTION)
            .await
            .expect("CRITICAL: Failed to create NetworkManager proxy.");

        let devices = nm
            .get_all_devices()
            .await
            .expect("CRITICAL: Failed to get devices from NetworkManager.");

        for device_path in devices {
            Self::add_device(sender.clone(), device_path.clone()).await;
        }

        let mut device_add_stream = nm
            .receive_device_added()
            .await
            .expect("Failed to subscribe to DeviceAdded signal.");
        let mut device_remove_stream = nm
            .receive_device_removed()
            .await
            .expect("Failed to subscribe to DeviceRemoved signal.");

        loop {
            futures_util::select_biased! {
                device_add_signal = device_add_stream.next().fuse() => {
                    if let Some(device_add_signal) = device_add_signal {
                        match device_add_signal.args() {
                            Ok(arg) => {
                                Self::add_device(sender.clone(), arg.device_path.into()).await;
                            }
                            Err(e) => {
                                eprintln!("NetworkService::watch_devices - Error parsing DeviceAdded signal args: {:?}", e);
                            }
                        }
                    } else {
                        eprintln!("DeviceAdded stream ended unexpectedly.");
                        break;
                    }
                }
                device_remove_signal = device_remove_stream.next().fuse() => {
                    if let Some(device_remove_signal) = device_remove_signal {
                        match device_remove_signal.args() {
                            Ok(arg) => {
                                sender.send(NetworkServiceInterEvent::UnregisterInterface {
                                    dbus_path: arg.device_path.to_string(),
                                }).await.expect("CRITICAL: Communication failed with NetworkService when sending UnregisterInterface.");
                            }
                            Err(e) => {
                                eprintln!("NetworkService::watch_devices - Error parsing DeviceRemoved signal args: {:?}", e);
                            }
                        }
                    } else {
                        eprintln!("DeviceRemoved stream ended unexpectedly.");
                        break;
                    }
                }
                complete => break,
            }
        }
    }

    pub async fn disconnect(device_path: OwnedObjectPath) {
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

    async fn send_msg(&mut self, event_type: NetworkEventType, event: NetworkEvent) {
        let mut remove_key_if_empty = false;
        if let Some(senders_vec) = self.handlers.get_mut(&event_type) {
            let mut active_senders = Vec::with_capacity(senders_vec.len());
            for sender in senders_vec.drain(..) {
                // Drains the original vector
                if sender.send(event.clone()).await.is_ok() {
                    active_senders.push(sender);
                } else {
                    // Listener is removed because send failed (e.g., channel closed)
                    eprintln!(
                        "NetworkService::send_msg - A listener for {:?} was removed due to send failure.",
                        event_type
                    );
                }
            }
            *senders_vec = active_senders;

            if senders_vec.is_empty() {
                remove_key_if_empty = true;
            }
        }

        if remove_key_if_empty {
            self.handlers.remove(&event_type);
        }
    }
}
