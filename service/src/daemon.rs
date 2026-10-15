// The long-running half: one queue that every request goes through, fed by
// the kernel log (the two ScreenPad keys) and by a socket (the command line,
// and so the key bindings in hypr.lua).

use crate::screenpad::ScreenPad;
use crate::state::Mode;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use systemd::journal::{self, JournalSeek};

pub enum Request {
    Line(String, Option<Sender<String>>),
}

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join("zenbook-duo-screenpad.sock")
}

pub fn run() -> Result<(), String> {
    let path = socket_path();
    // One instance only: a second one would act on every key press again.
    if UnixStream::connect(&path).is_ok() {
        return Err("already running".into());
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;

    let (tx, rx) = mpsc::channel();
    let keys = tx.clone();
    thread::spawn(move || {
        if let Err(e) = watch_keys(keys) {
            eprintln!("key watch stopped: {e}");
            std::process::exit(1);
        }
    });
    thread::spawn(move || serve(listener, tx));
    process(rx);
    Ok(())
}

fn process(rx: Receiver<Request>) {
    let mut pad = ScreenPad::start();
    eprintln!("started in mode {}", pad.mode.name());
    for Request::Line(line, reply) in rx {
        // One line per key press or command: what came in, and from where.
        eprintln!("{} {line}", if reply.is_some() { "command" } else { "key" });
        let answer = match handle(&mut pad, &line) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("{line}: {e}");
                format!("error: {e}")
            }
        };
        if let Some(reply) = reply {
            let _ = reply.send(answer);
        }
    }
}

fn handle(pad: &mut ScreenPad, line: &str) -> Result<String, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["cycle"] => pad.cycle().map(|_| pad.mode.name().into()),
        ["mode", name] => {
            let mode = Mode::parse(name).ok_or("mode is off, desktop or touchpad")?;
            pad.set_mode(mode).map(|_| pad.mode.name().into())
        }
        ["switch"] => pad.switch().map(|_| "ok".into()),
        ["brightness", "up"] => pad.brightness(true).map(|_| "ok".into()),
        ["brightness", "down"] => pad.brightness(false).map(|_| "ok".into()),
        ["brightness", "auto-up"] => pad.brightness_follow_focus(true).map(|_| "ok".into()),
        ["brightness", "auto-down"] => pad.brightness_follow_focus(false).map(|_| "ok".into()),
        ["status"] => Ok(format!("mode {}", pad.mode.name())),
        _ => Err(format!("unknown command: {line}")),
    }
}

fn serve(listener: UnixListener, tx: Sender<Request>) {
    for stream in listener.incoming().flatten() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                return;
            }
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send(Request::Line(line.trim().to_string(), Some(reply_tx))).is_err() {
                return;
            }
            if let Ok(answer) = reply_rx.recv() {
                let _ = (&stream).write_all(format!("{answer}\n").as_bytes());
            }
        });
    }
}

// The keys emit WMI codes asus-wmi has no keymap entry for, so they never
// become input events; the driver only logs them. Follow the kernel log for
// those lines, from now on: a key pressed before this started is not news.
// Drop this once asus-wmi maps the codes:
// https://github.com/OpenGamingCollective/asusctl/issues/372
fn watch_keys(tx: Sender<Request>) -> Result<(), String> {
    let mut log = journal::OpenOptions::default()
        .system(true)
        .local_only(true)
        .open()
        .map_err(|e| format!("cannot open the journal: {e}"))?;
    log.match_add("_TRANSPORT", "kernel").map_err(|e| e.to_string())?;
    log.seek(JournalSeek::Tail).map_err(|e| e.to_string())?;
    // Seeking to the tail points past the last entry; step onto it, so that
    // the next entry is the first new one.
    let _ = log.previous();
    loop {
        while log.next().map_err(|e| e.to_string())? > 0 {
            let Ok(Some(field)) = log.get_data("MESSAGE") else { continue };
            let Some(message) = field.value() else { continue };
            let message = String::from_utf8_lossy(message);
            let command = if message.contains("Unknown key code 0x6a") {
                "cycle"
            } else if message.contains("Unknown key code 0x9c") {
                "switch"
            } else {
                continue;
            };
            if tx.send(Request::Line(command.into(), None)).is_err() {
                return Ok(());
            }
        }
        log.wait(None).map_err(|e| e.to_string())?;
    }
}
