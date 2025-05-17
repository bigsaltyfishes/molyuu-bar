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

pub trait EventListener: Send + Sync {
    fn register_event_handler(
        &mut self,
        event_type: UIUpdateEventType,
        sender: Sender<UIUpdateEvent>,
    );
}

pub trait EventHandler {
    fn register_to_listener(&self, listener: &mut impl EventListener);
    async fn listen(&mut self);
}
