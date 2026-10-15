#!/bin/bash
# Build and install the ScreenPad service from this checkout, and load its
# Hyprland configuration. Asks before changing ~/.config/hypr/hyprland.lua.
# Safe to run again.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
conf="$HOME/.config/hypr/hyprland.lua"
bin="$HOME/.local/bin/zenbook-duo-screenpad"
unit_dir="$HOME/.config/systemd/user"
lua="$here/hypr/zenbook-duo-screenpad.lua"
line="local f = io.open(\"$lua\") if f then f:close() dofile(\"$lua\") end"

ask() {
	local answer
	read -r -p "$1 [y/N] " answer
	[[ $answer == [yY]* ]]
}

command -v cargo >/dev/null || { echo "Rust is needed to build: omarchy pkg add rustup && rustup default stable" >&2; exit 1; }

echo "Building..."
cargo build --release --quiet --manifest-path "$here/service/Cargo.toml"

# The service is stopped while its binary is replaced.
systemctl --user stop zenbook-duo-screenpad.service 2>/dev/null || true
mkdir -p "$(dirname "$bin")" "$unit_dir"
install -m 755 "$here/service/target/release/zenbook-duo-screenpad" "$bin"
install -m 644 "$here/systemd/zenbook-duo-screenpad.service" "$unit_dir/"

# The three projects this one replaces. Their key watchers would act on every
# key press as well, so they go before the new service starts.
old_lines=$(grep -nE 'zenbook-(brightness|screenpad|switch)/hypr\.lua' "$conf" || true)
old_watchers=$(pgrep -f 'zenbook-(screenpad|switch)-keywatch' || true)
if [[ -n $old_lines || -n $old_watchers ]]; then
	echo
	echo "The older zenbook-brightness, zenbook-screenpad and zenbook-switch are installed."
	[[ -n $old_lines ]] && echo "$old_lines" | sed 's/^/  hyprland.lua line /'
	if ask "Stop their key watchers and remove those lines and links?"; then
		cp "$conf" "$conf.bak.$(date +%s)"
		grep -vE 'zenbook-(brightness|screenpad|switch)/hypr\.lua' "$conf" >"$conf.tmp" && cat "$conf.tmp" >"$conf" && rm "$conf.tmp"
		pkill -f 'zenbook-(screenpad|switch)-keywatch' || true
		for old in zenbook-brightness zenbook-screenpad zenbook-screenpad-keywatch zenbook-switch zenbook-switch-keywatch; do
			[[ -L $HOME/.local/bin/$old ]] && rm "$HOME/.local/bin/$old"
		done
	else
		echo "Left in place. Both would answer the ScreenPad keys, so expect every press to count twice."
	fi
fi

if ! grep -qxF "$line" "$conf"; then
	echo
	echo "To bind its keys and keep the ScreenPad off across config reloads, this line"
	echo "goes at the end of $conf:"
	echo "  $line"
	if ask "Add it?"; then
		cp "$conf" "$conf.bak.$(date +%s)"
		printf '%s\n' "$line" >>"$conf"
	else
		echo "Not added. The keys work, but SUPER+ALT+P/O/L and the sun keys are not bound."
	fi
fi

# The touchpad helper is the one part that runs as root: it alone may open the
# touchscreen and /dev/uinput. See systemd/zenbook-duo-screenpad-touchpad@.service.
helper=/usr/local/bin/zenbook-duo-screenpad-touchpad
helper_unit="zenbook-duo-screenpad-touchpad@$(id -u).service"
echo
echo "The touchpad mode needs a small helper running as root, installed as"
echo "  $helper and /etc/systemd/system/zenbook-duo-screenpad-touchpad@.service"
if ask "Install it (asks for your password)?"; then
	sudo install -m 755 "$here/service/target/release/zenbook-duo-screenpad-touchpad" "$helper"
	sudo install -m 644 "$here/systemd/zenbook-duo-screenpad-touchpad@.service" /etc/systemd/system/
	sudo systemctl daemon-reload
	sudo systemctl enable "$helper_unit"
	sudo systemctl restart "$helper_unit"
else
	echo "Not installed. The ScreenPad key then skips the touchpad mode's touch input."
fi

# The shell plugin that draws the touchpad surface. A checkout made by
# 'omarchy plugin add' is left to 'omarchy plugin update'.
plugin_id=$(jq -r .id "$here/manifest.json")
plugin_dir="$HOME/.config/omarchy/plugins/$plugin_id"
if [[ ! -d $plugin_dir/.git ]]; then
	mkdir -p "$plugin_dir"
	cp "$here/manifest.json" "$here/Service.qml" "$here/LICENSE" "$plugin_dir/"
	omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true
	sleep 1
fi
omarchy plugin enable "$plugin_id" >/dev/null 2>&1 || true

systemctl --user daemon-reload
systemctl --user enable zenbook-duo-screenpad.service
systemctl --user restart zenbook-duo-screenpad.service
hyprctl reload >/dev/null
hyprctl configerrors
echo
for _ in 1 2 3 4 5 6 7 8 9 10; do "$bin" status >/dev/null 2>&1 && break; sleep 0.3; done
echo "Installed. Status: $("$bin" status 2>&1)"
