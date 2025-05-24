use std::{collections::HashMap, error::Error, net::Shutdown};

use niri_ipc::{Event, Request, Response, Window, Workspace};
use smol::{
    channel::{SendError, Sender},
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::unix::UnixStream,
};

use super::event::{EventListener, UIUpdateEvent, UIUpdateEventType};

#[derive(serde::Deserialize)]
#[allow(non_snake_case, dead_code, unused)]
struct OkReply {
    Ok: String,
}

pub struct NiriWorkspaces {
    workspaces: HashMap<u8, Workspace>,
    id_idx: HashMap<u64, u8>,
    focused: u8,
}

impl NiriWorkspaces {
    pub fn new() -> Self {
        NiriWorkspaces {
            workspaces: HashMap::new(),
            id_idx: HashMap::new(),
            focused: 1,
        }
    }

    pub fn update_all(&mut self, workspaces: Vec<Workspace>) {
        let mut id_idx = HashMap::new();
        let mut workspace_map = HashMap::new();
        for workspace in workspaces {
            if workspace.is_focused {
                self.focused = workspace.idx;
            }
            id_idx.insert(workspace.id, workspace.idx);
            workspace_map.insert(workspace.idx, workspace);
        }
        self.workspaces = workspace_map;
        self.id_idx = id_idx;
    }

    pub fn get_workspace_by_id(&self, id: u64) -> Option<Workspace> {
        if let Some(idx) = self.id_idx.get(&id) {
            self.workspaces.get(idx).cloned()
        } else {
            None
        }
    }

    pub fn last_workspace(&self) -> Option<Workspace> {
        self.workspaces.values().last().cloned()
    }

    pub fn add_workspace(&mut self, workspace: Workspace) {
        self.workspaces.insert(workspace.idx, workspace);
    }

    pub fn remove_workspace(&mut self, idx: u8) {
        self.workspaces.remove(&idx);
    }

    pub fn set_focused(&mut self, idx: u8) {
        if let Some(workspace) = self.workspaces.get_mut(&idx) {
            workspace.is_focused = true;
        }
        if let Some(workspace) = self.workspaces.get_mut(&self.focused) {
            workspace.is_focused = false;
        }
        self.focused = idx;
    }

    pub fn get_focused(&self) -> Option<Workspace> {
        self.workspaces.get(&self.focused).cloned()
    }

    pub fn num_workspaces(&self) -> usize {
        self.workspaces.len()
    }
}

pub struct NiriWindows {
    windows: HashMap<u64, Window>,
    focused: u64,
}

impl NiriWindows {
    pub fn new() -> Self {
        NiriWindows {
            windows: HashMap::new(),
            focused: 0,
        }
    }

    pub fn update_all(&mut self, windows: Vec<Window>) {
        for window in windows {
            if window.is_focused {
                self.focused = window.id;
            }
            self.windows.insert(window.id, window);
        }
    }

    pub fn add_window(&mut self, window: Window) {
        self.windows.insert(window.id, window);
    }

    pub fn remove_window(&mut self, id: u64) {
        self.windows.remove(&id);
        if id == self.focused {
            self.focused = 0;
        }
    }

    pub fn set_focused(&mut self, id: Option<u64>) {
        let id = id.unwrap_or(0);
        if let Some(window) = self.windows.get_mut(&id) {
            window.is_focused = true;
        }
        if let Some(window) = self.windows.get_mut(&self.focused) {
            window.is_focused = false;
        }
        self.focused = id;
    }

    pub fn get_focused(&self) -> Option<Window> {
        self.windows.get(&self.focused).cloned()
    }
}

pub struct NiriService {
    workspaces: NiriWorkspaces,
    windows: NiriWindows,
    event_handlers: HashMap<UIUpdateEventType, Vec<Sender<UIUpdateEvent>>>,
}

impl NiriService {
    pub fn new() -> Self {
        NiriService {
            workspaces: NiriWorkspaces::new(),
            windows: NiriWindows::new(),
            event_handlers: HashMap::new(),
        }
    }

    async fn send_event(&mut self, event_type: UIUpdateEventType, event: UIUpdateEvent) {
        if let Some(handlers) = self.event_handlers.get_mut(&event_type) {
            let mut closed_handlers = Vec::new();
            for (i, handler) in handlers.iter().enumerate() {
                let status = handler.send(event.clone()).await;

                if let Err(SendError(_)) = status {
                    closed_handlers.push(i);
                }
            }

            for i in closed_handlers.iter().rev() {
                handlers.remove(*i);
            }
        }
    }

