# Zenbook Duo 14 ScreenPad

The second screen of the ASUS Zenbook Duo 14 (UX481/UX482), the ScreenPad
Plus, under [Omarchy](https://omarchy.org/). Tested on a UX482EG.

## What it does

| Key | Does |
|---|---|
| ScreenPad key (next to the power button) | Cycles the ScreenPad: desktop, touchpad, off |
| Swap key (left of it) | Throws the focused window to the other screen, keeping focus where it was |
| Sun keys | Brightness of the screen in focus, the ScreenPad included |
| `SUPER + ALT + P` | Same as the ScreenPad key |
| `SUPER + ALT + O` / `L` | ScreenPad brightness up / down |

**Off** really is off: the panel goes dark first, then Hyprland stops using it,
its workspaces move to the laptop's own screen, and its touchscreen and pen are
switched off so a touch on the dark panel does nothing. **On** brings it back
with the brightness it had. Off and desktop survive a restart; a touchpad
comes back as a desktop.

**Touchpad** turns the ScreenPad into a large touchpad: a dim grey surface
with a honeycomb texture and the Omarchy logo, in the colours of your theme.
One finger moves the pointer, a tap clicks, two fingers scroll, all with your
own touchpad settings. The ScreenPad leaves the desktop meanwhile: its
workspaces go to the laptop's screen and the pointer cannot cross onto it.
The touchpad keeps a brightness of its own, lower than the desktop's.

The swap key works between two desktops only. Throwing a window down puts it on
the workspace the ScreenPad shows; throwing it up puts it on the lowest empty
workspace that belongs to the laptop's screen. Which workspaces belong where is
taken from [Display Setups](https://github.com/gillmania/omarchy-display-setups)
when it is installed, and from your own Hyprland rules otherwise.

## Why it is needed

- **Both keys** send WMI codes (`0x6a`, `0x9c`) that the kernel's `asus-wmi`
  driver has no keymap entry for. The driver only logs them, so they never
  reach Hyprland and cannot be bound. The service follows the kernel log for
  those lines instead. Reported upstream in
  [asusctl#372](https://github.com/OpenGamingCollective/asusctl/issues/372);
  once the codes are mapped, the keys can be bound directly.
- **The sun keys** do nothing on the ScreenPad with Omarchy's own bindings:
  the ScreenPad is an internal panel named `DP-1`, Omarchy takes it for an
  external screen and tries DDC/CI, which it never answers. Its brightness goes
  through `asusctl`, which owns the device and keeps the level across reboots.
- **[Display Setups](https://github.com/gillmania/omarchy-display-setups)**,
  if you use it, would switch the ScreenPad back on by its profile. The service
  asks it to hold the display instead (Display Setups 0.8.0 and later), so the
  key always wins. Older versions cannot hold it: the touchpad mode is then
  left out and the key goes from desktop straight to off, with a reminder to
  update.

## Install

Needs `asusctl` with `asusd` running. Display Setups is not needed; if you
use it, use 0.8.0 or later.

### From a release

Download the package, check it against the SHA-256 below, and install that
file. A release can be replaced after the fact; the checksum written here
cannot, so a package that does not match is never installed.

```bash
cd "$(mktemp -d)"
curl -fLO https://github.com/gillmania/zenbook-duo-screenpad/releases/download/v0.1.1/zenbook-duo-screenpad-0.1.1-1-x86_64.pkg.tar.zst
echo "e81237d0eba36a5dbebe4f28d6c3512b45737352534e972d6a2d8eb9af5b7e55  zenbook-duo-screenpad-0.1.1-1-x86_64.pkg.tar.zst" | sha256sum -c -
sudo pacman -U ./zenbook-duo-screenpad-0.1.1-1-x86_64.pkg.tar.zst
```

Run the lines one at a time, and stop if `sha256sum` does not print `OK`.

pacman then prints three steps to finish, as your own user: start the
touchpad helper, start the service, and add one line to
`~/.config/hypr/hyprland.lua` that loads the key bindings. The touchpad surface
is the shell plugin in this repository:

```bash
omarchy plugin add https://github.com/gillmania/zenbook-duo-screenpad.git --enable
```

### From source

Needs Rust (`omarchy pkg add rustup && rustup default stable`).

```bash
git clone https://github.com/gillmania/zenbook-duo-screenpad.git
cd zenbook-duo-screenpad
./install.sh
```

`install.sh` builds the service, installs it to `~/.local/bin` with a systemd
user unit, and starts it. It asks before it adds one line to the end of
`~/.config/hypr/hyprland.lua` that loads
[`hypr/zenbook-duo-screenpad.lua`](hypr/zenbook-duo-screenpad.lua) (the key
bindings, and keeping the ScreenPad off across config reloads), and before it
removes the older `zenbook-brightness`, `zenbook-screenpad` and
`zenbook-switch` if they are installed. A backup of `hyprland.lua` is made
before each change. It also asks before it installs the touchpad helper (see
below), which needs your password once, and installs the shell plugin that
draws the touchpad surface. Run it again after pulling a new version, and
restart the shell (`omarchy restart shell`) when `Service.qml` has changed.

## Remove

Installed from a release: `sudo pacman -R zenbook-duo-screenpad`, then
`omarchy plugin remove io.github.gillmania.zenbook-duo-screenpad`, the
`hyprland.lua` line, and the last two commands below. From source:

```bash
sudo systemctl disable --now zenbook-duo-screenpad-touchpad@$(id -u).service
sudo rm /etc/systemd/system/zenbook-duo-screenpad-touchpad@.service /usr/local/bin/zenbook-duo-screenpad-touchpad
omarchy plugin remove io.github.gillmania.zenbook-duo-screenpad
systemctl --user disable --now zenbook-duo-screenpad.service
rm ~/.config/systemd/user/zenbook-duo-screenpad.service ~/.local/bin/zenbook-duo-screenpad
omarchy-shell display-setups unhold "desc:BOE 0x0921" "ScreenPad key"
rm -r "${XDG_STATE_HOME:-$HOME/.local/state}/zenbook-duo-screenpad"
```

Then delete the line that loads `zenbook-duo-screenpad.lua` from
`~/.config/hypr/hyprland.lua`, and run `hyprctl reload`.

## Command line

```
zenbook-duo-screenpad cycle                          # what the ScreenPad key does
zenbook-duo-screenpad mode off|desktop|touchpad
zenbook-duo-screenpad switch                         # what the swap key does
zenbook-duo-screenpad brightness up|down             # the ScreenPad
zenbook-duo-screenpad brightness auto-up|auto-down   # the screen in focus
zenbook-duo-screenpad status
```

Every command goes through the running service, one at a time, so a key press
and a command can never step on each other.

## How it works

- `service/` is a small Rust program. Run as `daemon` (by the systemd unit), it
  follows the kernel log through libsystemd and listens on a socket in
  `$XDG_RUNTIME_DIR`. Idle, it sleeps; it costs well under a tenth of a percent
  of one core.
- The mode and the ScreenPad's last brightness are kept in
  `~/.local/state/zenbook-duo-screenpad/`, written atomically.
- `hypr/zenbook-duo-screenpad.lua` binds the keys and, at every config load,
  puts the saved mode back: off switches the ScreenPad and its touch input
  off, and a touchpad keeps its touches away from the desktop. A reload from
  anywhere therefore cannot light it up, even before the service or the shell
  is running.
- With Display Setups, the service asks it to hold the ScreenPad on, off or
  reserved (out of the desktop). Without it, Hyprland's own config does the
  work: the service saves the new mode and reloads the config, so the
  ScreenPad comes back with your own monitor and workspace rules. For a
  touchpad, the Lua file then moves it far to the right of the desktop, where
  no pointer reaches, and sends its workspaces to the laptop's screen.
- **The touchpad helper** (`zenbook-duo-screenpad-touchpad@<uid>.service`) is
  the only part that runs as root, because only root may open `/dev/uinput`
  and grab the touchscreen. It does nothing else: it creates one fixed device,
  a virtual touchpad named "ScreenPad Touchpad", and takes two commands, `on`
  and `off`, from one user over a socket only that user can open. The unit
  allows it nothing but those devices (no network, no home directories, a
  read-only system). It reports to systemd's watchdog from its main loop, so a
  hang while the touchscreen is grabbed restarts it and lets go; touches older
  than 100 ms are dropped rather than replayed as clicks.
- `Service.qml` is the Omarchy shell plugin: it draws the touchpad surface,
  once and without animation, so Hyprland has nothing to redraw.

## License

MIT. See [LICENSE](LICENSE).
