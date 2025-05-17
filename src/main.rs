mod ipc;
mod widgets;
mod windows;

use std::sync::Arc;
use std::thread;

use gtk4::{Application, gio, glib};
use gtk4::{Revealer, Settings, prelude::*};
use ipc::niri::NiriService;

const APP_ID: &str = "io.github.bigsaltyfishes.molyuubar";

const CSS: &str = include_str!("../target/style.css");

fn main() -> glib::ExitCode {
    gio::resources_register_include!("icons.gresource").expect("Failed to register resources.");

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let display = gtk4::gdk::Display::default().expect("Failed to get default display");
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(CSS);

        gtk4::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| {
        let mut service = NiriService::new();
        let taskbar = windows::bar::Taskbar::new(app, &mut service);
        thread::spawn(move || {
            smol::block_on(service.listen());
        });
        taskbar.export_widget().present();
    });

    app.run()
}
