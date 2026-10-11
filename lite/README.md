# lolPing Lite

English | [简体中文](README.zh-CN.md)

A tiny cut-down lolPing, a few hundred KB: press **Alt + ?** and League's "?" (Enemy Missing) ping pops up somewhere on
the screen of everyone on the LAN, with the original sound. No settings window, just a tray icon.

## Use

1. Run `lolping-lite.exe`. No window opens: its icon appears in the tray. The first time, Windows Firewall asks about
   network access: allow it on private networks.
2. Press **Alt + ?**: Alt and the `/?` key left of right Shift. It goes by the key's position, so any keyboard layout
   works; the keypad's `/` doesn't count. The key still reaches the app you're in: lolPing Lite only listens.
3. Everyone on the LAN running lolPing Lite sees it.

- Limits: one every 100 ms at most, and five in any 5 seconds. Holding the keys down is no faster.
- Each "?" plays the original animation for the original 3.2 s.
- Where it lands is picked on each receiving machine, on the display under its mouse. Spots fill an ellipse the shape of the display: densest on a ring, about 30 % of that in the middle, thinning out to almost nothing towards the edges, and never within 5 % of an edge.
- A burst from one person (each ping less than 3.2 s after their last) gathers around the burst's first ping.

## Tray menu

Right- or left-click the tray icon:

- **Master · N online**, or **N online**: information only.
- **Stop receiving**: hides everyone else's "?" and greys the icon. As the master, this machine still passes pings on.
- **Show my own "?"**: off by default, so your own pings don't appear on your own screen.
- **Start with Windows**
- **Quit**

The switches are kept in the registry, under `HKCU\Software\lolPing Lite`.

## Network

Everything uses UDP port **47475**. No multicast or IGMP is needed.

- Every machine broadcasts a hello each second, to `255.255.255.255` and to each network interface's own broadcast address.
- **Master lottery**: each node's ticket is a hash of its id and a seed every copy shares, and the lowest ticket is the master. Each machine works this out on its own and they all agree, with nothing to negotiate.
- **Relay**:
  - A sender unicasts its "?" to the master.
  - The master acks it, unicasts it to every member it knows, and broadcasts it once.
  - With no ack within 300 ms (say the master just went offline), the sender broadcasts it itself.
- **Member lists**: each hello lists the members its sender hears directly. So machines whose broadcasts don't reach each other are still connected through the master, and the head count stays right.
- **Master failure**: about 3.5 s after the master goes offline, the others elect the next one. Meanwhile the sender's own broadcast keeps pings flowing, so none are lost.
- **Duplicates and floods**:
  - Each ping carries its origin and a random id, so it shows once whichever way it arrives.
  - Receivers rate-limit each origin too, so a modified client can't flood anyone.
- Packets carry a tag keyed with the shared seed, and anything else on the port is dropped. That is not encryption: the only content is a "?".
- Lite doesn't talk to full lolPing.

## Known limitations

- While an app running as administrator has focus, Windows withholds its keys from ordinary apps, so Alt + ? does nothing.
- Exclusive-fullscreen games cover topmost windows: play in borderless mode.
- Windows 10 or later only.

## Build

With stable Rust:

```sh
cd lite
cargo test                 # network, limits, landing spots, animation: runs on any OS
cargo build --release      # no console: target/release/lolping-lite.exe
cargo build                # debug: keeps a console that shows the log
```

- Release builds log at info level and above to `%LOCALAPPDATA%\lolPing Lite\lolping-lite.log`.
- Debug builds take `--auto-send <ms>`, which presses Alt + ? on a timer, for testing several machines without a keyboard.
- `cargo run --release --example preview -- frames out.png` renders the animation frame by frame, and `... -- spots out.png` renders where pings land.
