use std::time::Duration;

use gtk4::{
    Box, Label, Revealer,
    prelude::{BoxExt, WidgetExt},
};
use smol::{
    Timer,
    channel::{Receiver, Sender},
};

use crate::service::event::{EventHandler, EventListener, UIUpdateEvent, UIUpdateEventType};

pub struct CurrentWindow {
    app_id: Label,
    app_title: Label,
    channel: (Sender<UIUpdateEvent>, Receiver<UIUpdateEvent>),
    box_revealer: Revealer,
}

impl CurrentWindow {
    pub fn new() -> Self {
        let app_id = Label::new(Some("Niri"));
        let app_title = Label::new(Some(""));
        let container = Box::new(gtk4::Orientation::Vertical, 0);
        let outer_container = Box::new(gtk4::Orientation::Horizontal, 0);

        app_id.add_css_class("app-id");
        container.add_css_class("current-window");
        app_title.add_css_class("app-title");

        app_id.set_halign(gtk4::Align::Start);
        app_title.set_halign(gtk4::Align::Start);

        container.append(&app_id);
        container.append(&app_title);
        container.set_halign(gtk4::Align::Start);
        container.set_valign(gtk4::Align::Center);

        outer_container.append(&container);

        let box_revealer = Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::Crossfade)
            .transition_duration(300)
            .reveal_child(false)
            .child(&outer_container)
            .build();

        Self {
            app_id,
            app_title,
            channel: smol::channel::unbounded(),
            box_revealer,
        }
    }

    pub fn export_widget(&self) -> &Revealer {
        &self.box_revealer
    }
}

impl EventHandler<UIUpdateEventType, UIUpdateEvent> for CurrentWindow {
    fn register_to_listener(&self, listener: &mut impl EventListener<UIUpdateEventType, UIUpdateEvent>) {
        listener.register_event_handler(
            UIUpdateEventType::WindowFocusChanged,
            self.channel.0.clone(),
        );
    }

    async fn listen(&mut self) {
        while let Ok(event) = self.channel.1.recv().await {
            match event {
                UIUpdateEvent::WindowFocusChanged { app_id, title } => {
                    self.box_revealer.set_reveal_child(false);
                    Timer::after(Duration::from_millis(300)).await;
                    self.app_id.set_text(app_id.as_deref().unwrap_or("Niri"));
                    self.app_title
                        .set_text(truncate_text(title.as_deref().unwrap_or("Niri"), 40).as_str());
                    self.box_revealer.set_reveal_child(true);
                }
                _ => {}
            }
        }
    }
}

fn truncate_text(text: &str, max_length: usize) -> String {
    let sanitized: String = text
        .chars()
        .filter(|&c| c != '\u{FFFD}')
        .collect::<String>();
    if sanitized.len() > max_length {
        let truncated = sanitized.chars().take(max_length).collect::<String>();
        format!("{}...", truncated)
    } else {
        sanitized
    }
}
