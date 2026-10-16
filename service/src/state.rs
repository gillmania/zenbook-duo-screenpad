// What survives a restart: the ScreenPad's mode and the brightness its desktop
// had, so that switching it on again brings that level back.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Desktop,
    Touchpad,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Desktop => "desktop",
            Mode::Touchpad => "touchpad",
        }
    }

    pub fn parse(text: &str) -> Option<Mode> {
        match text.trim() {
            "off" => Some(Mode::Off),
            "desktop" => Some(Mode::Desktop),
            "touchpad" => Some(Mode::Touchpad),
            _ => None,
        }
    }
}

fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    base.join("zenbook-duo-screenpad")
}

// Also read by hypr.lua at every config load, so off holds without this service.
pub fn mode_path() -> PathBuf {
    dir().join("mode")
}

// One level per mode that lights the panel: a desktop is read, a touchpad
// only has to be seen, so it is kept much dimmer.
fn brightness_path(mode: Mode) -> PathBuf {
    dir().join(match mode {
        Mode::Touchpad => "touchpad-brightness",
        _ => "desktop-brightness",
    })
}

// Written to a new file and renamed into place, then synced, so a crash or a
// power cut leaves the old value or the new one, never half of either.
fn write_atomic(path: &PathBuf, text: &str) -> io::Result<()> {
    fs::create_dir_all(dir())?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    fs::File::open(dir())?.sync_all()
}

pub fn load_mode() -> Option<Mode> {
    fs::read_to_string(mode_path()).ok().and_then(|t| Mode::parse(&t))
}

pub fn save_mode(mode: Mode) -> io::Result<()> {
    write_atomic(&mode_path(), &format!("{}\n", mode.name()))
}

pub fn load_brightness(mode: Mode) -> Option<u8> {
    let value: u8 = fs::read_to_string(brightness_path(mode)).ok()?.trim().parse().ok()?;
    (1..=100).contains(&value).then_some(value)
}

pub fn save_brightness(mode: Mode, pct: u8) -> io::Result<()> {
    write_atomic(&brightness_path(mode), &format!("{pct}\n"))
}

// The workspaces the service sent away from a ScreenPad it reserved itself,
// without Display Setups. Read by hypr.lua after a config reload.
fn reserved_path() -> PathBuf {
    dir().join("reserved")
}

pub fn save_reserved(ids: &str) -> io::Result<()> {
    write_atomic(&reserved_path(), &format!("{}\n", ids.trim()))
}

// The reservation, if there was one; it is gone afterwards.
pub fn take_reserved() -> Option<String> {
    let ids = fs::read_to_string(reserved_path()).ok()?;
    fs::remove_file(reserved_path()).ok()?;
    Some(ids)
}
