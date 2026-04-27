mod app;
mod pages;
mod presets;
mod widgets;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::panic;
use std::process::Child;

fn spawn_sidecar() -> Result<Child> {
    let exe_name = if cfg!(debug_assertions) {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let dev_path =
            std::path::Path::new(&manifest_dir).join("../sidecar/target/debug/coderouter-proxy");
        if dev_path.exists() {
            return Ok(std::process::Command::new(&dev_path).spawn()?);
        }
        "coderouter-proxy".to_string()
    } else {
        "coderouter-proxy".to_string()
    };

    Ok(std::process::Command::new(&exe_name).spawn()?)
}

fn kill_sidecar(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = execute!(io::stdout(), crossterm::cursor::Show);
        eprintln!("\nCodeRouter TUI crashed:");
        eprintln!("{}", info);
        eprintln!("\nTerminal state has been restored. Please report this issue.");
    }));
}

fn main() -> Result<()> {
    install_panic_hook();

    if let Err(e) = coderouter_proxy::metrics::db::init_db() {
        eprintln!("Warning: Failed to initialize metrics database: {e}");
    }

    let sidecar = spawn_sidecar().ok();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(sidecar);

    loop {
        terminal.draw(|frame| app.render(frame))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    if let Some(ref mut child) = app.sidecar {
        kill_sidecar(child);
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}
