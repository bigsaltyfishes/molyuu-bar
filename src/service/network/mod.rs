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

/// Represents the type of a network device.
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

/// Represents the state of a network device.
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

    /// Tries to convert a u32 value to a `NetworkDeviceState`.
    /// Maps unknown u32 values to `NetworkDeviceState::UnknownState`.
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

/// Represents the type of a network event.
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub enum NetworkEventType {
    DeviceAdded,
    DeviceRemoved,
    DeviceStateChanged,
    AccessPointScanReport,
}

/// Represents events related to the network service itself, primarily for communication
/// between the service and its clients during operations like Wi-Fi connection.
#[derive(Clone, Debug)]
pub enum NetworkServiceEvent {
    // Server Side
    /// Indicates that the server has accepted a connection request from a client.
    /// Contains a sender channel for the client to send further commands or information.
    ServerAcceptedConnection(Sender<NetworkServiceEvent>),
    /// Indicates that authentication is required for the current operation (e.g., Wi-Fi connection).
    AuthentiationRequired,
    /// Acknowledges that a client's request has been processed.
    RequestAcknowledged,

    // Client Side
    /// A client request to connect to a Wi-Fi network.
    WiFiConnect { ssid: String, hw_addr: HwAddress },
    /// Authentication information (e.g., PSK) provided by the client.
    AuthenticationInfo { psk: String },
}

/// Represents commands that can be sent to the `NetworkService`.
#[derive(Debug)]
pub enum NetworkCommand {
    /// Command to connect to a Wi-Fi network.
    /// Includes a channel for the service to communicate back with the requester.
    WiFiConnect {
        interface: String,
        channel: Sender<NetworkServiceEvent>,
    },
    /// Command to scan for Wi-Fi networks on a specific interface.
    WiFiScan {
        interface: String,
    },
    /// Command to disconnect from a Wi-Fi network on a specific interface.
    WiFiDisconnect {
        interface: String,
    },
}

/// Represents events that occur within the network service, to be broadcast to listeners.
#[derive(Clone, Debug)]
pub enum NetworkEvent {
    /// Indicates that a new network device has been added.
    DeviceAdded {
        interface: String,
        device_type: NetworkDeviceType,
    },
    /// Indicates that a network device has been removed.
    DeviceRemoved {
        interface: String,
    },
    /// Indicates that the state of a network device has changed.
    DeviceStateChanged {
        interface: String,
        state: NetworkDeviceState,
        reason: NMDeviceStateReason,
    },
    /// Reports the results of a Wi-Fi access point scan.
    AccessPointScanReport {
        interface: String,
        access_points: HashMap<HwAddress, AccessPoint>,
    },
}

/// Internal events used for communication within the `NetworkService` components.
#[derive(Debug)]
enum NetworkServiceInterEvent {
    /// Instructs the service to send a `NetworkEvent` to its listeners.
    SendMessage {
        event_type: NetworkEventType,
        event: NetworkEvent,
    },
    /// Registers a new network interface with the service.
    RegisterInterface {
        dbus_path: String,
        interface: String,
        device_type: NetworkDeviceType,
        task: smol::Task<()>, // Task for managing this interface (e.g., watchdog)
    },
    /// Unregisters an existing network interface from the service.
    UnregisterInterface {
        dbus_path: String,
    },
    /// Refreshes the list of access points for a given Wi-Fi interface.
    RefreshAccessPoints {
        interface: String,
        access_points: HashMap<HwAddress, AccessPoint>,
    },
    /// Requests information about a specific access point.
    GetAccessPoint {
        interface: String,
        hw_addr: HwAddress,
        sender: Sender<Option<AccessPoint>>, // Channel to send the AP info back
    },
    /// Requests the D-Bus path for a given network interface name.
    GetInterfaceDBusPath {
        interface: String,
        sender: Sender<Option<String>>, // Channel to send the D-Bus path back
    },
    /// Refreshes the known Wi-Fi connection profiles.
    RefreshAPConnections {
        map: HashMap<HwAddress, String>, // Map of HW Address to profile name/ID
    },
    /// Updates the validation status of a Wi-Fi connection profile.
    AssignProfileValidation {
        hw_addr: HwAddress,
        is_valid: bool,
    },
    /// Requests the connection profile (and its validation status) for a given AP.
    GetAPConnectionProfile {
        hw_addr: HwAddress,
        sender: Sender<(String, bool)>, // Channel to send profile and validity
    },
    /// Initiates an immediate Wi-Fi scan on the specified interface.
    ScanNow {
        interface: String,
    },
}

