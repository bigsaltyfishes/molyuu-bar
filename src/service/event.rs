use std::hash::{Hash, Hasher};

use smol::channel::Sender;

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum UIUpdateEventType {
    WorkspaceChanged,
    WindowFocusChanged,
    WindowClosed,
}

#[derive(Clone, Debug)]
pub enum UIUpdateEvent {
    WorkspaceChanged {
        num: u8,
        focused: u8,
    },
    WindowFocusChanged {
        app_id: Option<String>,
        title: Option<String>,
    },
    WindowClosed,
}

pub trait EventListener<T: Hash, EVENT>: Send + Sync {
    fn register_event_handler(
        &mut self,
        event_type: T,
        sender: Sender<EVENT>,
    );
}

pub trait EventHandler<T: Hash, EVENT> {
    fn register_to_listener(&self, listener: &mut impl EventListener<T, EVENT>);
    async fn listen(&mut self);
}
