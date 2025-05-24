pub mod ethernet;
pub mod wireless;
pub mod endpoints;
pub mod devices;

use std::{collections::{HashMap, HashSet}, hash::Hash, time::Duration};

use bimap::BiHashMap;
use futures_util::{FutureExt, TryFutureExt};
use num_enum::TryFromPrimitive;
use rusty_network_manager::{
    dbus_interface_types::{NMActiveConnectionStateReason, NMDeviceStateReason}, DeviceProxy, NetworkManagerProxy
};
use smol::{
    channel::{Receiver, Sender},
    stream::StreamExt,
};
use smol_timeout::TimeoutExt;
use zbus::zvariant::ObjectPath;
use zbus::{Connection, zvariant::OwnedObjectPath};

use wireless::prelude::*;

use devices::NetworkServiceDeviceExt;
use endpoints::{event::*, inter::{NetworkServiceInterEndpointExt, NetworkServiceInterEvent}, command::NetworkServiceCommandEndpointExt};

use super::event::EventListener;

lazy_static::lazy_static! {
    static ref DBUS_CONNECTION: Connection = smol::block_on(Connection::system())
        .expect("CRITICAL: Failed to connect to system bus. NetworkService cannot operate.");
}

/// Stores the state and data for the `NetworkService`.
/// This includes registered interfaces, D-Bus mappings, and access point information.
#[derive(Debug, Default)]
pub struct NetworkServiceStorage {
    interfaces: HashMap<String, smol::Task<()>>, // Map of interface name to its management task
    dbus_interface_map: BiHashMap<String, String>, // Bidirectional map: D-Bus path <=> interface name
    interface_ap_map: HashMap<String, HashMap<(String, AccessPointSecurity), Vec<AccessPoint>>>, // Map of interface name to its APs
    ap_connection_map: HashMap<(String, AccessPointSecurity), bool>, // Map of profile exists AP (SSID, KeyMgmt) to profile validation status
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
        access_points: HashMap<(String, AccessPointSecurity), Vec<AccessPoint>>,
    ) {
        self.interface_ap_map
            .insert(interface.clone(), access_points);
    }

    /// Tries to find an access point by its hardware address on a given interface.
    /// Returns a reference to the map of access points for the interface if the AP is found.
    pub fn try_find_access_points(
        &self,
        interface: &str,
        ssid: &str,
        key_mgmt: AccessPointSecurity,
    ) -> Option<&Vec<AccessPoint>> {
        self.interface_ap_map.get(interface).and_then(|map| {
            map.get(&(ssid.to_string(), key_mgmt))
        })
    }

    /// Gets the D-Bus path for a given interface name.
    pub fn get_dbus_path_by_interface(&self, interface: &str) -> Option<String> {
        self.dbus_interface_map.get_by_right(interface).cloned()
    }

    /// Refreshes the map of AP hardware addresses to their connection profile names.
    /// Initially marks all profiles as valid.
    pub fn refresh_ap_connections(
        &mut self,
        map: HashSet<(String, AccessPointSecurity)>,
    ) {
        self.ap_connection_map = map.into_iter()
            .map(|k| (k, true))
            .collect();
    }

    /// Assigns a validation status to a connection profile associated with an AP.
    pub fn assign_profile_validation(
        &mut self,
        ssid: &str,
        key_mgmt: AccessPointSecurity,
        is_valid: bool,
    ) {
        if let Some(valid) = self.ap_connection_map.get_mut(&(ssid.to_string(), key_mgmt)) {
            *valid = is_valid;
        }
    }

    /// Check if a connection profile exists for a given access point.
    /// 
    /// # Arguments
    /// 
    /// * `ssid` - The SSID of the access point.
    /// * `key_mgmt` - The key management type of the access point.
    /// 
    /// # Returns
    /// 
    /// * `Some(true)` if the profile exists and is valid.
    /// * `Some(false)` if the profile exists but is invalid.
    /// * `None` if the profile does not exist.
    pub fn has_ap_connection_profile(
        &self,
        ssid: &str,
        key_mgmt: AccessPointSecurity,        
    ) -> Option<bool> {
        self.ap_connection_map.get(&(ssid.to_string(), key_mgmt)).cloned()
    }
}

/// Manages network connectivity, devices, and events.
/// It interacts with NetworkManager via D-Bus to monitor and control network interfaces.
pub struct NetworkService {
    handlers: HashMap<NetworkServiceEventType, Vec<Sender<NetworkServiceEvent>>>, // Event listeners
    inter_channel: (Sender<NetworkServiceInterEvent>, Receiver<NetworkServiceInterEvent>), // Internal communication
    command_channel: (Sender<NetworkServiceRequest>, Receiver<NetworkServiceRequest>), // For receiving external commands
    storage: NetworkServiceStorage, // Holds the service's state
}

impl NetworkService {
    /// Creates a new `NetworkService` instance.
    /// Initializes internal channels and storage.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            inter_channel: smol::channel::unbounded::<NetworkServiceInterEvent>(),
            command_channel: smol::channel::unbounded::<NetworkServiceRequest>(),
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

        self.inter_event_service().await;
    }

    /// Sends a `NetworkServiceEvent` to all registered listeners for that event type.
    /// If a listener's channel is closed (send fails), it is removed.
    ///
    /// # Arguments
    /// * `event_type` - The type of the event to send.
    /// * `event` - The actual `NetworkServiceEvent` data.
    async fn send_msg(&mut self, event_type: NetworkServiceEventType, event: NetworkServiceEvent) {
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

impl EventListener<NetworkServiceEventType, NetworkServiceEvent> for NetworkService {
    fn register_event_handler(&mut self, event_type: NetworkServiceEventType, sender: Sender<NetworkServiceEvent>) {
        smol::block_on(sender.send(NetworkServiceEvent::HandlerRegistered {
            command_sender: self.command_channel.0.clone(),
        })).expect("Handler registration failed.");
        self.handlers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(sender);
    }
}