// Everything this service asks of the rest of the system: Hyprland, Display
// Setups, asusd and Omarchy's on-screen display. Each is a short command; none
// is on the input path.

use serde_json::Value;
use std::process::{Command, Stdio};

// The ScreenPad, by its EDID description: the connector name can change.
pub const PAD_DESC: &str = "BOE 0x0921";
pub const PAD_SELECTOR: &str = "desc:BOE 0x0921";
// Its touchscreen and pen, as Hyprland names them.
pub const PAD_INPUTS: [&str; 2] = ["elan9009:00-04f3:2c1b", "elan9009:00-04f3:2c1b-stylus"];
const BACKLIGHT: &str = "/sys/class/backlight/asus_screenpad";
const OWNER: &str = "ScreenPad key";

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn hyprctl_json(what: &str) -> Option<Value> {
    serde_json::from_str(&output("hyprctl", &["-j", what])?).ok()
}

// `hyprctl eval` answers "ok", or the Lua error.
pub fn hypr_eval(lua: &str) -> Result<(), String> {
    match output("hyprctl", &["eval", lua]) {
        Some(answer) if answer.trim() == "ok" => Ok(()),
        Some(answer) => Err(answer.trim().to_string()),
        None => Err("hyprctl did not answer".into()),
    }
}

#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub focused: bool,
}

impl Monitor {
    pub fn is_pad(&self) -> bool {
        self.description.contains(PAD_DESC)
    }

    // The laptop's own panel, the one the ScreenPad sits under.
    pub fn is_main(&self) -> bool {
        self.name.starts_with("eDP-")
    }
}

pub fn monitors() -> Vec<Monitor> {
    let Some(Value::Array(list)) = hyprctl_json("monitors all") else { return vec![] };
    list.iter()
        .map(|m| Monitor {
            name: m["name"].as_str().unwrap_or("").to_string(),
            description: m["description"].as_str().unwrap_or("").to_string(),
            enabled: m["disabled"].as_bool() != Some(true),
            focused: m["focused"].as_bool() == Some(true),
        })
        .collect()
}

pub fn pad() -> Option<Monitor> {
    monitors().into_iter().find(Monitor::is_pad)
}

// Switch the ScreenPad's touchscreen and pen on or off in Hyprland. Off, a
// touch on the dark panel would otherwise land on the main screen.
pub fn set_pad_inputs(enabled: bool) -> Result<(), String> {
    let lua: Vec<String> = PAD_INPUTS
        .iter()
        .map(|name| format!("hl.device({{ name = \"{name}\", enabled = {enabled} }})"))
        .collect();
    hypr_eval(&lua.join("\n"))
}

// ---- Display Setups ----

pub enum Hold {
    Done,
    Refused(String),
    // Display Setups is not installed or not running.
    Unavailable,
}

fn display_setups(args: &[&str]) -> Hold {
    match output("omarchy-shell", &[&["display-setups"][..], args].concat()) {
        Some(answer) => {
            let answer = answer.trim();
            match answer.strip_prefix("refused: ") {
                Some(why) => Hold::Refused(why.to_string()),
                None if answer == "held" || answer == "released" => Hold::Done,
                None => Hold::Unavailable,
            }
        }
        None => Hold::Unavailable,
    }
}

// At login this service can start before the shell has read its holds;
// Display Setups then says so, and is asked again for up to half a minute
// rather than taken for a refusal.
pub fn hold(state: &str) -> Hold {
    for _ in 0..30 {
        match display_setups(&["hold", PAD_SELECTOR, state, OWNER]) {
            Hold::Refused(why) if why.contains("still starting") => {
                std::thread::sleep(std::time::Duration::from_secs(1))
            }
            answer => return answer,
        }
    }
    Hold::Refused("Display Setups did not finish starting".into())
}

