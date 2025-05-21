use std::time::Duration;

use gtk4::{
    Box, Button, Revealer, RevealerTransitionType,
    prelude::{BoxExt, ButtonExt, WidgetExt},
};
use smol::{
    Timer,
    channel::{Receiver, Sender},
};

use crate::service::{
    event::{EventHandler, EventListener, UIUpdateEvent, UIUpdateEventType},
    niri::NiriService,
};

pub struct Workspace {
    revealer: Revealer,
    container: Box,
    outer_container: Box,
    current_focused: u8,
    channel: (Sender<UIUpdateEvent>, Receiver<UIUpdateEvent>),
    buttons: Vec<Button>,
}

impl Workspace {
    pub fn new() -> Self {
        let outer_container = Box::new(gtk4::Orientation::Horizontal, 0);
        let workspace = Box::new(gtk4::Orientation::Horizontal, 5);
        workspace.add_css_class("workspace");
        outer_container.add_css_class("workspace-container");
        outer_container.append(&workspace);

        let revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::Crossfade)
            .transition_duration(300)
            .reveal_child(false)
            .child(&outer_container)
            .build();

        Workspace {
            revealer,
            container: workspace,
            outer_container,
            current_focused: 0,
            channel: smol::channel::unbounded(),
            buttons: Vec::new(),
        }
    }

    pub async fn increase_button(&mut self) {
        let idx = self.buttons.len() as u8;
        let button = Button::new();
        button.add_css_class("workspace-button");
        button.connect_clicked(move |_| {
            smol::spawn(NiriService::send_command(niri_ipc::Request::Action(
                niri_ipc::Action::FocusWorkspace {
                    reference: niri_ipc::WorkspaceReferenceArg::Index(idx + 1),
                },
            )))
            .detach();
        });

        // Add button after a delay to allow for animation
        self.revealer.set_reveal_child(false);
        Timer::after(Duration::from_millis(300)).await;
        self.container.append(&button);
        self.revealer.set_reveal_child(true);
        self.buttons.push(button);
    }

    pub async fn decrease_button(&mut self) {
        if let Some(button) = self.buttons.pop() {
            // Remove button after a delay to allow for animation
            self.revealer.set_reveal_child(false);
            Timer::after(Duration::from_millis(300)).await;
            self.container.remove(&button);
            self.revealer.set_reveal_child(true);
        }
    }

    pub fn export_widget(&self) -> &Revealer {
        &self.revealer
    }
}

impl EventHandler for Workspace {
    fn register_to_listener(&self, listener: &mut impl EventListener) {
        listener
            .register_event_handler(UIUpdateEventType::WorkspaceChanged, self.channel.0.clone());
    }

    async fn listen(&mut self) {
        while let Ok(event) = self.channel.1.recv().await {
            match event {
                UIUpdateEvent::WorkspaceChanged { num, focused } => {
                    if num > self.buttons.len() as u8 {
                        for _ in 0..(num - self.buttons.len() as u8) {
                            self.increase_button().await;
                        }
                    } else if num < self.buttons.len() as u8 {
                        for _ in 0..(self.buttons.len() as u8 - num) {
                            self.decrease_button().await;
                        }
                    }

                    if focused != self.current_focused {
                        if let Some(button) = self.buttons.get(focused as usize - 1) {
                            button.add_css_class("active");
                        }
                        if let Some(button) = self
                            .buttons
                            .get(self.current_focused.wrapping_sub(1) as usize)
                        {
                            button.remove_css_class("active");
                        }
                        self.current_focused = focused;
                    }
                }
                _ => {}
            }
        }
    }
}
