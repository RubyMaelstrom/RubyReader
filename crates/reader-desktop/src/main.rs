mod backend;
mod decora;
mod desktop;
mod platform;
mod ui;

use backend::{Backend, Event};
use std::path::PathBuf;
use winit::event_loop::EventLoop;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!(
            "Ruby Reader\n\n  --data-dir PATH       Use a separate local library\n  --sample              Add the local sample scrapbook\n  --renderer auto|cpu|hybrid\n  --snapshot PATH       Render a headless PNG and exit\n  --width N --height N  Snapshot dimensions (default 1360×920)\n  --smoke               Exercise a native window, then exit\n"
        );
        return Ok(());
    }
    let value = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let path = value("--data-dir")
        .map(PathBuf::from)
        .unwrap_or_else(reader_core::db::data_directory);
    anyhow::ensure!(
        !args.contains(&"--smoke".into()) || value("--data-dir").is_some(),
        "Use --data-dir with --smoke to protect your normal library"
    );
    let new_directory = !path.exists();
    std::fs::create_dir_all(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if new_directory {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let renderer = value("--renderer").unwrap_or_else(|| "auto".into());
    ui::install_fonts();
    anyhow::ensure!(
        ["auto", "cpu", "hybrid"].contains(&renderer.as_str()),
        "Unknown renderer"
    );
    if let Some(output) = value("--snapshot") {
        return desktop::snapshot(
            &path,
            &PathBuf::from(output),
            args.contains(&"--sample".into()),
            value("--width")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1360),
            value("--height")
                .and_then(|s| s.parse().ok())
                .unwrap_or(920),
            &renderer,
        );
    }
    let event_loop = EventLoop::<Event>::with_user_event().build()?;
    let Some(_instance) = platform::single_instance(&path, event_loop.create_proxy())? else {
        return Ok(());
    };
    let backend = Backend::new(path, event_loop.create_proxy())?;
    if args.contains(&"--sample".into()) {
        backend.store.lock().unwrap().sample()?;
    }
    let mut app = desktop::App::new(backend, renderer, args.contains(&"--smoke".into()));
    event_loop.run_app(&mut app)?;
    Ok(())
}