/// Stores the state and data for the `NetworkService`.
/// This includes registered interfaces, D-Bus mappings, and access point information.
#[derive(Debug, Default)]
pub struct NetworkServiceStorage {
    interfaces: HashMap<String, smol::Task<()>>, // Map of interface name to its management task
    dbus_interface_map: BiHashMap<String, String>, // Bidirectional map: D-Bus path <=> interface name
    interface_ap_map: HashMap<String, HashMap<HwAddress, AccessPoint>>, // Map of interface name to its APs
    ap_connection_map: HashMap<HwAddress, (String, bool)>, // Map of AP HW Address to (profile, is_valid)
}

impl NetworkServiceStorage {
    /// Creates a new, empty `NetworkServiceStorage`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new network interface.
    ///
    /// # Arguments
    /// * `interface` - The name of the interface (e.g., "wlan0").
    /// * `dbus_path` - The D-Bus object path for the interface.
    /// * `task` - The smol task associated with managing this interface.
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

    /// Unregisters a network interface by its name.
    /// Returns the D-Bus path of the unregistered interface if it existed.
    pub fn unregister_interface(&mut self, interface: &str) -> Option<String> {
        self.interfaces.remove(interface);
        if let Some((k, _)) = self.dbus_interface_map.remove_by_right(interface) {
            self.interface_ap_map.remove(interface);
            return Some(interface.to_string());
        }
        None
    }

    /// Unregisters a network interface by its D-Bus path.
    /// Returns the name of the unregistered interface if it existed.
    pub fn unregister_interface_by_dbus_path(&mut self, dbus_path: &str) -> Option<String> {
        if let Some((interface, _)) = self.dbus_interface_map.remove_by_left(dbus_path) {
            self.interfaces.remove(&interface);
            self.interface_ap_map.remove(&interface);
            return Some(interface);
        }
        None
    }

    /// Refreshes the list of known access points for a specific interface.
    pub fn refresh_access_points(
        &mut self,
        interface: String,
        access_points: HashMap<HwAddress, AccessPoint>,
    ) {
        self.interface_ap_map
            .insert(interface.clone(), access_points);
    }

    /// Tries to find an access point by its hardware address on a given interface.
    /// Returns a reference to the map of access points for the interface if the AP is found.
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

    /// Gets the D-Bus path for a given interface name.
    pub fn get_dbus_path_by_interface(&self, interface: &str) -> Option<String> {
        self.dbus_interface_map.get_by_left(interface).cloned()
    }

    /// Refreshes the map of AP hardware addresses to their connection profile names.
    /// Initially marks all profiles as valid.
    pub fn refresh_ap_connections(
        &mut self,
        map: HashMap<HwAddress, String>,
    ) {
        self.ap_connection_map = map.into_iter()
            .map(|(k, v)| (k, (v, true)))
            .collect();
    }

    /// Assigns a validation status to a connection profile associated with an AP.
    pub fn assign_profile_validation(
        &mut self,
        hw_addr: &HwAddress,
        is_valid: bool,
    ) {
        if let Some((_, valid)) = self.ap_connection_map.get_mut(hw_addr) {
            *valid = is_valid;
        }
    }

    /// Gets the connection profile name and its validation status for a given AP.
    pub fn get_ap_connection_profile(
        &self,
        hw_addr: &HwAddress,
    ) -> Option<(String, bool)> {
        self.ap_connection_map.get(hw_addr).cloned()
    }
}

/// Manages network connectivity, devices, and events.
/// It interacts with NetworkManager via D-Bus to monitor and control network interfaces.
pub struct NetworkService {
    handlers: HashMap<NetworkEventType, Vec<Sender<NetworkEvent>>>, // Event listeners
    inter_channel: (Sender<NetworkServiceInterEvent>, Receiver<NetworkServiceInterEvent>), // Internal communication
    command_channel: (Sender<NetworkCommand>, Receiver<NetworkCommand>), // For receiving external commands
    storage: NetworkServiceStorage, // Holds the service's state
}

