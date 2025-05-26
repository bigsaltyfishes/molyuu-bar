use gtk4::{prelude::{ButtonExt, PopoverExt, WidgetExt}, Button, Image, Popover};

pub struct PowerMenu(Popover);

impl PowerMenu {
    pub fn new() -> Self {
        let popover = Popover::new();
        let label = gtk4::Label::new(Some("Power Menu"));
        label.set_justify(gtk4::Justification::Center);

        popover.add_css_class("popup");
        popover.set_child(Some(&label));

        PowerMenu(popover)
    }

    pub fn export_widget(&self) -> &Popover {
        &self.0
    }
}

pub struct Power(Button);

impl Power {
    pub fn new() -> Self {
        let button = Button::new();
        button.add_css_class("power-button");
        button.add_css_class("pill");
        button.set_label("Power");
        button.set_vexpand(true);

        let icon = Image::from_resource("/io/github/bigsaltyfishes/molyuubar/icons/power_settings_24.svg");
        icon.add_css_class("icon");
        icon.set_valign(gtk4::Align::Center);
        icon.set_halign(gtk4::Align::Center);
        button.set_child(Some(&icon));
        button.set_tooltip_text(Some("Power"));

        let popup = PowerMenu::new();
        popup.export_widget().connect_hide(move |popup| {
            popup.remove_css_class("visible");
        });
        
        popup.export_widget().set_parent(&button);
        button.connect_clicked(move |_| {
            popup.export_widget().popup();
            popup.export_widget().add_css_class("visible");
        });

        Power(button)
    }

    pub fn export_widget(&self) -> &Button {
        &self.0
    }
}