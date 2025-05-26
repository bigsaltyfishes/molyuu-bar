use std::collections::{HashMap, HashSet};

use adw::{gio::NetworkService, glib::object::IsA, prelude::{ActionRowExt, PreferencesRowExt}};
use gtk4::{prelude::{BoxExt, ButtonExt, PopoverExt, WidgetExt}, Button, Popover, Widget};
use rusty_network_manager::AccessPointProxy;
use smol::channel::{Receiver, Sender};
use tracing::{instrument, warn};

use crate::{service::{event::{EventHandler, EventHandlerExt, EventHandlerMutExt, EventListener}, network::{endpoints::event::{NetworkDeviceState, NetworkDeviceType, NetworkServiceEvent, NetworkServiceEventType, NetworkServiceRequest}, wireless::{self, ap::{AccessPoint, AccessPointSecurity}}}}, utils::strings};

const WIFI_OFF: &str = "/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_off_24.svg";
const WIFI_NOT_CONNECTED_BUT_AVAILABLE: &str = "/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_statusbar_not_connected_24.svg";
const NETWORK_NOT_CONNECTED: &str = "/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_bad_24.svg";
const ETHERNET_CONNECTED: &str = "/io/github/bigsaltyfishes/molyuubar/icons/settings_ethernet_24.svg";

pub struct NetworkMenu {
    popover: Popover,
    wireless_menu: WirelessMenu,
}

impl NetworkMenu {
    pub fn new(parent: &impl IsA<Widget>) -> Self {
        let popover = Popover::new();
        let wireless_menu = WirelessMenu::new();
        popover.add_css_class("popup");
        popover.set_parent(parent);
        popover.set_child(Some(wireless_menu.export_widget()));

        Self {
            popover,
            wireless_menu,
        }
    }

    pub fn export_widget(&self) -> &Popover {
        &self.popover
    }
}

pub struct EthernetMenu {
    ethernet_row: adw::ActionRow,
    icon: gtk4::Image,
}

impl EthernetMenu {
    pub fn new() -> Self {
        let ethernet_row = adw::ActionRow::new();
        ethernet_row.set_title("Ethernet");
        ethernet_row.set_subtitle("Not connected");
        ethernet_row.add_css_class("ethernet");

        let icon = gtk4::Image::from_resource(ETHERNET_CONNECTED);
        icon.add_css_class("icon");
        icon.set_valign(gtk4::Align::Center);
        icon.set_halign(gtk4::Align::Center);
        ethernet_row.add_prefix(&icon);

        Self {
            ethernet_row,
            icon,
        }
    }

    pub fn export_widget(&self) -> &adw::ActionRow {
        &self.ethernet_row
    }
}

pub struct WirelessMenu {
    access_points: HashSet<(String, AccessPointSecurity)>,
    controller_icon: gtk4::Image,
    controller: adw::SwitchRow,
    menus: HashMap<String, adw::ExpanderRow>,
    outer_box: gtk4::Box,
}

impl WirelessMenu {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let controller = adw::SwitchRow::new();
        let controller_icon = gtk4::Image::from_paintable(Some(Self::match_controller_icon(false, false, false)));

        container.add_css_class("wireless");
        controller.add_css_class("controller");
        controller_icon.add_css_class("icon");

        controller_icon.set_halign(gtk4::Align::Fill);
        controller_icon.set_valign(gtk4::Align::Fill);
        controller_icon.set_hexpand(true);
        controller_icon.set_vexpand(true);

        controller.add_prefix(&controller_icon);
        controller.set_title("Wireless Radio");
        controller.set_subtitle("Disabled");

        container.append(&controller);

        Self {
            access_points: HashSet::new(),
            controller_icon: controller_icon,
            controller: controller,
            menus: HashMap::new(),
            outer_box: container,
        }
    }

    pub fn export_widget(&self) -> &gtk4::Box {
        &self.outer_box
    }
}