impl NetworkService {
    /// Creates a new `NetworkService` instance.
    /// Initializes internal channels and storage.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            inter_channel: smol::channel::unbounded::<NetworkServiceInterEvent>(),
            command_channel: smol::channel::unbounded::<NetworkCommand>(),
            storage: NetworkServiceStorage::default(),
        }
    }

    /// Starts the network service, listening for internal events and commands.
    /// This is the main loop of the service.
    /// It spawns tasks for watching devices, syncing connections, and handling commands.
    pub async fn listen(&mut self) {
        // Spawn a task to watch for network device additions and removals.
        smol::spawn(Self::watch_devices(self.inter_channel.0.clone())).detach();
        // Spawn a task to synchronize Wi-Fi connection profiles.
        smol::spawn(Self::sync_connections(self.inter_channel.0.clone())).detach();
        // Spawn a task to handle incoming commands.
        smol::spawn(Self::command_endpoint(
            self.inter_channel.0.clone(),
            self.command_channel.1.clone(),
        )).detach();

        // Main event loop for processing internal messages.
        loop {
            let event = self.inter_channel.1.recv().await
                .expect("CRITICAL: Failed to receive from watchdog_channel in NetworkService::listen. This may indicate a fatal error in watch_devices.");

            match event {
                NetworkServiceInterEvent::SendMessage { event_type, event } => {
                    // Dispatch the network event to registered listeners.
                    self.send_msg(event_type, event).await;
                }
                NetworkServiceInterEvent::RegisterInterface {
                    dbus_path,
                    interface,
                    device_type,
                    task,
                } => {
                    // Register a new interface and notify listeners.
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
                    // Unregister an interface and notify listeners if it was found.
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
                    // Update the AP list for an interface and notify listeners.
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
                    // Request an immediate Wi-Fi scan on the specified interface.
                    if let Some(dbus_addr) = self.storage.get_dbus_path_by_interface(&interface) {
                        smol::spawn(Self::request_scan(
                            ObjectPath::try_from(dbus_addr.clone()).unwrap().into(),
                        ))
                        .detach();
                    }
                }
                NetworkServiceInterEvent::RefreshAPConnections { map } => {
                    // Update the storage with the latest AP connection profiles.
                    self.storage.refresh_ap_connections(map);
                }
                NetworkServiceInterEvent::GetAccessPoint { interface, hw_addr, sender } => {
                    // Retrieve and send access point details to the requester.
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
                    // Update the validation status of a specific AP's connection profile.
                    self.storage.assign_profile_validation(&hw_addr, is_valid);
                }
                NetworkServiceInterEvent::GetAPConnectionProfile { hw_addr, sender } => {
                    // Retrieve and send AP connection profile details to the requester.
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
                    // Retrieve and send the D-Bus path for an interface to the requester.
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

    /// Helper function to send a `RegisterInterface` internal event.
    /// This is typically called when a new device is detected.
    ///
    /// # Arguments
    /// * `sender` - The sender channel for internal service events.
    /// * `dbus_path` - The D-Bus object path of the device.
    /// * `interface` - The network interface name (e.g., "eth0", "wlan0").
    /// * `device_type` - The type of the network device.
    /// * `task` - A `smol::Task` that will manage the device (e.g., a watchdog).
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

    /// Processes a newly detected network device.
    /// It retrieves device details (interface name, type) via D-Bus and then
    /// calls `register_device` to register it with the service, spawning an
    /// appropriate watchdog task (Ethernet or Wi-Fi).
    ///
    /// # Arguments
    /// * `sender` - The sender channel for internal service events.
    /// * `device_path` - The D-Bus object path of the newly added device.
    async fn add_device(sender: Sender<NetworkServiceInterEvent>, device_path: OwnedObjectPath) {
        // Create a D-Bus proxy for the device.
        let device_proxy = DeviceProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create device proxy for {:?}", device_path).as_str());
        // Get the interface name (e.g., "eth0", "wlan0").
        let interface = device_proxy
            .interface()
            .await
            .expect(format!("Failed to get interface for {:?}", device_path).as_str());
        // Get the device type (e.g., Ethernet, Wi-Fi).
        let device_type_u32 = device_proxy
            .device_type()
            .await
            .expect("Failed to get device type");

        if let Ok(dev_type) = NetworkDeviceType::try_from(device_type_u32) {
            // Handle supported device types.
            match dev_type {
                NetworkDeviceType::Ethernet => {
                    // Register Ethernet device and spawn an Ethernet watchdog.
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
                    // Register Wi-Fi device and spawn a Wi-Fi watchdog.
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
                    // Log unsupported device types.
                    eprintln!(
                        "NetworkService::watch_devices - Device type {:?} is not supported for interface {}",
                        dev_type, interface
                    );
                }
            }
        } else {
            // Log unknown device type values.
            eprintln!(
                "NetworkService::watch_devices - Unknown device type value: {} for interface {}",
                device_type_u32, interface
            );
        }
    }

    /// Asynchronous task that listens for and processes `NetworkCommand`s.
    /// This endpoint allows external components to interact with the `NetworkService`.
    ///
    /// # Arguments
    /// * `inter_sender` - Sender for internal `NetworkServiceInterEvent`s, used to communicate
    /// with other parts of the `NetworkService`.
    /// * `command_receiver` - Receiver for incoming `NetworkCommand`s.
    async fn command_endpoint(
        inter_sender: Sender<NetworkServiceInterEvent>,
        command_receiver: Receiver<NetworkCommand>,
    ) {
        // Loop indefinitely, processing commands as they arrive.
        while let Ok(command) = command_receiver.recv().await {
            match command {
                NetworkCommand::WiFiConnect { interface, channel } => {
                    // Handle Wi-Fi connection requests.
                    let inter_sender = inter_sender.clone();
                    smol::spawn(async move {
                        // Create a new channel for this specific connection attempt.
                        let (event_sender, event_receiver) = smol::channel::unbounded::<NetworkServiceEvent>();
                        // Notify the requester that the connection process has started.
                        channel
                            .send(NetworkServiceEvent::ServerAcceptedConnection(event_sender))
                            .await
                            .expect("Failed to send connection event.");

                        let mut ap_storage = None; // To store AP details if auth is needed.
                        // Loop to handle messages from the client (e.g., SSID, PSK).
                        // Timeout if no message is received within 5 seconds.
                        while let Some(Ok(event)) = event_receiver.recv().timeout(Duration::from_secs(5)).await {
                            match event {
                                NetworkServiceEvent::WiFiConnect { ssid, hw_addr } => {
                                    // Client provided SSID and HW address.
                                    let (ap_sender, ap_reciver) = smol::channel::unbounded::<Option<AccessPoint>>();

                                    // Request AP details from the main service.
                                    inter_sender
                                        .send(NetworkServiceInterEvent::GetAccessPoint {
                                            interface: interface.clone(),
                                            hw_addr,
                                            sender: ap_sender,
                                        })
                                        .await
                                        .expect("Failed to send GetAccessPoint event.");

                                    if let Ok(Some(ap)) = ap_reciver.recv().await {
                                        // AP found, verify SSID.
                                        assert!(ap.ssid.contains(&ssid) && ap.ssid.len() == ssid.len(),
                                            "SSID mismatch: expected {}, got {}",
                                            ssid, ap.ssid
                                        );

                                        ap_storage = Some(ap.clone()); // Store AP for potential auth step.

                                        // Get D-Bus path for the interface.
                                        let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                                        inter_sender
                                            .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                                                interface: interface.clone(),
                                                sender: dbus_sender,
                                            })
                                            .await
                                            .expect("Failed to send GetInterfaceDBusPath event.");

                                        if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                                            // Attempt to connect to the AP.
                                            if let Err(ret) = Self::request_connect(inter_sender.clone(), ap, ObjectPath::try_from(dbus_path).unwrap().into(), None).await {
                                                if ret == AccessPointConnectError::AuthentiationRequired {
                                                    // Authentication is needed. Notify client.
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
                                    // Client provided PSK for authentication.
                                    if let Some(ap) = ap_storage.take() { // Retrieve stored AP.
                                        let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                                        // Get D-Bus path for the interface again.
                                        inter_sender
                                            .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                                                interface: interface.clone(),
                                                sender: dbus_sender,
                                            })
                                            .await
                                            .expect("Failed to send GetInterfaceDBusPath event.");

                                        if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                                            // Attempt to connect with the provided PSK.
                                            if let Err(ret) = Self::request_connect(inter_sender.clone(), ap, ObjectPath::try_from(dbus_path).unwrap().into(), Some(psk)).await {
                                                eprintln!("NetworkService::command_endpoint - Failed to connect to access point: {:?}", ret);
                                            }
                                            // Acknowledge the request, regardless of connection outcome.
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
                    // Handle Wi-Fi disconnection requests.
                    let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                    // Get D-Bus path for the interface.
                    inter_sender
                        .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                            interface: interface.clone(),
                            sender: dbus_sender,
                        })
                        .await
                        .expect("Failed to send GetInterfaceDBusPath event.");
                    if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                        // Spawn a task to perform the disconnection.
                        smol::spawn(Self::disconnect(ObjectPath::try_from(dbus_path).unwrap().into())).detach();
                    }
                }
                NetworkCommand::WiFiScan { interface } => {
                    // Handle Wi-Fi scan requests.
                    let (dbus_sender, dbus_reciver) = smol::channel::unbounded::<Option<String>>();
                    // Get D-Bus path for the interface.
                    inter_sender
                        .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                            interface: interface.clone(),
                            sender: dbus_sender,
                        })
                        .await
                        .expect("Failed to send GetInterfaceDBusPath event.");
                    if let Ok(Some(dbus_path)) = dbus_reciver.recv().await {
                        // Spawn a task to request a scan on the wireless device.
                        smol::spawn(Self::request_scan(ObjectPath::try_from(dbus_path).unwrap().into())).detach();
                    }
                }
            }
        }
    }

    /// Asynchronous task that watches for network device additions and removals using D-Bus signals
    /// from NetworkManager.
    ///
    /// When a device is added, it calls `add_device`.
    /// When a device is removed, it sends an `UnregisterInterface` internal event.
    ///
    /// # Arguments
    /// * `sender` - Sender channel for internal `NetworkServiceInterEvent`s.
    async fn watch_devices(sender: Sender<NetworkServiceInterEvent>) {
        // Create a D-Bus proxy for NetworkManager.
        let nm = NetworkManagerProxy::new(&DBUS_CONNECTION)
            .await
            .expect("CRITICAL: Failed to create NetworkManager proxy.");

        // Get all currently connected devices.
        let devices = nm
            .get_all_devices()
            .await
            .expect("CRITICAL: Failed to get devices from NetworkManager.");

        // Process initially detected devices.
        for device_path in devices {
            Self::add_device(sender.clone(), device_path.clone()).await;
        }

        // Subscribe to D-Bus signals for device addition and removal.
        let mut device_add_stream = nm
            .receive_device_added()
            .await
            .expect("Failed to subscribe to DeviceAdded signal.");
        let mut device_remove_stream = nm
            .receive_device_removed()
            .await
            .expect("Failed to subscribe to DeviceRemoved signal.");

        // Loop to process incoming D-Bus signals.
        loop {
            futures_util::select_biased! {
                // Handle DeviceAdded signal.
                device_add_signal = device_add_stream.next().fuse() => {
                    if let Some(device_add_signal) = device_add_signal {
                        match device_add_signal.args() {
                            Ok(arg) => {
                                // A new device was added, process it.
                                Self::add_device(sender.clone(), arg.device_path.into()).await;
                            }
                            Err(e) => {
                                eprintln!("NetworkService::watch_devices - Error parsing DeviceAdded signal args: {:?}", e);
                            }
                        }
                    } else {
                        eprintln!("DeviceAdded stream ended unexpectedly.");
                        break; // Exit loop if stream ends.
                    }
                }
                // Handle DeviceRemoved signal.
                device_remove_signal = device_remove_stream.next().fuse() => {
                    if let Some(device_remove_signal) = device_remove_signal {
                        match device_remove_signal.args() {
                            Ok(arg) => {
                                // A device was removed, send an unregister event.
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
                        break; // Exit loop if stream ends.
                    }
                }
                // This future completes if both streams have ended.
                complete => break,
            }
        }
    }

    /// Disconnects a network device.
    ///
    /// # Arguments
    /// * `device_path` - The D-Bus object path of the device to disconnect.
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

    /// Sends a `NetworkEvent` to all registered listeners for that event type.
    /// If a listener's channel is closed (send fails), it is removed.
    ///
    /// # Arguments
    /// * `event_type` - The type of the event to send.
    /// * `event` - The actual `NetworkEvent` data.
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
