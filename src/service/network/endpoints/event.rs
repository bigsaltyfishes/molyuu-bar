use std::{collections::{HashMap, HashSet}, fmt::Debug};

use num_enum::TryFromPrimitive;
use rusty_network_manager::dbus_interface_types::NMDeviceStateReason;
use smol::channel::Sender;

use crate::service::network::wireless::ap::{AccessPoint, AccessPointSecurity};

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
pub enum NetworkServiceEventType {
    DeviceAdded,
    DeviceRemoved,
    DeviceStateChanged,
    AccessPointScanReport,
}

pub enum WiFiConnServiceRequest {
    /// A client request to connect to a Wi-Fi network.
    WiFiConnect { ssid: String, key_mgmt: AccessPointSecurity },
    /// Authentication information (e.g., PSK) provided by the client.
    ProvideAuthenticationInfo { psk: String },
}

impl WiFiConnServiceRequest {
    pub fn into_message(self) -> WiFiConnServiceMessage {
        WiFiConnServiceMessage::Request(self)
    }
}

impl Debug for WiFiConnServiceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WiFiConnect { ssid, key_mgmt } => {
                write!(f, "WiFiConnect {{ ssid: {}, key_mgmt: {:?} }}", ssid, key_mgmt)
            }
            Self::ProvideAuthenticationInfo { psk: _ } => {
                write!(f, "ProvideAuthenticationInfo {{ psk: {{ ... }} }}")
            }
        }
    }
}

#[derive(Debug)]
pub enum WiFiConnServiceResponse {
    /// Indicates that the server has accepted a connection request from a client.
    /// Contains a sender channel for the client to send further commands or information.
    ServerAcceptedConnection(Sender<WiFiConnServiceMessage>),
    /// Indicates that authentication is required for the current operation (e.g., Wi-Fi connection).
    AuthentiationRequired,
    /// Acknowledges that a client's request has been processed.
    RequestAcknowledged,
}

impl WiFiConnServiceResponse {
    pub fn into_message(self) -> WiFiConnServiceMessage {
        WiFiConnServiceMessage::Response(self)
    }
}

#[derive(Debug)]
pub enum WiFiConnServiceMessage {
    Request(WiFiConnServiceRequest),
    Response(WiFiConnServiceResponse),
}

impl WiFiConnServiceMessage {
    pub fn into_request(self) -> Option<WiFiConnServiceRequest> {
        if let Self::Request(request) = self {
            Some(request)
        } else {
            None
        }
    }

    pub fn into_response(self) -> Option<WiFiConnServiceResponse> {
        if let Self::Response(response) = self {
            Some(response)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum NetworkServiceRequest {
    /// Request to connect to a Wi-Fi network.
    /// Includes a channel for the service to communicate back with the requester.
    WiFiConnect {
        interface: String,
        channel: Sender<WiFiConnServiceMessage>,
    },
    /// Request to scan for Wi-Fi networks on a specific interface.
    WiFiScan {
        interface: String,
    },
    /// Request to disconnect from a Wi-Fi network on a specific interface.
    WiFiDisconnect {
        interface: String,
    },
}

/// Represents events that occur within the network service, 
/// to be broadcast to listeners.
#[derive(Clone, Debug)]
pub enum NetworkServiceEvent {
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
        access_points: HashSet<(String, AccessPointSecurity)>,
    },
    /// Return a command sender for registering event handlers.
    HandlerRegistered {
        command_sender: Sender<NetworkServiceRequest>,
    }
}