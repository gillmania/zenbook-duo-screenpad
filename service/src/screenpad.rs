// The ScreenPad's modes and the two keys that belong to it.
//
// Every request goes through one queue (see daemon.rs), so a mode change is
// never interleaved with another. Each step of a change can be run again, so
// a change cut short by a crash is finished by the next one.

use crate::state::{self, Mode};
use crate::system::{self, Hold};
use std::thread;
use std::time::Duration;

const DEFAULT_BRIGHTNESS: u8 = 50;
// A touchpad only has to be seen: grey and dim, not read.
const DEFAULT_TOUCHPAD_BRIGHTNESS: u8 = 15;
const STEP: i16 = 5;

fn default_brightness(mode: Mode) -> u8 {
    if mode == Mode::Touchpad { DEFAULT_TOUCHPAD_BRIGHTNESS } else { DEFAULT_BRIGHTNESS }
}

pub struct ScreenPad {
    pub mode: Mode,
}

impl ScreenPad {
    // The mode from the last run, or, the first time, what is on screen now.
    pub fn start() -> ScreenPad {
        let mode = state::load_mode().unwrap_or_else(|| match system::pad() {
            Some(pad) if pad.enabled && system::pad_brightness().unwrap_or(0) > 0 => Mode::Desktop,
            _ => Mode::Off,
        });
        let mut pad = ScreenPad { mode };
        // Off is put in force again, in case the last change was cut short. A
        // touchpad does not outlive this service: the helper may still hold
        // the touchscreen, so it is let go and the ScreenPad becomes a desktop.
        // A desktop needs nothing: the profile already shows it.
        let restore = match mode {
            Mode::Off => Some(Mode::Off),
            Mode::Touchpad => Some(Mode::Desktop),
            Mode::Desktop => None,
        };
        if let Some(to) = restore {
            if let Err(e) = pad.set_mode(to) {
                eprintln!("could not restore {}: {e}", to.name());
            }
        }
        pad
    }

    // The ScreenPad key: off, desktop, touchpad, and off again.
    pub fn cycle(&mut self) -> Result<(), String> {
        let next = match self.mode {
            Mode::Off => Mode::Desktop,
            Mode::Desktop => Mode::Touchpad,
            Mode::Touchpad => Mode::Off,
        };
        self.set_mode(next)
    }

    pub fn set_mode(&mut self, mode: Mode) -> Result<(), String> {
        // Leaving a touchpad, the touchscreen is let go before anything else.
        if mode != Mode::Touchpad {
            let _ = system::touchpad("off");
        }
        let result = match mode {
            Mode::Off => self.go_off(),
            Mode::Desktop => self.go_desktop(),
            Mode::Touchpad => self.go_touchpad(),
        };
        if result.is_ok() {
            self.mode = mode;
            state::save_mode(mode).map_err(|e| format!("could not save the mode: {e}"))?;
        }
        system::tell_shell(self.mode.name());
        result
    }

    // The level the panel has now is kept for its mode, for next time. Only a
    // lit mode has one: putting off back in force at startup finds whatever
    // level asusd left on a dark panel, which says nothing.
    fn keep_brightness(&self) {
        if self.mode != Mode::Off {
            if let Some(pct) = system::pad_brightness().filter(|p| *p > 0) {
                let _ = state::save_brightness(self.mode, pct);
            }
        }
    }

