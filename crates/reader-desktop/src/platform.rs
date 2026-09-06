use crate::backend::{Backend, Event};
use ksni::TrayMethods;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use winit::event_loop::EventLoopProxy;

#[derive(Debug)]
pub struct Tray {
    proxy: EventLoopProxy<Event>,
    pub unread: i64,
    pub status: String,
}
impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "ruby-reader".into()
    }
    fn title(&self) -> String {
        format!("Ruby Reader · {} unread · {}", self.unread, self.status)
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let img = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
            .unwrap()
            .to_rgba8();
        let mut data = Vec::with_capacity(img.as_raw().len());
        for px in img.pixels() {
            data.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
        }
        vec![ksni::Icon {
            width: 128,
            height: 128,
            data,
        }]
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.proxy.send_event(Event::Open);
    }
    fn watcher_online(&self) {
        let _ = self.proxy.send_event(Event::TrayStatus(true));
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        let _ = self.proxy.send_event(Event::TrayStatus(false));
        true
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: format!("Ruby Reader · {} unread", self.unread),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open Ruby Reader".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.proxy.send_event(Event::Open);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Refresh all feeds".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.proxy.send_event(Event::Refresh);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.proxy.send_event(Event::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
pub type TrayHandle = Arc<Mutex<Option<ksni::Handle<Tray>>>>;
pub fn start_tray(backend: &Backend) -> TrayHandle {
    let handle: TrayHandle = Arc::default();
    let out = handle.clone();
    let proxy = backend.proxy.clone();
    backend.rt.spawn(async move {
        match (Tray {
            proxy: proxy.clone(),
            unread: 0,
            status: "Ready".into(),
        })
        .spawn()
        .await
        {
            Ok(tray) => {
                *out.lock().unwrap() = Some(tray);
                let _ = proxy.send_event(Event::TrayStatus(true));
            }
            Err(_) => {
                let _ = proxy.send_event(Event::TrayStatus(false));
            }
        }
    });
    handle
}
pub fn update_tray(backend: &Backend, handle: &TrayHandle, unread: i64, status: String) {
    let handle = handle.lock().unwrap().clone();
    if let Some(handle) = handle {
        backend.rt.spawn(async move {
            handle
                .update(move |t| {
                    t.unread = unread;
                    t.status = status;
                })
                .await;
        });
    }
}
pub fn notify(backend: &Backend, count: usize) {
    let proxy = backend.proxy.clone();
    backend.rt.spawn_blocking(move || {
        if let Ok(notification) = notify_rust::Notification::new()
            .appname("Ruby Reader")
            .summary("A few new lovely things")
            .body(&format!(
                "{count} new {} in your collection",
                if count == 1 { "story" } else { "stories" }
            ))
            .icon("ruby-reader")
            .action("open", "Read articles")
            .timeout(7000)
            .show()
        {
            notification.wait_for_action(|action| {
                if action == "open" || action == "default" {
                    let _ = proxy.send_event(Event::Open);
                }
            });
        }
    });
}
pub fn open_url(raw: &str) {
    if let Ok(url) = url::Url::parse(raw)
        && (reader_core::content::web_url(&url) || url.scheme() == "mailto")
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url.as_str())
            .spawn();
    }
}

pub struct Instance {
    path: std::path::PathBuf,
}
impl Drop for Instance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
pub fn single_instance(
    path: &Path,
    proxy: EventLoopProxy<Event>,
) -> anyhow::Result<Option<Instance>> {
    use std::io::{Read, Write};
    use std::os::unix::{
        fs::FileTypeExt,
        net::{UnixListener, UnixStream},
    };
    let socket = path.join("instance.sock");
    if let Ok(mut stream) = UnixStream::connect(&socket) {
        stream.write_all(b"open\n")?;
        return Ok(None);
    }
    if let Ok(meta) = std::fs::symlink_metadata(&socket) {
        anyhow::ensure!(
            meta.file_type().is_socket(),
            "Instance path is occupied by a non-socket file"
        );
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut bytes = [0; 16];
            if stream.read(&mut bytes).is_ok() {
                let _ = proxy.send_event(Event::Open);
            }
        }
    });
    Ok(Some(Instance { path: socket }))
}
