use gtk4::{
    Box, Label,
    prelude::{BoxExt, WidgetExt},
};

pub struct DateTime {
    container: Box,
}

impl DateTime {
    pub fn new() -> Self {
        let container = Box::new(gtk4::Orientation::Horizontal, 4);
        let time = Label::new(Some(""));
        container.add_css_class("datetime");
        time.add_css_class("time");
        container.append(&time);

        smol::spawn(gtk4::glib::spawn_future_local(
            async move {
                loop {
                    let now = chrono::Local::now();
                    time.set_label(&now.format("%A %d, %H:%M").to_string());
                    smol::Timer::after(std::time::Duration::from_secs(1)).await;
                }
            }
        )).detach();

        Self { container }
    }

    pub fn export_widget(&self) -> &Box {
        &self.container
    }
}