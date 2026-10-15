// The privileged half: turns the ScreenPad's touchscreen into a touchpad.
//
// While on, it grabs the ScreenPad touchscreen so nothing else sees its
// touches, and forwards the fingers to a virtual multitouch touchpad created
// through uinput. libinput treats that device as an ordinary touchpad, so
// pointer motion, tap to click, scrolling and gestures follow the user's own
// touchpad settings.
//
// It is the only part that touches raw input, and it does nothing else: it
// creates one fixed device and takes two commands, "on" and "off", from one
// user, over a socket only that user can open. Run by systemd as
// zenbook-duo-screenpad-touchpad@<uid>.service.
//
// A hang while grabbed would leave the ScreenPad dead to touch, so the loop
// reports to systemd's watchdog every few seconds and is restarted if it
// stops; touches older than STALE are dropped rather than replayed.

use evdev::raw_stream::RawDevice;
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode as Abs, AttributeSet, BusType, EventType, InputEvent, InputId,
    KeyCode as Key, PropType, SynchronizationCode as Syn, UinputAbsSetup,
};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

// The ScreenPad's touch controller (ELAN9009) on the I2C bus.
const VENDOR: u16 = 0x04f3;
const PRODUCT: u16 = 0x2c1b;
// Touches this much older than now are from a moment this process was not
// running, and are not turned into clicks after the fact.
const STALE: Duration = Duration::from_millis(100);
const WATCHDOG_EVERY_MS: i32 = 4000;
const TOOLS: [Key; 5] = [
    Key::BTN_TOOL_FINGER,
    Key::BTN_TOOL_DOUBLETAP,
    Key::BTN_TOOL_TRIPLETAP,
    Key::BTN_TOOL_QUADTAP,
    Key::BTN_TOOL_QUINTTAP,
];

