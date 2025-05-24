use std::pin::Pin;

use futures_util::{FutureExt, Stream, StreamExt};
use rusty_network_manager::{DeviceProxy, NetworkManagerProxy};
use smol::channel::Sender;
use zbus::zvariant::OwnedObjectPath;

use super::{
    DBUS_CONNECTION, NetworkService, NetworkServiceInterEvent, WirelessWatchDogExt,
    endpoints::event::NetworkDeviceType, ethernet::EthernetWatchDogExt,
};

enum DeviceEvent {
    Added(zbus::zvariant::OwnedObjectPath),
    Removed(zbus::zvariant::OwnedObjectPath),
}

#[async_trait::async_trait]
pub trait NetworkServiceDeviceExt: WirelessWatchDogExt + EthernetWatchDogExt
where
    Self: 'static,
{
    /// Helper function to send a `RegisterInterface` internal event.
    /// This is typically called when a new device is detected.
    ///
    /// # Arguments
    /// * `sender` - The sender channel for internal service events.
    /// * `dbus_path` - The D-Bus object path of the device.
    /// * `interface` - The network interface name (e.g., "eth0", "wlan0").
    /// * `device_type` - The type of the network device.
    /// * `task` - A `smol::Task` that will manage the device (e.g., a watchdog).
    async fn register_interface(
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
                device_type,
                task,
            })
            .await
            .expect("Inter Service channel closed");
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
        // Get the device type and interface name.
        let interface = device_proxy
            .interface()
            .await
            .expect(format!("Failed to get interface for {:?}", device_path).as_str());

        let device_type = device_proxy
            .device_type()
            .await
            .ok()
            .and_then(|t| NetworkDeviceType::try_from(t).ok());

        // Register the device with the service
        match device_type {
            Some(NetworkDeviceType::Ethernet) => {
                // Register Ethernet device and spawn an Ethernet watchdog.
                Self::register_interface(
                    &sender,
                    device_path.to_string(),
                    interface.clone(),
                    NetworkDeviceType::Ethernet,
                    smol::spawn(Self::ethernet_watch_dog(sender.clone(), device_path)),
                )
                .await;
            }
            Some(NetworkDeviceType::WiFi) => {
                // Register Wi-Fi device and spawn a Wi-Fi watchdog.
                Self::register_interface(
                    &sender,
                    device_path.to_string(),
                    interface.clone(),
                    NetworkDeviceType::WiFi,
                    smol::spawn(Self::wifi_watchdog(sender.clone(), device_path)),
                )
                .await;
            }
            t => eprintln!(
                "NetworkService::add_device - Unknown device type: {:?} for interface {}",
                t, interface
            ),
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
        let streams: Vec<Pin<Box<dyn Stream<Item = DeviceEvent> + Send>>> = vec![
            nm.receive_device_added()
                .await
                .expect("Failed to subscribe to DeviceAdded signal.")
                .filter_map(|msg| async move {
                    match msg.args() {
                        Ok(sig) => Some(DeviceEvent::Added(sig.device_path.into())),
                        Err(e) => {
                            eprintln!("parse DeviceAdded error: {:?}", e);
                            None
                        }
                    }
                })
                .boxed(),
            nm.receive_device_removed()
                .await
                .expect("Failed to subscribe to DeviceRemoved signal.")
                .filter_map(|msg| async move {
                    match msg.args() {
                        Ok(sig) => Some(DeviceEvent::Removed(sig.device_path.into())),
                        Err(e) => {
                            eprintln!("parse DeviceAdded error: {:?}", e);
                            None
                        }
                    }
                })
                .boxed(),
        ];

        let mut streams = futures_util::stream::select_all(streams);
        while let Some(event) = streams.next().await {
            match event {
                DeviceEvent::Added(path) => {
                    Self::add_device(sender.clone(), path).await;
                }
                DeviceEvent::Removed(path) => {
                    sender
                        .send(NetworkServiceInterEvent::UnregisterInterface {
                            dbus_path: path.to_string(),
                        })
                        .await
                        .expect("Failed to send UnregisterInterface event");
                }
            }
        }
        eprintln!("NetworkService::watch_devices - Device watch task ended.");
    }
}

impl NetworkServiceDeviceExt for NetworkService {}