    pub async fn listen(&mut self) {
        let niri_socket =
            std::env::var("NIRI_SOCKET").expect("NIRI_SOCKET not set. Is niri running?");
        let mut stream = UnixStream::connect(niri_socket)
            .await
            .expect("Failed to connect to niri socket");
        let command =
            serde_json::to_string(&Request::EventStream).expect("Failed to serialize command");
        stream
            .write_all(command.as_bytes())
            .await
            .expect("Failed to write command to socket");
        stream
            .shutdown(Shutdown::Write)
            .expect("Failed to shutdown command stream");

        let mut buffer = String::new();
        let mut reader = BufReader::new(stream.clone());

        // Read the initial state
        reader
            .read_line(&mut buffer)
            .await
            .expect("Failed to read line from socket");

        if let Ok(_) = serde_json::from_str::<OkReply>(&buffer) {
            println!("Niri is ready to handle events");
        } else {
            panic!("Failed to read initial state from niri");
        }

        loop {
            buffer.clear();
            reader
                .read_line(&mut buffer)
                .await
                .expect("Failed to read line from socket");
            if buffer.is_empty() {
                continue;
            }
            let event: niri_ipc::Event =
                serde_json::from_str(&buffer).expect("Failed to parse event");
            match event {
                Event::WorkspacesChanged { workspaces } => {
                    self.workspaces.update_all(workspaces);
                    if let Some(focused_workspace) = self.workspaces.get_focused() {
                        self.send_event(
                            UIUpdateEventType::WorkspaceChanged,
                            UIUpdateEvent::WorkspaceChanged {
                                num: self.workspaces.num_workspaces() as _,
                                focused: focused_workspace.idx,
                            },
                        )
                        .await;
                    }
                }
                Event::WorkspaceActivated { id, focused } => {
                    if focused {
                        self.workspaces
                            .set_focused(self.workspaces.get_workspace_by_id(id).unwrap().idx);
                        if let Some(focused_workspace) = self.workspaces.get_focused() {
                            self.send_event(
                                UIUpdateEventType::WorkspaceChanged,
                                UIUpdateEvent::WorkspaceChanged {
                                    num: self.workspaces.num_workspaces() as _,
                                    focused: focused_workspace.idx,
                                },
                            )
                            .await;
                        }
                    }
                }
                Event::WorkspaceActiveWindowChanged {
                    workspace_id,
                    active_window_id,
                } => {
                    // TODO: Should check if window and workspace is focused?
                    if let Some(workspace) =
                        self.workspaces.workspaces.get_mut(&(workspace_id as _))
                    {
                        workspace.active_window_id = active_window_id;
                    }
                }
                Event::WindowsChanged { windows } => {
                    self.windows.update_all(windows);
                    if let Some(focused_window) = self.windows.get_focused() {
                        self.send_event(
                            UIUpdateEventType::WindowFocusChanged,
                            UIUpdateEvent::WindowFocusChanged {
                                app_id: focused_window.app_id.clone(),
                                title: focused_window.title.clone(),
                            },
                        )
                        .await;
                    }
                }
                Event::WindowClosed { id } => {
                    self.windows.remove_window(id);
                    if self.windows.focused == id {
                        self.send_event(
                            UIUpdateEventType::WindowFocusChanged,
                            UIUpdateEvent::WindowFocusChanged {
                                app_id: Some("Niri".to_string()),
                                title: Some("Desktop".to_string()),
                            },
                        )
                        .await;
                    }
                }
                Event::WindowOpenedOrChanged { window } => {
                    self.windows.add_window(window.clone());
                    if window.is_focused {
                        self.send_event(
                            UIUpdateEventType::WindowFocusChanged,
                            UIUpdateEvent::WindowFocusChanged {
                                app_id: window.app_id.clone(),
                                title: window.title.clone(),
                            },
                        )
                        .await;
                    }
                }
                Event::WindowFocusChanged { id } => {
                    self.windows.set_focused(id);
                    if let Some(focused_window) = self.windows.get_focused() {
                        self.send_event(
                            UIUpdateEventType::WindowFocusChanged,
                            UIUpdateEvent::WindowFocusChanged {
                                app_id: focused_window.app_id.clone(),
                                title: focused_window.title.clone(),
                            },
                        )
                        .await;
                    } else {
                        self.send_event(
                            UIUpdateEventType::WindowFocusChanged,
                            UIUpdateEvent::WindowFocusChanged {
                                app_id: Some("Niri".to_string()),
                                title: Some("Desktop".to_string()),
                            },
                        )
                        .await;
                    }
                }
                _ => {
                    println!("Unhandled event: {:?}", event);
                }
            }
        }
    }

    pub async fn send_command(command: Request) -> Result<Response, Box<dyn Error + Send + Sync>> {
        let niri_socket = std::env::var("NIRI_SOCKET")?;
        let mut stream = UnixStream::connect(niri_socket).await?;
        let command_str = serde_json::to_string(&command).expect("Failed to serialize command");
        stream.write_all(command_str.as_bytes()).await?;
        stream.shutdown(Shutdown::Write)?;

        let mut buffer = String::new();
        let mut reader = BufReader::new(stream.clone());
        reader.read_line(&mut buffer).await?;

        let _: OkReply = serde_json::from_str(&buffer)?;

        if let Request::Action(_) = command {
            return Ok(Response::Handled);
        }

        buffer.clear();
        reader.read_line(&mut buffer).await?;
        if buffer.is_empty() {
            return Err("Niri did not return a valid response".into());
        }

        let response: Response = serde_json::from_str(&buffer)?;
        Ok(response)
    }
}

impl EventListener<UIUpdateEventType, UIUpdateEvent> for NiriService {
    fn register_event_handler(
        &mut self,
        event_type: UIUpdateEventType,
        sender: Sender<UIUpdateEvent>,
    ) {
        self.event_handlers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(sender);
    }
}