fn main() {
    let uid: u32 = match std::env::args().nth(1).and_then(|a| a.parse().ok()) {
        Some(uid) => uid,
        None => {
            eprintln!("usage: zenbook-duo-screenpad-touchpad <uid>");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(uid) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

pub fn socket_path(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/zenbook-duo-screenpad-touchpad-{uid}/socket"))
}

fn run(uid: u32) -> Result<(), String> {
    let path = socket_path(uid);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    // The socket belongs to the one user this instance serves. Its mode is
    // set first: once it is that user's, this process (which may change
    // owners but holds no other privilege) can no longer change it.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict {}: {e}", path.display()))?;
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    if unsafe { libc::chown(c_path.as_ptr(), uid, u32::MAX) } != 0 {
        return Err(format!("cannot hand {} to uid {uid}", path.display()));
    }
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    notify("READY=1");
    let mut pad: Option<Touchpad> = None;
    loop {
        let mut fds = vec![libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        if let Some(p) = &pad {
            fds.push(libc::pollfd { fd: p.source.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, WATCHDOG_EVERY_MS) };
        // Reported from this loop, so a hang anywhere in it stops the reports.
        notify("WATCHDOG=1");
        if ready < 0 {
            continue;
        }
        if fds.len() > 1 && fds[1].revents != 0 {
            let lost = pad.as_mut().map(|p| p.forward().is_err()).unwrap_or(false);
            if lost {
                // The device went away (suspend, a driver reload): let go, and
                // grab it again when found, rather than spin on a dead fd.
                eprintln!("touchscreen lost, reconnecting");
                // The old grab and virtual device go first.
                drop(pad.take());
                pad = Touchpad::start().map_err(|e| eprintln!("{e}")).ok();
            }
        }
        if fds[0].revents != 0 {
            while let Ok((stream, _)) = listener.accept() {
                if peer_uid(&stream) != Some(uid) {
                    continue;
                }
                let answer = command(&stream, &mut pad);
                let _ = (&stream).write_all(format!("{answer}\n").as_bytes());
            }
        }
    }
}

fn command(stream: &UnixStream, pad: &mut Option<Touchpad>) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return "error: no command".into();
    }
    match line.trim() {
        "on" if pad.is_some() => "on".into(),
        "on" => match Touchpad::start() {
            Ok(p) => {
                *pad = Some(p);
                "on".into()
            }
            Err(e) => format!("error: {e}"),
        },
        "off" => {
            // Dropping it lets go of the touchscreen and removes the virtual
            // touchpad, with any finger still down lifted first.
            if let Some(mut p) = pad.take() {
                p.lift_all();
            }
            "off".into()
        }
        "status" => if pad.is_some() { "on" } else { "off" }.into(),
        other => format!("error: unknown command {other}"),
    }
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0).then_some(cred.uid)
}

// sd_notify, without a library: one datagram to the socket systemd names.
fn notify(state: &str) {
    if let Some(path) = std::env::var_os("NOTIFY_SOCKET") {
        if let Ok(socket) = UnixDatagram::unbound() {
            let _ = socket.send_to(state.as_bytes(), path);
        }
    }
}

// ---- The touchpad ----

struct Touchpad {
    source: RawDevice,
    sink: VirtualDevice,
    // Contacts down on the touchscreen, by slot.
    active: Vec<bool>,
    // Contacts that were down while touches were being dropped. They are not
    // forwarded until lifted, so a finger resting through a hang does not
    // turn into a sudden drag.
    ignored: Vec<bool>,
    slot: usize,
    tool: Option<Key>,
    frame: Vec<InputEvent>,
    dropping: bool,
}

fn find_touchscreen() -> Result<RawDevice, String> {
    for (_, dev) in evdev::raw_stream::enumerate() {
        let id = dev.input_id();
        let direct = dev.properties().contains(PropType::DIRECT);
        let mt = dev.supported_absolute_axes().is_some_and(|a| a.contains(Abs::ABS_MT_POSITION_X));
        let pen = dev.supported_keys().is_some_and(|k| k.contains(Key::BTN_TOOL_PEN));
        if id.vendor() == VENDOR && id.product() == PRODUCT && direct && mt && !pen {
            return Ok(dev);
        }
    }
    Err("the ScreenPad touchscreen was not found".into())
}

impl Touchpad {
    fn start() -> Result<Touchpad, String> {
        let mut source = find_touchscreen()?;
        let abs: Vec<(Abs, AbsInfo)> = source.get_absinfo().map_err(|e| e.to_string())?.collect();
        let info = |code| abs.iter().find(|(c, _)| *c == code).map(|(_, i)| *i);
        let x = info(Abs::ABS_MT_POSITION_X).ok_or("no ABS_MT_POSITION_X")?;
        let y = info(Abs::ABS_MT_POSITION_Y).ok_or("no ABS_MT_POSITION_Y")?;
        let slots = info(Abs::ABS_MT_SLOT).ok_or("no ABS_MT_SLOT")?;
        // The controller reports its own resolution (12 units/mm); libinput
        // needs it to size motion and gestures.
        if x.resolution() <= 0 || y.resolution() <= 0 {
            return Err("the touchscreen reports no resolution".into());
        }
        let x = AbsInfo::new(0, x.minimum(), x.maximum(), 0, 0, x.resolution());
        let y = AbsInfo::new(0, y.minimum(), y.maximum(), 0, 0, y.resolution());
        let slot_count = (slots.maximum() + 1).clamp(1, 10) as usize;

        let mut keys = AttributeSet::<Key>::new();
        for k in [Key::BTN_LEFT, Key::BTN_TOUCH].into_iter().chain(TOOLS) {
            keys.insert(k);
        }
        let mut props = AttributeSet::<PropType>::new();
        props.insert(PropType::POINTER);
        let e = |e: std::io::Error| e.to_string();
        // An identity of its own, so it matches no device-specific quirk.
        let sink = VirtualDevice::builder()
            .map_err(e)?
            .name("ScreenPad Touchpad")
            .input_id(InputId::new(BusType::BUS_VIRTUAL, 0x1209, 0x5350, 1))
            .with_keys(&keys)
            .map_err(e)?
            .with_properties(&props)
            .map_err(e)?
            .with_absolute_axis(&UinputAbsSetup::new(Abs::ABS_X, x))
            .map_err(e)?
            .with_absolute_axis(&UinputAbsSetup::new(Abs::ABS_Y, y))
            .map_err(e)?
            .with_absolute_axis(&UinputAbsSetup::new(Abs::ABS_MT_POSITION_X, x))
            .map_err(e)?
            .with_absolute_axis(&UinputAbsSetup::new(Abs::ABS_MT_POSITION_Y, y))
            .map_err(e)?
            .with_absolute_axis(&UinputAbsSetup::new(
                Abs::ABS_MT_SLOT,
                AbsInfo::new(0, 0, slot_count as i32 - 1, 0, 0, 0),
            ))
            .map_err(e)?
            .with_absolute_axis(&UinputAbsSetup::new(Abs::ABS_MT_TRACKING_ID, AbsInfo::new(0, 0, 65535, 0, 0, 0)))
            .map_err(e)?
            .build()
            .map_err(e)?;

        source.grab().map_err(|e| format!("cannot grab the touchscreen: {e}"))?;
        // Fingers already down when the grab starts are left to lift first.
        let state = source.get_abs_state().map_err(e)?;
        let slot = (state[Abs::ABS_MT_SLOT.0 as usize].value.max(0) as usize).min(slot_count - 1);
        Ok(Touchpad {
            source,
            sink,
            active: vec![false; slot_count],
            ignored: vec![false; slot_count],
            slot,
            tool: None,
            frame: vec![abs_event(Abs::ABS_MT_SLOT, slot as i32)],
            dropping: false,
        })
    }

    // Read what is waiting and forward it. An error means the device is gone.
    fn forward(&mut self) -> Result<(), ()> {
        let events: Vec<InputEvent> = self.source.fetch_events().map_err(|_| ())?.collect();
        for ev in events {
            match (ev.event_type(), ev.code()) {
                (EventType::SYNCHRONIZATION, c) if c == Syn::SYN_DROPPED.0 => self.drop_contacts(),
                (EventType::SYNCHRONIZATION, c) if c == Syn::SYN_REPORT.0 => {
                    if self.dropping {
                        self.dropping = false;
                        self.frame.clear();
                        continue;
                    }
                    if SystemTime::now().duration_since(ev.timestamp()).unwrap_or_default() > STALE {
                        self.drop_contacts();
                        self.dropping = false;
                        continue;
                    }
                    self.finish_frame();
                    let _ = self.sink.emit(&self.frame);
                    self.frame.clear();
                }
                (EventType::ABSOLUTE, c) => {
                    let code = Abs(c);
                    match code {
                        Abs::ABS_MT_SLOT => {
                            self.slot = (ev.value().max(0) as usize).min(self.active.len() - 1);
                        }
                        Abs::ABS_MT_TRACKING_ID => {
                            let down = ev.value() >= 0;
                            self.active[self.slot] = down;
                            if !down && self.ignored[self.slot] {
                                // Its lift was never seen downstream; nothing to send.
                                self.ignored[self.slot] = false;
                                continue;
                            }
                        }
                        Abs::ABS_MT_POSITION_X | Abs::ABS_MT_POSITION_Y | Abs::ABS_X | Abs::ABS_Y => {}
                        // Touch size and orientation: no use to a touchpad.
                        _ => continue,
                    }
                    if self.dropping || (self.ignored[self.slot] && code != Abs::ABS_MT_SLOT) {
                        continue;
                    }
                    self.frame.push(abs_event(code, ev.value()));
                }
                // BTN_TOUCH is derived below; timestamps are not forwarded.
                _ => {}
            }
        }
        Ok(())
    }

    // Lift every contact downstream, and ignore the ones still down upstream
    // until they lift.
    fn drop_contacts(&mut self) {
        self.lift_all();
        for s in 0..self.active.len() {
            self.ignored[s] = self.active[s];
        }
        self.dropping = true;
    }

    fn lift_all(&mut self) {
        let mut frame = vec![];
        for s in 0..self.active.len() {
            if self.active[s] && !self.ignored[s] {
                frame.push(abs_event(Abs::ABS_MT_SLOT, s as i32));
                frame.push(abs_event(Abs::ABS_MT_TRACKING_ID, -1));
            }
        }
        if let Some(tool) = self.tool.take() {
            frame.push(key_event(tool, 0));
            frame.push(key_event(Key::BTN_TOUCH, 0));
        }
        frame.push(abs_event(Abs::ABS_MT_SLOT, self.slot as i32));
        let _ = self.sink.emit(&frame);
        self.frame.clear();
    }

    // A touchpad says how many fingers are down (BTN_TOUCH, BTN_TOOL_*); the
    // touchscreen does not, so it is derived from the slots on every frame.
    fn finish_frame(&mut self) {
        let count = (0..self.active.len()).filter(|&s| self.active[s] && !self.ignored[s]).count();
        let want = (count > 0).then(|| TOOLS[count.min(TOOLS.len()) - 1]);
        if want == self.tool {
            return;
        }
        if let Some(old) = self.tool {
            self.frame.push(key_event(old, 0));
        }
        match want {
            Some(new) => {
                if self.tool.is_none() {
                    self.frame.push(key_event(Key::BTN_TOUCH, 1));
                }
                self.frame.push(key_event(new, 1));
            }
            None => self.frame.push(key_event(Key::BTN_TOUCH, 0)),
        }
        self.tool = want;
    }
}

fn abs_event(code: Abs, value: i32) -> InputEvent {
    InputEvent::new(EventType::ABSOLUTE.0, code.0, value)
}

fn key_event(code: Key, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY.0, code.0, value)
}