impl WirelessMenu {
    fn match_controller_icon(enabled: bool, available: bool, connected: bool) -> &'static gtk4::gdk::Texture {
        lazy_static::lazy_static! {
            static ref WIFI_OFF_TEXTRUE: gtk4::gdk::Texture = gtk4::gdk::Texture::for_pixbuf(
                &gtk4::gdk_pixbuf::Pixbuf::from_resource_at_scale(WIFI_OFF, 36, 36, true).expect("Failed to load resource")
            );
            static ref WIFI_NOT_CONNECTED_TEXTRUE: gtk4::gdk::Texture = gtk4::gdk::Texture::for_pixbuf(
                &gtk4::gdk_pixbuf::Pixbuf::from_resource_at_scale(WIFI_NOT_CONNECTED_BUT_AVAILABLE, 36, 36, true).expect("Failed to load resource")
            );
            static ref NETWORK_NOT_CONNECTED_TEXTRUE: gtk4::gdk::Texture = gtk4::gdk::Texture::for_pixbuf(
                &gtk4::gdk_pixbuf::Pixbuf::from_resource_at_scale(NETWORK_NOT_CONNECTED, 36, 36, true).expect("Failed to load resource")
            );
        }
        
        if !enabled {
            &WIFI_OFF_TEXTRUE
        } else if !available {
            &WIFI_NOT_CONNECTED_TEXTRUE
        } else if connected {
            // TODO: Replace with actual signal strength icon
            &NETWORK_NOT_CONNECTED_TEXTRUE
        } else {
            &NETWORK_NOT_CONNECTED_TEXTRUE
        }
    }
}

#[derive(Default)]
pub struct NetworkStateStorage {
    wifi_enabled: bool,
    interfaces: HashMap<String, (NetworkDeviceType, bool)>,
    default_routing_interface: Option<(NetworkDeviceType, String)>,
}

pub struct Network {
    button: Button,
    icon: gtk4::Image,
    event_channel: (Sender<NetworkServiceEvent>, Receiver<NetworkServiceEvent>),
    cmd_sender: Option<Sender<NetworkServiceRequest>>,
    storage: NetworkStateStorage,
}

impl Network {
    pub fn new() -> Self {
        let button = Button::new();
        button.add_css_class("network-button");
        button.add_css_class("pill");
        button.set_label("Network");
        button.set_vexpand(true);

        let icon = gtk4::Image::from_resource("/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_bad_24.svg");
        icon.add_css_class("icon");
        icon.set_valign(gtk4::Align::Center);
        icon.set_halign(gtk4::Align::Center);
        button.set_child(Some(&icon));
        button.set_tooltip_text(Some("Network"));
        let menu = NetworkMenu::new(&button);

        button.connect_clicked(move |_| {
            let popover = menu.export_widget();
            popover.popup();
        });

        Self {
            button,
            icon,
            event_channel: smol::channel::unbounded(),
            cmd_sender: None,
            storage: NetworkStateStorage::default(),
        }
    }

    pub fn export_widget(&self) -> &Button {
        &self.button
    }
}

impl Network {
    fn match_icon(ap: AccessPoint) -> &'static str {
        match (ap.signal_strength, ap.key_management() != AccessPointSecurity::None) {
            (0..=20, true)   => "/io/github/bigsaltyfishes/molyuubar/icons/wifi_lock_24.svg",
            (21..=40, true)  => "/io/github/bigsaltyfishes/molyuubar/icons/network_wifi_1_bar_locked_24.svg",
            (41..=60, true)  => "/io/github/bigsaltyfishes/molyuubar/icons/network_wifi_2_bar_locked_24.svg",
            (61..=80, true)  => "/io/github/bigsaltyfishes/molyuubar/icons/network_wifi_3_bar_locked_24.svg",
            (_, true)        => "/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_4_bar_lock_24.svg",
            (0..=20, false)  => "/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_0_24.svg",
            (21..=40, false) => "/io/github/bigsaltyfishes/molyuubar/icons/network_wifi_1_bar_24.svg",
            (41..=60, false) => "/io/github/bigsaltyfishes/molyuubar/icons/network_wifi_2_bar_24.svg",
            (61..=80, false) => "/io/github/bigsaltyfishes/molyuubar/icons/network_wifi_3_bar_24.svg",
            (_, false)       => "/io/github/bigsaltyfishes/molyuubar/icons/signal_wifi_4_bar_24.svg",
        }
    }
}

impl EventHandler<NetworkServiceEventType, NetworkServiceEvent> for Network {
    fn register_to_listener(&mut self, listener: &mut impl EventListener<NetworkServiceEventType, NetworkServiceEvent>) {
        listener.register_event_handler_many(vec![
            NetworkServiceEventType::DeviceAdded,
            NetworkServiceEventType::DeviceRemoved,
            NetworkServiceEventType::DeviceStateChanged,
            NetworkServiceEventType::AccessPointScanReport,
            NetworkServiceEventType::ActiveAccessPointChanged,
            NetworkServiceEventType::GlobalWirelessEnabledStateChanged,
        ], self.event_channel.0.clone());

        match smol::block_on(self.event_channel.1.recv()).expect("Unable to register event handler.") {
            NetworkServiceEvent::HandlerRegistered { command_sender } => {
                self.cmd_sender = Some(command_sender);
            }
            _ => {
                panic!("Unexpected event received during handler registration.");
            }
        }
    }
}

