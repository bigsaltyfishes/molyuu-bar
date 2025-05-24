use rusty_network_manager::{DeviceProxy, dbus_interface_types::NMDeviceStateReason};
use smol::{channel::Sender, stream::StreamExt};
use zbus::zvariant::OwnedObjectPath;

use crate::service::network::endpoints::event::{NetworkDeviceState, NetworkServiceEvent, NetworkServiceEventType};

use super::{endpoints::inter::NetworkServiceInterEvent, NetworkService, DBUS_CONNECTION};

#[async_trait::async_trait]
pub(in super::super) trait EthernetWatchDogExt {
    /// A watchdog function for a Ethernet device.
    ///
    /// Monitors device state changes.
    /// Sends events to the `NetworkService` to update its state accordingly.
    async fn ethernet_watch_dog(
        sender: Sender<NetworkServiceInterEvent>,
        device_path: OwnedObjectPath,
    ) {
        let device = DeviceProxy::new_from_path(device_path.clone(), &DBUS_CONNECTION)
            .await
            .expect(format!("Failed to create device proxy for {:?}", device_path).as_str());
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
                    event_type: NetworkServiceEventType::DeviceStateChanged,
                    event: NetworkServiceEvent::DeviceStateChanged {
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

        let mut device_state_changed_stream = device.receive_state_changed().await;
        while let Some(signal) = device_state_changed_stream.next().await {
            if let Ok(state) = signal.get().await {
                if let Ok(state_enum) = NetworkDeviceState::try_from(state) {
                    let reason = device
                        .state_reason()
                        .await
                        .expect(
                            format!("Failed to get state reason for {:?}", device_path).as_str(),
                        )
                        .1;
                    eprintln!(
                        "NetworkService::ethernet_watch_dog - Device state changed: {:?} for interface {}, reason: {:?}",
                        state_enum, device_interface, reason
                    );

                    let ret = sender
                        .send(NetworkServiceInterEvent::SendMessage {
                            event_type: NetworkServiceEventType::DeviceStateChanged,
                            event: NetworkServiceEvent::DeviceStateChanged {
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
        }
        eprintln!(
            "NetworkService::ethernet_watch_dog - Device state changed stream closed for {:?}. Watchdog terminating.",
            device_path
        );
    }
}

impl EthernetWatchDogExt for NetworkService {}