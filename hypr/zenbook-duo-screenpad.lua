-- Loaded from the end of ~/.config/hypr/hyprland.lua (install.sh adds the
-- line, after asking). Skipped when the file is gone, so removing the project
-- cannot break the rest of the configuration.

local home = os.getenv("HOME")
local bin = home .. "/.local/bin/zenbook-duo-screenpad"
local packaged = io.open("/usr/bin/zenbook-duo-screenpad")
if packaged then
	packaged:close()
	bin = "/usr/bin/zenbook-duo-screenpad"
end

-- ---- Off, at every config load ----
-- A reload from anywhere (Display Setups' safety timer, handing the screens
-- back to the config, login) passes through here, so a ScreenPad switched off
-- stays off even before the service or the shell is up. The mode is read as
-- data, never loaded as code.
local state = os.getenv("XDG_STATE_HOME") or (home .. "/.local/state")
local file = io.open(state .. "/zenbook-duo-screenpad/mode", "r")
if file then
	local mode = file:read("*l")
	file:close()
	if mode == "off" then
		-- Matched on the EDID description: the connector name can change.
		hl.monitor({ output = "desc:BOE 0x0921", disabled = true })
		hl.device({ name = "elan9009:00-04f3:2c1b", enabled = false })
		hl.device({ name = "elan9009:00-04f3:2c1b-stylus", enabled = false })
	end
end

-- ---- Keys ----
-- The ScreenPad key and the swap key never reach Hyprland (the service reads
-- them from the kernel log), so only the keyboard stand-ins are bound here.
o.bind("SUPER + ALT + P", "ScreenPad mode", bin .. " cycle")
o.bind("SUPER + ALT + O", "ScreenPad brightness up", bin .. " brightness up")
o.bind("SUPER + ALT + L", "ScreenPad brightness down", bin .. " brightness down")
-- The Fn brightness keys come from a separate device that handles modifiers
-- unevenly (with SUPER+ALT held, up fires and down does not), so O/L above
-- are the dependable pair.
o.bind("SUPER + ALT + XF86MonBrightnessUp", "ScreenPad brightness up (Fn)", bin .. " brightness up")
o.bind("SUPER + ALT + XF86MonBrightnessDown", "ScreenPad brightness down (Fn)", bin .. " brightness down")

-- The plain sun keys follow focus: Omarchy takes the ScreenPad for an external
-- screen (it is named DP-1) and tries DDC/CI, which it never answers.
hl.unbind("XF86MonBrightnessUp")
hl.unbind("XF86MonBrightnessDown")
o.bind("XF86MonBrightnessUp", "Brightness up (follows focus)", bin .. " brightness auto-up")
o.bind("XF86MonBrightnessDown", "Brightness down (follows focus)", bin .. " brightness auto-down")