impl EventHandlerMutExt<NetworkServiceEventType, NetworkServiceEvent> for Network {
    #[instrument(skip_all)]
    async fn listen_mut(&mut self) {
        while let Ok(event) = self.event_channel.1.recv().await {
            match event {
                NetworkServiceEvent::DeviceAdded { interface, device_type } => {
                    match device_type {
                        NetworkDeviceType::WiFi | NetworkDeviceType::Ethernet=> {
                            self.storage.interfaces.insert(interface.clone(), (device_type, false));
                            if self.storage.default_routing_interface.is_none() {
                                self.storage.default_routing_interface = Some((device_type.clone(), interface.clone()));
                            }
                        }
                        _ => {
                            warn!("Unsupported device type: {:?}", device_type);
                        }
                    }
                }
                NetworkServiceEvent::DeviceRemoved { interface } => {
                    if let Some((device_type, _)) = self.storage.interfaces.remove(&interface) {
                        if self.storage.default_routing_interface.as_ref().map_or(false, |(dt, _)| *dt == device_type) {
                            self.storage.default_routing_interface = None;
                            self.storage.interfaces.iter().find(|(_, (_, state))| {
                                *state
                            }).map(|(name, (dt, _))| {
                                self.storage.default_routing_interface = Some((dt.clone(), name.clone()));
                            });
                        }

                        if let Some((dt, _)) = self.storage.default_routing_interface {
                            if let Some((_, true)) = self.storage.interfaces.get(&interface) {
                                match dt {
                                    NetworkDeviceType::WiFi => {
                                        todo!() // Handle WiFi icon update
                                    }
                                    NetworkDeviceType::Ethernet => {
                                        self.icon.set_resource(Some(ETHERNET_CONNECTED));
                                    }
                                    _ => {
                                        warn!("Unsupported device type for icon update: {:?}", dt);
                                    }
                                }
                            }
                        } else {
                            self.icon.set_resource(Some(NETWORK_NOT_CONNECTED));
                        }
                    } else {
                        warn!("Attempted to remove non-existent interface: {}", interface);
                    }
                }
                NetworkServiceEvent::DeviceStateChanged { interface, state, reason } => {
                    if let Some((device_type, is_activated)) = self.storage.interfaces.get_mut(&interface) {
                        if state == NetworkDeviceState::Activated {
                            *is_activated = true;
                        } else {
                            *is_activated = false;
                        }
                        
                        if let Some((_, default_interface)) = self.storage.default_routing_interface.as_mut() {
                            if default_interface.as_str() == interface.as_str() {
                                if *is_activated {
                                    match device_type {
                                        NetworkDeviceType::WiFi => {
                                            // TODO: Handle WiFi icon update
                                        }
                                        NetworkDeviceType::Ethernet => {
                                            self.icon.set_resource(Some(ETHERNET_CONNECTED));
                                        }
                                        _ => {
                                            warn!("Unsupported device type for icon update: {:?}", device_type);
                                        }
                                    }
                                } else {
                                    self.icon.set_resource(Some(NETWORK_NOT_CONNECTED));
                                    self.storage.default_routing_interface = None;
                                }
                            }
                        } else if *is_activated {
                            self.storage.default_routing_interface = Some((device_type.clone(), interface.clone()));
                            match device_type {
                                NetworkDeviceType::WiFi => {
                                    // TODO: Handle WiFi icon update
                                }
                                NetworkDeviceType::Ethernet => {
                                    self.icon.set_resource(Some(ETHERNET_CONNECTED));
                                }
                                _ => {
                                    warn!("Unsupported device type for icon update: {:?}", device_type);
                                }
                            }
                        }
                    } else {
                        warn!("Received state change for unknown interface: {}", interface);
                    }
                }
                NetworkServiceEvent::AccessPointScanReport { interface, access_points } => {
                    // TODO: Handle access point scan report
                }
                NetworkServiceEvent::ActiveAccessPointChanged { interface, ap } => {
                    // TODO: Handle active access point change
                }
                NetworkServiceEvent::GlobalWirelessEnabledStateChanged { enabled } => {
                    // TODO: Handle global wireless state change
                }
                _ => {}
            }
        }
    }
}