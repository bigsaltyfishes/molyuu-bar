use gtk4::{Application, ApplicationWindow};
use gtk4::{CenterBox, prelude::*};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::ipc::event::{EventHandler, EventListener};
use crate::widgets::current_window::CurrentWindow;
use crate::widgets::panel::Panel;
use crate::widgets::workspace::Workspace;

pub struct Taskbar {
    window: ApplicationWindow,
    container: CenterBox,
}

impl Taskbar {
    pub fn new(application: &Application, service: &mut impl EventListener) -> Self {
        let window = ApplicationWindow::new(application);

        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.auto_exclusive_zone_enable();

        let anchors = [(Edge::Left, true), (Edge::Right, true), (Edge::Top, true)];

        for (edge, anchor) in anchors {
            window.set_anchor(edge, anchor);
        }

        let container = CenterBox::new();
        let mut workspace = Workspace::new();
        let mut current_window = CurrentWindow::new();
        let panel = Panel::new();
        workspace.register_to_listener(service);
        current_window.register_to_listener(service);
        container.set_start_widget(Some(workspace.export_widget()));
        container.set_center_widget(Some(current_window.export_widget()));
        container.set_end_widget(Some(panel.export_widget()));

        smol::spawn(gtk4::glib::spawn_future_local(async move {
            workspace.listen().await;
        }))
        .detach();
        smol::spawn(gtk4::glib::spawn_future_local(async move {
            current_window.listen().await;
        }))
        .detach();

        window.set_css_classes(&["taskbar"]);
        window.set_child(Some(&container));

        Taskbar { window, container }
    }

    pub fn export_widget(&self) -> &ApplicationWindow {
        &self.window
    }
}
