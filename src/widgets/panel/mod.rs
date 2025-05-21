mod power;
mod datetime;
mod network;

use gtk4::{
    Box,
    prelude::{BoxExt, WidgetExt},
};
use power::Power;

pub struct Panel(Box);

impl Panel {
    pub fn new() -> Self {
        let panel = Box::new(gtk4::Orientation::Horizontal, 4);
        let datetime = datetime::DateTime::new();
        let power = Power::new();
        panel.set_css_classes(&["panel"]);
        panel.append(datetime.export_widget());
        panel.append(power.export_widget());
        Panel(panel)
    }

    pub fn export_widget(&self) -> &Box {
        &self.0
    }
}
