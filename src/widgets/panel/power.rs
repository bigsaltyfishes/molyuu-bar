use std::time::Duration;

use gtk4::{
    Box, Button, Image, Popover,
    prelude::{BoxExt, ButtonExt, PopoverExt, WidgetExt},
};

pub struct Power(Button);

impl Power {
    pub fn new() -> Self {
        let button = Button::new();
        button.add_css_class("power-button");
        button.set_label("Power");
        button.set_vexpand(true);

        let icon =
            Image::from_resource("/io/github/bigsaltyfishes/molyuubar/icons/power_settings_24.svg");
        icon.set_valign(gtk4::Align::Center);
        icon.set_halign(gtk4::Align::Center);
        button.set_child(Some(&icon));
        button.set_tooltip_text(Some("Power"));

        Power(button)
    }

    pub fn export_widget(&self) -> &Button {
        &self.0
    }
}
