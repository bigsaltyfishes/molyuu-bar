mod power;
mod datetime;
mod network;

use std::os::unix::net;

use gtk4::{
    Box,
    prelude::{BoxExt, WidgetExt},
};
use power::Power;

use crate::service::{event::{EventHandler, EventHandlerMutExt}, network::NetworkService};

pub struct Panel(Box);

impl Panel {
    pub fn new() -> Self {
        let panel = Box::new(gtk4::Orientation::Horizontal, 4);
        let datetime = datetime::DateTime::new();
        let mut network = network::Network::new();
        let power = Power::new();
        let mut network_service = NetworkService::new();
        network.register_to_listener(&mut network_service);

        panel.set_css_classes(&["panel"]);
        panel.append(network.export_widget());
        panel.append(datetime.export_widget());
        panel.append(power.export_widget());

        smol::spawn(async move {
            network_service.listen().await;
        }).detach();
        smol::spawn(gtk4::glib::spawn_future_local(async move {
            network.listen_mut().await;
        })).detach();
        
        Panel(panel)
    }

    pub fn export_widget(&self) -> &Box {
        &self.0
    }
}
