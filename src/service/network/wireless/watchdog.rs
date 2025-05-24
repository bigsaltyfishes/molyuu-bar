use std::{collections::HashMap, pin::Pin};

use futures_util::{Stream, StreamExt};
use rusty_network_manager::{
    DeviceProxy, WirelessProxy, dbus_interface_types::NMDeviceStateReason,
};
use smol::channel::Sender;
use zbus::zvariant::OwnedObjectPath;

use crate::service::network::{
    endpoints::{event::{NetworkDeviceState, NetworkServiceEvent, NetworkServiceEventType}, inter::NetworkServiceInterEvent}, NetworkService, DBUS_CONNECTION
};

use super::ap::{AccessPoint, AccessPointSecurity};

/// Unified event type for the watchdog
enum WatchdogEvent {
    StateChanged(NetworkDeviceState, NMDeviceStateReason),
    ActiveApChanged(OwnedObjectPath),
    AccessPointsChanged(Vec<OwnedObjectPath>),
}

#[async_trait::async_trait]
pub(in super::super) trait WirelessWatchDogHelperExt {
    async fn emit(
        sender: &Sender<NetworkServiceInterEvent>,
        iface: &str,
        state: NetworkDeviceState,
        reason: NMDeviceStateReason,
    ) {
        sender
            .send(NetworkServiceInterEvent::SendMessage {
                event_type: NetworkServiceEventType::DeviceStateChanged,
                event: NetworkServiceEvent::DeviceStateChanged {
                    interface: iface.to_string(),
                    state,
                    reason,
                },
            })
            .await
            .expect("Inter Service channel closed");
    }
}

#[async_trait::async_trait]
pub(in super::super) trait WirelessWatchDogExt: WirelessWatchDogHelperExt {
    async fn wifi_watchdog(sender: Sender<NetworkServiceInterEvent>, path: OwnedObjectPath) {
        let conn = &*DBUS_CONNECTION;
        let device = DeviceProxy::new_from_path(path.clone(), conn)
            .await
            .expect("failed to create device proxy");
        let wireless = WirelessProxy::new_from_path(path.clone(), conn)
            .await
            .expect("failed to create wireless proxy");
        let interface = device.interface().await.expect("failed to get iface");

        // Initial state
        if let (Ok(s), Ok((_, r))) = (device.state().await, device.state_reason().await) {
            if let Ok(ds) = NetworkDeviceState::try_from(s) {
                let nr = NMDeviceStateReason::try_from(r).unwrap_or(NMDeviceStateReason::UNKNOWN);
                eprintln!("[init] {:?}@{} ({:?})", ds, interface, nr);
                Self::emit(&sender, &interface, ds, nr).await;
            }
        }

        // Map each signal stream into WatchdogEvent
        let streams: Vec<Pin<Box<dyn Stream<Item = WatchdogEvent> + Send>>> = vec![
            device
                .receive_state_changed()
                .await
                .filter_map(|sig| {
                    let device = device.clone();
                    let interface = interface.clone();
                    async move {
                        if let Some(ds) = sig
                            .get()
                            .await
                            .ok()
                            .and_then(|s| NetworkDeviceState::try_from(s).ok())
                        {
                            let r = device
                                .state_reason()
                                .await
                                .expect("failed to get state reason")
                                .1;
                            eprintln!("[state] {:?}@{} ({:?})", ds, interface, r);
                            Some(WatchdogEvent::StateChanged(
                                ds,
                                NMDeviceStateReason::try_from(r)
                                    .unwrap_or(NMDeviceStateReason::UNKNOWN),
                            ))
                        } else {
                            None
                        }
                    }
                })
                .boxed(),
            wireless
                .receive_active_access_point_changed()
                .await
                .filter_map(|sig| async move {
                    sig.get().await.ok().map(WatchdogEvent::ActiveApChanged)
                })
                .boxed(),
            wireless
                .receive_access_points_changed()
                .await
                .filter_map(|sig| async move {
                    sig.get().await.ok().map(WatchdogEvent::AccessPointsChanged)
                })
                .boxed(),
        ];

        let mut streams = futures_util::stream::select_all(streams);

        // Process merged events
        while let Some(evt) = streams.next().await {
            match evt {
                WatchdogEvent::StateChanged(ds, rs) => {
                    eprintln!("[state] {:?}@{} ({:?})", ds, interface, rs);
                    Self::emit(&sender, &interface, ds, rs).await;
                }
                WatchdogEvent::ActiveApChanged(ap) => {
                    // TODO: get and send active AP details
                    eprintln!("[active_ap] {:?}@{}", ap, interface);
                }
                WatchdogEvent::AccessPointsChanged(list) => {
                    // build map
                    let mut map: HashMap<(String, AccessPointSecurity), Vec<AccessPoint>> =
                        HashMap::new();
                    for p in list {
                        if let Some(ap) = AccessPoint::try_from_path(p.to_string()).await {
                            map.entry((ap.ssid.clone(), ap.key_management()))
                                .or_default()
                                .push(ap);
                        }
                    }
                    eprintln!("[aps] @{}", interface);
                    if sender
                        .send(NetworkServiceInterEvent::RefreshAccessPoints {
                            interface: interface.clone(),
                            access_points: map,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }
}

impl WirelessWatchDogHelperExt for NetworkService {}
impl WirelessWatchDogExt for NetworkService {}
