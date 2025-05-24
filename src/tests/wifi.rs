use std::time::Duration;

use smol::Timer;

use crate::service::{event::EventListener, network::{endpoints::event::{NetworkServiceEvent, NetworkServiceEventType, NetworkServiceRequest, WiFiConnServiceMessage, WiFiConnServiceRequest, WiFiConnServiceResponse}, wireless::ap::AccessPointSecurity, NetworkService}};

async fn test_wifi_async() {
    let (sender, receiver) = smol::channel::unbounded::<NetworkServiceEvent>();
    let mut network_service = NetworkService::new();
    network_service.register_event_handler(NetworkServiceEventType::AccessPointScanReport, sender.clone());

    smol::spawn(async move {
        network_service.listen().await;
    }).detach();
    
    // Wait for the network service to start
    Timer::after(Duration::from_secs(1)).await;

    let command_sender = receiver.recv().await.unwrap();
    match command_sender {
        NetworkServiceEvent::HandlerRegistered { command_sender } => {
            let (event_sender, event_receiver) = smol::channel::unbounded();
            command_sender.send(NetworkServiceRequest::WiFiConnect { interface: "wlan0".to_string(), channel: event_sender }).await.unwrap();
            let accept = event_receiver.recv().await.unwrap();
            println!("Accept: {:?}", accept);
            assert!(matches!(accept, WiFiConnServiceMessage::Response(WiFiConnServiceResponse::ServerAcceptedConnection(_))));
            if let Some(WiFiConnServiceResponse::ServerAcceptedConnection(event_sender)) = accept.into_response() {
                event_sender.send(WiFiConnServiceRequest::WiFiConnect { ssid: "Test".to_string(), key_mgmt: AccessPointSecurity::WPA }.into_message()).await.unwrap();
                let event = event_receiver.recv().await.unwrap();
                assert!(matches!(event, WiFiConnServiceMessage::Response(WiFiConnServiceResponse::AuthentiationRequired)));
                event_sender.send(WiFiConnServiceRequest::ProvideAuthenticationInfo { psk: "test_wifi".to_string() }.into_message()).await.unwrap();
                
                let event = event_receiver.recv().await.unwrap();
                assert!(matches!(event, WiFiConnServiceMessage::Response(WiFiConnServiceResponse::RequestAcknowledged)));
            };
        }
        _ => {}
    }
}

#[test]
fn test_wifi() {
    smol::block_on(test_wifi_async());
}