// Workspace id (1..10) to the connector it belongs on, as Display Setups sees
// it, with Hyprland's own rules as the fallback.
pub fn workspace_owners() -> Vec<(u32, String)> {
    let from_setups = output("omarchy-shell", &["display-setups", "status"])
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|status| status["workspaceOwners"].as_object().cloned());
    if let Some(map) = from_setups {
        let mut owners: Vec<(u32, String)> = map
            .iter()
            .filter_map(|(id, name)| Some((id.parse().ok()?, name.as_str()?.to_string())))
            .collect();
        owners.sort();
        return owners;
    }
    // The last rule for a workspace is the one in force.
    let mut owners: Vec<(u32, String)> = vec![];
    if let Some(Value::Array(rules)) = hyprctl_json("workspacerules") {
        for rule in rules {
            let (Some(id), Some(monitor)) = (
                rule["workspaceString"].as_str().and_then(|s| s.parse::<u32>().ok()),
                rule["monitor"].as_str(),
            ) else { continue };
            if !(1..=10).contains(&id) {
                continue;
            }
            owners.retain(|(i, _)| *i != id);
            owners.push((id, monitor.to_string()));
        }
    }
    owners.sort();
    owners
}

// ---- Backlight ----

pub fn pad_brightness() -> Option<u8> {
    let now: u32 = std::fs::read_to_string(format!("{BACKLIGHT}/actual_brightness")).ok()?.trim().parse().ok()?;
    let max: u32 = std::fs::read_to_string(format!("{BACKLIGHT}/max_brightness")).ok()?.trim().parse().ok()?;
    (max > 0).then(|| ((now * 100 + max / 2) / max) as u8)
}

// Through asusctl rather than the sysfs file: asusd owns the device and keeps
// the level across reboots.
pub fn set_pad_brightness(pct: u8) -> Result<(), String> {
    let pct = pct.min(100).to_string();
    output("asusctl", &["backlight", "--screenpad-brightness", &pct])
        .map(|_| ())
        .ok_or_else(|| "asusctl failed".into())
}

pub fn main_brightness(step: &str) {
    let _ = output("omarchy", &["brightness", "display", step]);
}

// ---- On-screen display ----

// Like Omarchy's own brightness keys: the icon and the level as a percentage.
pub fn osd_level(pct: u8) {
    let pct = pct.to_string();
    let _ = Command::new("omarchy").args(["osd", "-i", "brightness", "-p", &pct]).status();
}

// ---- The touchpad helper ----

// The privileged helper (zenbook-duo-screenpad-touchpad@<uid>.service) that
// grabs the touchscreen and feeds a virtual touchpad. Only "on" and "off".
pub fn touchpad(command: &str) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    let uid = unsafe { libc::getuid() };
    let path = format!("/run/zenbook-duo-screenpad-touchpad-{uid}/socket");
    let mut stream = std::os::unix::net::UnixStream::connect(&path)
        .map_err(|_| "the touchpad helper is not running".to_string())?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    stream.write_all(format!("{command}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer).map_err(|e| e.to_string())?;
    match answer.trim() {
        a if a == command => Ok(()),
        a => Err(a.strip_prefix("error: ").unwrap_or(a).to_string()),
    }
}

// The shell plugin draws the touchpad surface; it is told the mode on every
// change. Nothing is lost if the shell is not running: it reads the mode file
// when it starts.
pub fn tell_shell(mode: &str) {
    let _ = output("omarchy-shell", &["zenbook-duo-screenpad", "show", mode]);
}

// ---- Focus ----

// Back from a touchpad: the ScreenPad shows the technical workspace Hyprland
// gave it, and focus may have followed a workspace down to it. Show its own
// first workspace again, then put focus back on the main screen.
pub fn settle_focus(main: &str, pad: &str, pad_ids: &[u32]) -> Result<(), String> {
    let safe = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !safe(main) || !safe(pad) {
        return Ok(());
    }
    let first = pad_ids.first().copied().unwrap_or(0);
    hypr_eval(&format!(
        r#"
local function find(name)
	for _, m in ipairs(hl.get_monitors()) do
		if m.name == name then return m end
	end
end
local pad = find("{pad}")
if not (pad and find("{main}")) then return end
if {first} > 0 and pad.active_workspace.id ~= {first} then
	hl.dispatch(hl.dsp.focus({{ workspace = {first} }}))
end
-- Read again: the workspace the main screen showed may have gone back down.
hl.dispatch(hl.dsp.focus({{ workspace = find("{main}").active_workspace.id }}))
"#
    ))
}

pub fn osd_text(text: &str) {
    let _ = Command::new("omarchy").args(["osd", "-m", text]).status();
}
