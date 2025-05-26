use std::{collections::HashMap, pin::Pin};

use futures_util::{Stream, StreamExt};
use rusty_network_manager::{
    DeviceProxy, WirelessProxy, dbus_interface_types::NMDeviceStateReason,
};
use smol::channel::Sender;
use tracing::{info, instrument};
use zbus::zvariant::OwnedObjectPath;

use crate::service::network::{
    DBUS_CONNECTION, NetworkService,
    endpoints::{
        event::{NetworkDeviceState, NetworkServiceEvent, NetworkServiceEventType},
        inter::NetworkServiceInterEvent,
    },
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
    #[instrument(skip_all)]
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
                info!(
                    "Initial device state: {:?} for interface {}, state reason: {:?}",
                    ds, interface, nr
                );
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
                    info!(
                        "Device state changed: {:?} for interface {}, state reason: {:?}",
                        ds, interface, rs
                    );
                    // Emit the state change event
                    Self::emit(&sender, &interface, ds, rs).await;
                }
                WatchdogEvent::ActiveApChanged(ap) => {
                    if let Some(ap) = AccessPoint::try_from_path(ap.to_string()).await {
                        info!(
                            "Active access point changed for interface {}, SSID: {}, Security: {:?}",
                            interface,
                            ap.ssid,
                            ap.key_management()
                        );
                        if sender
                            .send(NetworkServiceInterEvent::SendMessage {
                                event_type: NetworkServiceEventType::ActiveAccessPointChanged,
                                event: NetworkServiceEvent::ActiveAccessPointChanged {
                                    interface: interface.clone(),
                                    ap,
                                },
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    } else {
                        info!("Failed to parse active access point from path: {}", ap);
                    }
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
                    info!(
                        "Access points changed for interface {}, num: {}",
                        interface,
                        map.len()
                    );
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