    fn light(mode: Mode) -> Result<(), String> {
        // Light last, once there is a picture to light.
        for _ in 0..20 {
            if system::pad().is_some_and(|p| p.enabled) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let pct = state::load_brightness(mode).unwrap_or(default_brightness(mode));
        system::set_pad_brightness(pct)?;
        system::osd_level(pct);
        Ok(())
    }

    fn go_off(&mut self) -> Result<(), String> {
        let Some(pad) = system::pad() else { return Err("the ScreenPad is not connected".into()) };
        self.keep_brightness();
        let was = self.mode;
        // Dark first, so the panel is never lit with no picture on it.
        system::set_pad_brightness(0)?;
        match system::hold("off") {
            Hold::Done => {}
            Hold::Refused(why) => {
                // Nothing else is switched off, so a lit panel comes back as it
                // was. A panel that was off already stays dark.
                if was != Mode::Off {
                    let _ = system::set_pad_brightness(state::load_brightness(was).unwrap_or(default_brightness(was)));
                    system::osd_text("ScreenPad stays on");
                }
                return Err(why);
            }
            Hold::Unavailable => {
                system::hypr_eval(&format!("hl.monitor({{ output = \"{}\", disabled = true }})", pad.name))?
            }
        }
        system::set_pad_inputs(false)?;
        system::osd_text("ScreenPad off");
        Ok(())
    }

    fn go_desktop(&mut self) -> Result<(), String> {
        let was = self.mode;
        self.keep_brightness();
        system::set_pad_inputs(true)?;
        match system::hold("on") {
            Hold::Done => {}
            Hold::Refused(why) => {
                system::osd_text("ScreenPad stays off");
                return Err(why);
            }
            Hold::Unavailable => {
                system::hypr_eval(&format!("hl.monitor({{ output = \"{}\", disabled = false }})", system::PAD_SELECTOR))?
            }
        }
        Self::light(Mode::Desktop)?;
        if was == Mode::Touchpad {
            self.settle_focus();
        }
        Ok(())
    }

    // On, but out of the desktop: Display Setups moves it away from the other
    // screens and its workspaces elsewhere, and the helper turns its touches
    // into a touchpad. Hyprland's own handling of the touchscreen and the pen
    // is switched off, so nothing reaches the desktop even before the grab.
    fn go_touchpad(&mut self) -> Result<(), String> {
        let was = self.mode;
        self.keep_brightness();
        system::set_pad_inputs(false)?;
        match system::hold("reserved") {
            Hold::Done => {}
            Hold::Refused(why) => {
                if was == Mode::Desktop {
                    let _ = system::set_pad_inputs(true);
                }
                system::osd_text("No touchpad now");
                return Err(why);
            }
            Hold::Unavailable => system::hypr_eval(&format!(
                "hl.monitor({{ output = \"{}\", disabled = false, position = \"10000x10000\" }})",
                system::PAD_SELECTOR
            ))?,
        }
        if let Err(e) = system::touchpad("on") {
            // Back to what it was rather than a lit panel that does nothing.
            system::osd_text("Touchpad unavailable");
            let back = if was == Mode::Off { Mode::Off } else { Mode::Desktop };
            self.mode = Mode::Touchpad;
            let _ = match back {
                Mode::Off => self.go_off(),
                _ => self.go_desktop(),
            };
            self.mode = back;
            let _ = state::save_mode(back);
            return Err(e);
        }
        Self::light(Mode::Touchpad)
    }

    // Back from a touchpad, the ScreenPad shows its own workspace again and
    // focus stays on the main screen.
    fn settle_focus(&self) {
        let monitors = system::monitors();
        let (Some(main), Some(pad)) = (
            monitors.iter().find(|m| m.is_main() && m.enabled),
            monitors.iter().find(|m| m.is_pad() && m.enabled),
        ) else {
            return;
        };
        let pad_ids: Vec<u32> = system::workspace_owners()
            .into_iter()
            .filter(|(_, m)| *m == pad.name)
            .map(|(id, _)| id)
            .collect();
        if let Err(e) = system::settle_focus(&main.name, &pad.name, &pad_ids) {
            eprintln!("could not settle focus: {e}");
        }
    }

    // ---- Brightness ----

    // The ScreenPad's own brightness keys. Each lit mode keeps its own level.
    pub fn brightness(&mut self, up: bool) -> Result<(), String> {
        if self.mode == Mode::Off {
            system::osd_text("ScreenPad is off");
            return Ok(());
        }
        let now = system::pad_brightness().unwrap_or(default_brightness(self.mode)) as i16;
        let pct = (now + if up { STEP } else { -STEP }).clamp(0, 100) as u8;
        system::set_pad_brightness(pct)?;
        if pct > 0 {
            let _ = state::save_brightness(self.mode, pct);
        }
        system::osd_level(pct);
        Ok(())
    }

    // The sun keys: the ScreenPad when it has focus, the screen in focus
    // otherwise. Omarchy's own keys take the ScreenPad for an external screen
    // (it is named DP-1) and try DDC/CI, which it never answers.
    pub fn brightness_follow_focus(&mut self, up: bool) -> Result<(), String> {
        let pad_focused = system::monitors().iter().any(|m| m.is_pad() && m.focused && m.enabled);
        if pad_focused {
            return self.brightness(up);
        }
        system::main_brightness(if up { "+5%" } else { "5%-" });
        Ok(())
    }

    // ---- The swap key ----

    // Throw the focused window to the other screen and keep focus on this one.
    // Only between two desktops: with the ScreenPad off there is nowhere to go.
    // The OSD has room for a few words only.
    pub fn switch(&mut self) -> Result<(), String> {
        let monitors = system::monitors();
        let main = monitors.iter().find(|m| m.is_main() && m.enabled);
        let pad = monitors.iter().find(|m| m.is_pad() && m.enabled);
        let (Some(main), Some(pad)) = (main, pad) else {
            system::osd_text(if self.mode == Mode::Off { "ScreenPad is off" } else { "No second screen" });
            return Ok(());
        };
        if self.mode != Mode::Desktop {
            system::osd_text(if self.mode == Mode::Touchpad { "ScreenPad is a touchpad" } else { "ScreenPad is off" });
            return Ok(());
        }
        let owners = system::workspace_owners();
        let main_ids: Vec<String> = owners.iter().filter(|(_, m)| *m == main.name).map(|(id, _)| id.to_string()).collect();
        system::hypr_eval(&switch_lua(&main.name, &pad.name, &main_ids.join(", ")))
    }
}

// Down (main -> ScreenPad): onto the workspace the ScreenPad shows.
// Up (ScreenPad -> main): onto the lowest empty workspace the main screen owns,
// which it then switches to; with none empty, onto the one it shows.
// On an empty screen the key moves focus across instead.
//
// Connector names come from Hyprland and ids are numbers, but both are checked
// before they go into Lua.
fn switch_lua(main: &str, pad: &str, main_ids: &str) -> String {
    let safe = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    let ids_ok = main_ids.chars().all(|c| c.is_ascii_digit() || c == ',' || c == ' ');
    if !safe(main) || !safe(pad) || !ids_ok {
        return "return".into();
    }
    format!(
        r#"
local main, pad
for _, m in ipairs(hl.get_monitors()) do
	if m.name == "{main}" then main = m end
	if m.name == "{pad}" then pad = m end
end
if not (main and pad) then return end
local up = pad.focused
if not (up or main.focused) then return end
local here, there = main, pad
if up then here, there = pad, main end

local function count(ws)
	local n = 0
	if ws then
		for _, w in ipairs(ws:get_windows()) do
			if w.mapped then n = n + 1 end
		end
	end
	return n
end

local src = here.active_workspace
if count(src) == 0 then
	hl.dispatch(hl.dsp.focus({{ workspace = there.active_workspace.id }}))
	return
end

local w = hl.get_active_window()
if not (w and w.workspace and w.workspace.id == src.id) then return end

local target = there.active_workspace.id
if up then
	for _, i in ipairs({{ {main_ids} }}) do
		if count(hl.get_workspace(i)) == 0 then target = i; break end
	end
end

hl.dispatch(hl.dsp.window.move({{ window = w, workspace = target, follow = false }}))

-- Going up, the main screen has to show the target for the window to be seen.
-- Emptying this screen leaves keyboard focus on the thrown window while
-- Hyprland still calls this workspace active; the round trip settles it.
if up or count(hl.get_workspace(src.id)) == 0 then
	hl.dispatch(hl.dsp.focus({{ workspace = target }}))
	hl.dispatch(hl.dsp.focus({{ workspace = src.id }}))
end
"#
    )
}
