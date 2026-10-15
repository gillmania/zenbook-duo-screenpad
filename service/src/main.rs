// zenbook-duo-screenpad: the ScreenPad Plus of an ASUS Zenbook Duo 14 under
// Omarchy. `daemon` runs the service; every other command is sent to it, so
// that keys and commands go through the same queue.

mod daemon;
mod screenpad;
mod state;
mod system;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

const USAGE: &str = "usage: zenbook-duo-screenpad <command>

  daemon                         run the service (systemd starts it)
  cycle                          what the ScreenPad key does: off, desktop, touchpad
  mode off|desktop|touchpad      put the ScreenPad in that mode
  switch                         what the swap key does
  brightness up|down             the ScreenPad's brightness
  brightness auto-up|auto-down   the screen in focus
  status                         the current mode";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("daemon") => match daemon::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("zenbook-duo-screenpad: {e}");
                ExitCode::FAILURE
            }
        },
        Some(_) => send(&args.join(" ")),
    }
}

fn send(line: &str) -> ExitCode {
    let path = daemon::socket_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        eprintln!("zenbook-duo-screenpad: the service is not running ({})", path.display());
        return ExitCode::FAILURE;
    };
    if stream.write_all(format!("{line}\n").as_bytes()).is_err() {
        return ExitCode::FAILURE;
    }
    let mut answer = String::new();
    let _ = BufReader::new(&stream).read_line(&mut answer);
    let answer = answer.trim();
    println!("{answer}");
    if answer.starts_with("error:") || answer.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
