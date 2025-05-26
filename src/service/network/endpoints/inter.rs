use std::collections::{HashMap, HashSet};

use smol::channel::Sender;
use tracing::{error, info, instrument};
use zbus::zvariant::ObjectPath;

use crate::service::network::{
    ethernet::EthernetWatchDogExt, wireless::ap::{AccessPoint, AccessPointSecurity}, NetworkService, WirelessScanExt, WirelessWatchDogExt
};

use super::event::*;

/// Internal events used for communication within the `NetworkService` components.
#[derive(Debug)]
pub(in super::super) enum NetworkServiceInterEvent {
    /// Instructs the service to send a `NetworkEvent` to its listeners.
    SendMessage {
        event_type: NetworkServiceEventType,
        event: NetworkServiceEvent,
    },
    /// Registers a new network interface with the service.
    RegisterInterface {
        dbus_path: String,
        interface: String,
        device_type: NetworkDeviceType,
        task: smol::Task<()>, // Task for managing this interface (e.g., watchdog)
    },
    /// Unregisters an existing network interface from the service.
    UnregisterInterface { dbus_path: String },
    /// Refreshes the list of access points for a given Wi-Fi interface.
    RefreshAccessPoints {
        interface: String,
        access_points: HashMap<(String, AccessPointSecurity), Vec<AccessPoint>>,
    },
    /// Requests information about a specific access point.
    GetAccessPoints {
        interface: String,
        ssid: String,
        key_mgmt: AccessPointSecurity,
        sender: Sender<Option<Vec<AccessPoint>>>, // Channel to send the AP info back
    },
    /// Requests the D-Bus path for a given network interface name.
    GetInterfaceDBusPath {
        interface: String,
        sender: Sender<Option<String>>, // Channel to send the D-Bus path back
    },
    /// Refreshes the known Wi-Fi connection profiles.
    RefreshAPConnections {
        map: HashSet<(String, AccessPointSecurity)>, // Set of (SSID, KeyMgmt) pairs
    },
    /// Updates the validation status of a Wi-Fi connection profile.
    AssignProfileValidation {
        ssid: String,
        key_mgmt: AccessPointSecurity,
        is_valid: bool,
    },
    /// Check if a connection profile exists for a given access point.
    HasAPConnectionProfile {
        ssid: String,
        key_mgmt: AccessPointSecurity,
        sender: Sender<Option<bool>>, // Channel to send profile and validity
    },
    /// Initiates an immediate Wi-Fi scan on the specified interface.
    ScanNow { interface: String },
}

#[async_trait::async_trait]
pub(in super::super) trait NetworkServiceInterEndpointExt: WirelessWatchDogExt + EthernetWatchDogExt where Self: 'static {
    /// Handles incoming internal events for the network service.
    /// This function processes events such as device state changes, interface registration,
    /// and access point refresh requests.
    async fn inter_event_service(&mut self);
}

#[async_trait::async_trait]
impl NetworkServiceInterEndpointExt for NetworkService {
    #[instrument(skip_all)]
    async fn inter_event_service(&mut self) {
        while let Ok(event) = self.inter_channel.1.recv().await {
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
                    info!(
                        "Registering interface: {} ({:?})",
                        interface, device_type
                    );
                    self.storage
                        .register_interface(interface.clone(), dbus_path.clone(), task);
                    self.send_msg(
                        NetworkServiceEventType::DeviceAdded,
                        NetworkServiceEvent::DeviceAdded {
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
                        info!(
                            "Unregistered interface: {}",
                            interface
                        );
                        self.send_msg(
                            NetworkServiceEventType::DeviceRemoved,
                            NetworkServiceEvent::DeviceRemoved { interface },
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
                        NetworkServiceEventType::AccessPointScanReport,
                        NetworkServiceEvent::AccessPointScanReport {
                            interface: interface.clone(),
                            access_points: access_points
                                .keys()
                                .map(|(ssid, key_mgmt)| (ssid.clone(), key_mgmt.clone()))
                                .collect(),
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
                    info!("Refreshing AP connections.");
                    self.storage.refresh_ap_connections(map);
                }
                NetworkServiceInterEvent::GetAccessPoints {
                    interface,
                    ssid,
                    key_mgmt,
                    sender,
                } => {
                    // Retrieve and send access point details to the requester.
                    let ret = if let Some(map) = self
                        .storage
                        .try_find_access_points(&interface, &ssid, key_mgmt)
                    {
                        sender.send(Some(map.clone())).await
                    } else {
                        sender.send(None).await
                    };

                    if ret.is_err() {
                        info!("Failed to send access point.");
                    }
                }
                NetworkServiceInterEvent::AssignProfileValidation {
                    ssid,
                    key_mgmt,
                    is_valid,
                } => {
                    // Update the validation status of a specific AP's connection profile.
                    self.storage
                        .assign_profile_validation(&ssid, key_mgmt, is_valid);
                }
                NetworkServiceInterEvent::HasAPConnectionProfile {
                    ssid,
                    key_mgmt,
                    sender,
                } => {
                    // Retrieve and send AP connection profile details to the requester.
                    let ret = sender
                        .send(self.storage.has_ap_connection_profile(&ssid, key_mgmt))
                        .await;

                    if ret.is_err() {
                        error!("Failed to send AP connection profile.");
                    }
                }
                NetworkServiceInterEvent::GetInterfaceDBusPath { interface, sender } => {
                    // Retrieve and send the D-Bus path for an interface to the requester.
                    let ret = sender
                        .send(self.storage.get_dbus_path_by_interface(&interface))
                        .await;

                    if ret.is_err() {
                        error!("Failed to send DBus path.");
                    }
                }
            }
        }

        error!("CRITICAL ERROR: Channel closed.");
    }
}
