use std::time::Duration;

use rusty_network_manager::dbus_interface_types::NMActiveConnectionStateReason;
use smol::channel::{Receiver, Sender};
use smol_timeout::TimeoutExt;
use zbus::zvariant::ObjectPath;

use crate::service::network::{
    endpoints::event::{WiFiConnServiceMessage, WiFiConnServiceResponse}, wireless::ap::{AccessPoint, AccessPointSecurity}, AccessPointConnectResult, NetworkService, WirelessConnExt, WirelessScanExt
};

use super::{
    event::{NetworkServiceRequest, WiFiConnServiceRequest},
    inter::NetworkServiceInterEvent,
};

#[async_trait::async_trait]
pub(in super::super) trait NetworkServiceCommandEndpointHelperExt:
    WirelessConnExt + WirelessScanExt
{
    async fn handle_connect(
        inter_sender: Sender<NetworkServiceInterEvent>,
        interface: String,
        client_chan: Sender<WiFiConnServiceMessage>,
    ) {
        // setup event channel
        let (evt_tx, evt_rx) = smol::channel::unbounded();
        // notify client
        let _ = client_chan
            .send(WiFiConnServiceMessage::Response(
                WiFiConnServiceResponse::ServerAcceptedConnection(evt_tx.clone()),
            ))
            .await;

        let mut aps: Vec<AccessPoint> = Vec::new();
        // loop for client events with timeout
        while let Some(Ok(evt)) = evt_rx.recv().timeout(Duration::from_secs(30)).await {
            match evt.into_request() {
                Some(WiFiConnServiceRequest::WiFiConnect { ssid, key_mgmt }) => {
                    eprintln!("[debug] Connecting to SSID: {}", ssid);
                    if let Some(ap_list) =
                        Self::get_access_points(&inter_sender, &interface, ssid, key_mgmt).await
                    {
                        eprintln!("[debug] Found access points: {:?}", ap_list);
                        aps = ap_list;
                        Self::try_connect(&inter_sender, &interface, None, &aps, &client_chan)
                            .await;
                    } else {
                        eprintln!("[debug] No access points found");
                        break;
                    }
                }
                Some(WiFiConnServiceRequest::ProvideAuthenticationInfo { psk }) => {
                    Self::try_connect(&inter_sender, &interface, Some(psk), &aps, &client_chan)
                        .await;
                }
                e => eprintln!("Unhandled event: {:?}", e),
            }
        }
    }

    async fn handle_disconnect(inter_sender: Sender<NetworkServiceInterEvent>, interface: String) {
        if let Some(path) = Self::get_dbus_path(&inter_sender, &interface).await {
            let _ = Self::disconnect(ObjectPath::try_from(path).unwrap().into()).await;
        }
    }

    async fn handle_scan(inter_sender: Sender<NetworkServiceInterEvent>, interface: String) {
        if let Some(path) = Self::get_dbus_path(&inter_sender, &interface).await {
            let _ = Self::request_scan(ObjectPath::try_from(path).unwrap().into()).await;
        }
    }

    async fn get_dbus_path(
        inter_sender: &Sender<NetworkServiceInterEvent>,
        interface: &str,
    ) -> Option<String> {
        let (tx, rx) = smol::channel::unbounded::<Option<String>>();
        let _ = inter_sender
            .send(NetworkServiceInterEvent::GetInterfaceDBusPath {
                interface: interface.to_string(),
                sender: tx,
            })
            .await;
        rx.recv().await.unwrap_or(None)
    }

    async fn get_access_points(
        inter_sender: &Sender<NetworkServiceInterEvent>,
        interface: &str,
        ssid: String,
        key_mgmt: AccessPointSecurity,
    ) -> Option<Vec<AccessPoint>> {
        let (tx, rx) = smol::channel::unbounded();
        let _ = inter_sender
            .send(NetworkServiceInterEvent::GetAccessPoints {
                interface: interface.to_string(),
                ssid,
                key_mgmt,
                sender: tx,
            })
            .await;
        rx.recv().await.unwrap_or(None)
    }

    async fn try_connect(
        inter_sender: &Sender<NetworkServiceInterEvent>,
        interface: &str,
        psk: Option<String>,
        aps: &[AccessPoint],
        client_chan: &Sender<WiFiConnServiceMessage>,
    ) {
        if let Some(dbus_path) = Self::get_dbus_path(inter_sender, interface).await {
            let obj = ObjectPath::try_from(dbus_path).unwrap();
            for ap in aps.iter() {
                match Self::request_connect(
                    inter_sender.clone(),
                    ap.clone(),
                    obj.into(),
                    aps.len() == 1,
                    psk.clone(),
                )
                .await
                {
                    AccessPointConnectResult::Connected => {
                        let _ = client_chan
                            .send(WiFiConnServiceMessage::Response(
                                WiFiConnServiceResponse::RequestAcknowledged,
                            ))
                            .await;
                        return;
                    }
                    AccessPointConnectResult::Failed(_) => {
                        let _ = client_chan
                            .send(WiFiConnServiceMessage::Response(
                                WiFiConnServiceResponse::AuthentiationRequired,
                            ))
                            .await;
                        return;
                    }
                }
            }
            eprintln!("Connection attempt failed for all APs");
        }
    }
}

#[async_trait::async_trait]
pub(in super::super) trait NetworkServiceCommandEndpointExt:
    NetworkServiceCommandEndpointHelperExt
where
    Self: 'static,
{
    /// Asynchronous task that listens for and processes `NetworkServiceRequest`s.
    /// This endpoint allows external components to interact with the `NetworkService`.
    ///
    /// # Arguments
    /// * `inter_sender` - Sender for internal `NetworkServiceInterEvent`s, used to communicate
    /// with other parts of the `NetworkService`.
    /// * `command_receiver` - Receiver for incoming `NetworkServiceRequest`s.
    async fn command_endpoint(
        inter_sender: Sender<NetworkServiceInterEvent>,
        command_receiver: Receiver<NetworkServiceRequest>,
    ) {
        while let Ok(command) = command_receiver.recv().await {
            eprintln!("Received command: {:?}", command);
            match command {
                NetworkServiceRequest::WiFiConnect { interface, channel } => {
                    let inter = inter_sender.clone();
                    smol::spawn(Self::handle_connect(inter, interface, channel)).detach();
                }
                NetworkServiceRequest::WiFiDisconnect { interface } => {
                    let inter = inter_sender.clone();
                    smol::spawn(Self::handle_disconnect(inter, interface)).detach();
                }
                NetworkServiceRequest::WiFiScan { interface } => {
                    let inter = inter_sender.clone();
                    smol::spawn(Self::handle_scan(inter, interface)).detach();
                }
            }
        }
    }
}

impl NetworkServiceCommandEndpointHelperExt for NetworkService {}
impl NetworkServiceCommandEndpointExt for NetworkService {}