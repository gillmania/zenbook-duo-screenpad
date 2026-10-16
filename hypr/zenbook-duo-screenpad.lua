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

-- ---- Off and touchpad, at every config load ----
-- A reload from anywhere (Display Setups' safety timer, handing the screens
-- back to the config, login) passes through here, so a ScreenPad switched off
-- stays off even before the service or the shell is up. The mode is read as
-- data, never loaded as code.
local state = os.getenv("XDG_STATE_HOME") or (home .. "/.local/state")
local mode
local file = io.open(state .. "/zenbook-duo-screenpad/mode", "r")
if file then
	mode = file:read("*l")
	file:close()
	if mode == "off" then
		-- Matched on the EDID description: the connector name can change.
		hl.monitor({ output = "desc:BOE 0x0921", disabled = true })
	end
	-- A touchpad's touches go to the helper, never to the desktop.
	if mode == "off" or mode == "touchpad" then
		hl.device({ name = "elan9009:00-04f3:2c1b", enabled = false })
		hl.device({ name = "elan9009:00-04f3:2c1b-stylus", enabled = false })
	end
end

-- ---- Touchpad, without Display Setups ----
-- With Display Setups 0.8.0 or later, the service asks it to hold the
-- ScreenPad reserved. Without it, the config does it, after every load while
-- the mode is touchpad and the "reserved" file is there: the ScreenPad is
-- moved far to the right of the desktop, where no pointer reaches, and its
-- workspaces go to the main screen. The service writes the file (the ids of
-- the workspaces the ScreenPad owns, read as numbers only) and reloads. To
-- give the ScreenPad back it removes the file and reloads again, which brings
-- back the user's own monitor and workspace rules exactly.
local reserved_file = state .. "/zenbook-duo-screenpad/reserved"

local function reserved_ids()
	local f = io.open(reserved_file, "r")
	if not f then return nil end
	local text = f:read("*a") or ""
	f:close()
	local ids = {}
	for digits in text:gmatch("%d+") do
		local id = tonumber(digits)
		if id >= 1 and id <= 10 then ids[#ids + 1] = id end
	end
	return ids
end

-- A monitor rule set here replaces what the config said for that monitor, so
-- everything it has now is said again, not just the position.
local function place(m, position)
	hl.monitor({
		output = m.name,
		mode = m.width .. "x" .. m.height .. "@" .. string.format("%.3f", m.refresh_rate),
		scale = m.scale,
		transform = m.transform,
		position = position,
	})
end

local function reserve()
	local ids = reserved_ids()
	if not ids then return end
	local pad, main, right, top
	local others = {}
	for _, m in ipairs(hl.get_monitors()) do
		if m.description == "BOE 0x0921" then
			pad = m
		else
			others[#others + 1] = m
			local edge = m.x + m.width / m.scale
			if not right or edge > right then right = edge end
			if not top or m.y < top then top = m.y end
			if not main or m.name:find("^eDP%-") then main = m end
		end
	end
	if not pad then return end
	if not main then error("no other screen to take the ScreenPad's workspaces") end

	-- Workspaces on the ScreenPad now go too, not only those it owns.
	local seen = {}
	for _, id in ipairs(ids) do seen[id] = true end
	for _, ws in ipairs(hl.get_workspaces()) do
		if not ws.special and ws.monitor and ws.monitor.name == pad.name and ws.id >= 1 and ws.id <= 10 and not seen[ws.id] then
			seen[ws.id] = true
			ids[#ids + 1] = ws.id
		end
	end

	-- The other screens keep their places: with positions left to "auto",
	-- Hyprland would otherwise rearrange them around the moved ScreenPad.
	for _, m in ipairs(others) do place(m, m.x .. "x" .. m.y) end
	place(pad, math.floor(right + 10000) .. "x" .. top)
	for _, id in ipairs(ids) do
		hl.workspace_rule({ workspace = tostring(id), monitor = main.name })
		if hl.get_workspace(id) then
			hl.dispatch(hl.dsp.workspace.move({ workspace = id, monitor = main.name }))
		end
	end
	hl.dispatch(hl.dsp.focus({ monitor = main.name }))
end

hl.on("config.reloaded", function()
	if mode == "touchpad" then reserve() end
end)

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
