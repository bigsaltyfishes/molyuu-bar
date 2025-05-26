use futures_util::StreamExt;
use rusty_network_manager::NetworkManagerProxy;
use smol::channel::Sender;
use tracing::{error, info, instrument};

use crate::service::network::{
    endpoints::event::{NetworkServiceEvent, NetworkServiceEventType}, NetworkService, NetworkServiceInterEvent, DBUS_CONNECTION
};

#[async_trait::async_trait]
pub(in super::super) trait RadioExt {
    #[instrument(skip_all)]
    async fn radio_watchdog(sender: Sender<NetworkServiceInterEvent>) {
        async fn emit(enabled: bool, sender: &Sender<NetworkServiceInterEvent>) {
            // This function can be used to emit an event or log the state change
            info!("Global wireless state changed: {}", enabled);
            sender
                .send(NetworkServiceInterEvent::SendMessage {
                    event_type: NetworkServiceEventType::GlobalWirelessEnabledStateChanged,
                    event: NetworkServiceEvent::GlobalWirelessEnabledStateChanged { enabled },
                })
                .await
                .expect("Inter Service channel closed");
        }
        let nm = NetworkManagerProxy::new(&DBUS_CONNECTION)
            .await
            .expect("Failed to create NetworkManager proxy");

        let initial = nm
            .wireless_enabled()
            .await
            .expect("Failed to get initial wireless state");
        emit(initial, &sender).await;

        let mut stream = nm
            .receive_wireless_enabled_changed()
            .await
            .filter_map(|sig| async move { sig.get().await.ok() })
            .boxed();

        while let Some(enabled) = stream.next().await {
            // State change detected
            emit(enabled, &sender).await;
        }

        error!("Radio monitoring unexpectedly stopped.");
    }

    async fn set_global_radio_state(enabled: bool) {
        let nm = NetworkManagerProxy::new(&DBUS_CONNECTION)
            .await
            .expect("Failed to create NetworkManager proxy");
        
        nm.set_wireless_enabled(enabled)
            .await
            .expect("Failed to set global wireless state");
    }
}

impl RadioExt for NetworkService {}