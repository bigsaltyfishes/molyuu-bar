use rusty_network_manager::{dbus_interface_types::NMActiveConnectionStateReason, AccessPointProxy, NM80211ApSecurityFlags};
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use crate::service::network::DBUS_CONNECTION;

pub type HwAddress = String;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// Represents the security type of a wireless access point.
pub enum AccessPointSecurity {
    None,
    WPA,
    WPA3,
    Unsupported,
}

impl AccessPointSecurity {
    /// Converts security flags (u32) to an `AccessPointSecurity` enum.
    ///
    /// It prioritizes WPA3, then WPA/WPA2 Personal, then None.
    /// If the flags do not match any known or supported security type, it returns `Unsupported`.
    pub fn from_flags(flags: u32) -> Self {
        NM80211ApSecurityFlags::from_bits(flags)
            .map_or(None, |security| {
                security.bits().eq(&NM80211ApSecurityFlags::NONE.bits()).then(|| Self::None)
                    .or_else(|| security.contains(NM80211ApSecurityFlags::KEY_MGMT_SAE).then(|| Self::WPA3))
                    .or_else(|| security.contains(NM80211ApSecurityFlags::KEY_MGMT_PSK).then(|| Self::WPA))
            })
            .unwrap_or(Self::Unsupported)
    }
}

impl TryInto<String> for AccessPointSecurity {
    type Error = ();
    /// Tries to convert `AccessPointSecurity` into a string representation suitable for NetworkManager.
    ///
    /// Returns `Err(())` if the security type is `Unsupported`.
    fn try_into(self) -> Result<String, Self::Error> {
        match self {
            AccessPointSecurity::None => Ok("none".to_string()),
            AccessPointSecurity::WPA => Ok("wpa-psk".to_string()),
            AccessPointSecurity::WPA3 => Ok("sae".to_string()),
            AccessPointSecurity::Unsupported => Err(()),
        }
    }
}

impl TryFrom<&str> for AccessPointSecurity {
    type Error = ();
    /// Tries to convert a string representation into `AccessPointSecurity`.
    ///
    /// Returns `Err(())` if the string does not match any known security type.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "none" => Ok(AccessPointSecurity::None),
            "wpa-psk" => Ok(AccessPointSecurity::WPA),
            "sae" => Ok(AccessPointSecurity::WPA3),
            _ => Err(()),
        }
    }
}

impl TryFrom<String> for AccessPointSecurity {
    type Error = ();
    /// Tries to convert a string representation into `AccessPointSecurity`.
    ///
    /// Returns `Err(())` if the string does not match any known security type.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Errors that can occur when trying to connect to an access point.
pub(in super::super) enum AccessPointConnectResult {
    Connected,
    Failed(NMActiveConnectionStateReason)
}

#[derive(Clone, Debug)]
/// Represents a wireless access point.
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
    /// Tries to create an `AccessPoint` from a D-Bus object path.
    ///
    /// Fetches access point details from D-Bus.
    /// Returns `None` if the path is invalid or fetching details fails.
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

    /// Checks if the access point has a hidden SSID.
    pub fn is_hidden(&self) -> bool {
        self.ssid.is_empty()
    }

    /// Checks if the access point requires authentication.
    pub fn authentication_required(&self) -> bool {
        self.key_management() != AccessPointSecurity::None
    }

    /// Determines the key management security type of the access point.
    /// It checks RSN flags first, then WPA flags.
    pub fn key_management(&self) -> AccessPointSecurity {
        let mut ret = AccessPointSecurity::from_flags(self.rsn_flags);
        if ret == AccessPointSecurity::Unsupported {
            ret = AccessPointSecurity::from_flags(self.wpa_flags);
        }
        ret
    }
}