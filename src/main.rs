#[cfg(test)]
mod tests;
mod utils;
mod service;
mod widgets;
mod windows;


use std::time::Duration;

use adw::Application;
use gtk4::{gio, glib};
use gtk4::prelude::*;
use service::event::EventListener;
use service::network::endpoints::event::{NetworkServiceEvent, NetworkServiceEventType, NetworkServiceRequest, WiFiConnServiceMessage, WiFiConnServiceRequest, WiFiConnServiceResponse};
use service::network::wireless::ap::AccessPointSecurity;
use service::network::{NetworkService};
use service::niri::NiriService;
use smol::Timer;
use tracing_subscriber::EnvFilter;

const APP_ID: &str = "io.github.bigsaltyfishes.molyuubar";

const CSS: &str = include_str!("../target/style.css");


fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_level(true)
        .with_target(true)
        .init();
}

fn main() -> glib::ExitCode {
    gio::resources_register_include!("icons.gresource").expect("Failed to register resources.");

    init_logging();

    // Initialize GTK application
    let app = Application::builder()
        .application_id(APP_ID)
        .build();
    
    app.connect_startup(|_| {
        let display = gtk4::gdk::Display::default().expect("Failed to get default display");
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(CSS);

        adw::StyleManager::default()
            .set_color_scheme(adw::ColorScheme::PreferDark);

        gtk4::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| {
        let mut service = NiriService::new();
        let taskbar = windows::bar::Taskbar::new(app, &mut service);

        smol::spawn(async move {
            service.listen().await;
        }).detach();
        taskbar.export_widget().present();
    });
    app.run()
}