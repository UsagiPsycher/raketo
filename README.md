# Raketo

Raketo prints on the pocket thermal printers sold for the **iPrint** phone
app, from a computer. It connects over Bluetooth and covers what the app's
Print tab does: photos, text with images, documents, labels, sticky notes,
to-do lists and long banners.

While a printer is connected, Raketo also shares it on the local network.
Other devices can print from a web page, or add it as a regular printer
without installing a driver.

<p align="center"><img src="src-tauri/icons/icon.png" width="96" alt=""></p>

## Download

Get the latest build from [Releases](https://github.com/UsagiPsycher/raketo/releases).

| System | File |
|--------|------|
| Windows 10/11 | `Raketo_x.y.z_x64-setup.exe` (installer), or `Raketo_x.y.z_x64_portable.zip` (no install) |
| macOS 11+ | `Raketo_x.y.z_aarch64.dmg` (Apple Silicon), `Raketo_x.y.z_x64.dmg` (Intel) |
| Linux | `.AppImage` (any distribution), `.deb` (Debian, Ubuntu), `.rpm` (Fedora, openSUSE) |

The builds are not code-signed yet:

- **Windows**: if SmartScreen stops the file, choose *More info → Run anyway*.
- **macOS**: right-click the app, choose *Open*, then confirm. Or run
  `xattr -cr /Applications/Raketo.app`.
- **Linux**: make the AppImage executable (`chmod +x Raketo*.AppImage`). Raketo
  uses BlueZ, which every desktop distribution ships.

## Printers

Any printer the iPrint app supports should work. Raketo carries the same
catalogue of 141 models, read from the app. The printer's Bluetooth name
selects the model, and the model decides everything else: Bluetooth LE or
classic Bluetooth, head width (58 mm, 80 mm, 110 mm, A4…), resolution,
compression, heat and speed.

Tested so far: **SC03h** (58 mm, classic Bluetooth) on Windows. If you have
another model, reports are welcome, even when it works.

## Using it

1. Turn the printer on and click **Connect**. Raketo searches for printers and
   pairs with yours if needed. You don't have to pair it in the system
   settings first.
2. Pick a tool on the left and build your print.
3. The paper strip on the right shows the exact dots the printer will print,
   and the print button tells how much paper it will use.
4. Choose the rendering (text, drawing or photo), the density, the number of
   copies, and grayscale when the model supports it, then click **Print**.

**Grayscale calibration.** On printers that print in gray, the slider button
next to the battery prints a 16-step scale. Enter the first gray step and the
first black step, and Raketo spreads gray levels over the range that printer
can actually render. The setting is kept per printer.

**Start with the system.** In *Settings* (bottom of the left bar), turn on
*Start with the system*. Raketo then opens minimized when you sign in and
reconnects to the last printer, so sharing and the print API are ready
without touching anything. It uses the usual mechanism of each system: the
Run key on Windows, a login item (LaunchAgent) on macOS, and an XDG autostart
entry on Linux.

**Label paper.** In the Label tool, turn on *Label paper*. Before each copy,
Raketo finds the edge of the next label with the printer's paper sensor, so
each print lands on its own label.

## Sharing on the network

Sharing starts on its own when a printer is connected, and stops when it
disconnects. You can turn it off in the printer panel.

- **Web page.** Open `http://raketo.local` from another computer or phone on
  the same network to use the same tools and print. Android does not resolve
  `.local` names: use the IP address shown in Raketo, or scan its QR code.
- **Driverless printer.** Raketo announces an IPP Everywhere printer over
  Bonjour/mDNS, which systems with driverless printing can add: Windows
  (*Settings → Bluetooth & devices → Printers → Add device*), macOS
  (*System Settings → Printers & Scanners*), Linux with CUPS, and Android
  (*Default Print Service*). Pages are scaled to the roll width and blank
  margins are trimmed to save paper.

Raketo listens on port 80 when it is free, 8631 otherwise. On Windows it adds
its own firewall rule, so no prompt shows up. The installer does this during
setup; the portable version asks for administrator rights once, the first time
sharing starts. If two computers share a printer on the same network, the
second one is `raketo-2.local`.

There is no authentication: anyone on the local network can print.

## Printing from scripts and other apps

While Raketo is open, any program on the computer can print through it. The
API listens on `127.0.0.1` at all times, and on the network too while sharing
is on.

### Command line

```sh
raketo print photo.jpg --gray
raketo print receipt.png --mode text --copies 2
raketo text "Order 42: two coffees" --size 40 --bold --center
some-command | raketo text -
raketo status
```

Options: `--mode text|image|photo`, `--gray`, `--density 1-7`, `--copies n`,
`--label`, `--label-height mm`, `--trim`, `--no-wait`; for text, `--size px`,
`--bold`, `--center`, `--right`. `--url` (or `RAKETO_URL`) targets Raketo on
another computer, `--json` prints the raw reply. Run `raketo help` for the
full list.

Exit codes: `0` printed, `1` usage error, `2` Raketo not running, `3` no
printer connected, `4` printer problem (paper, cover, heat), `5` document or
other error.

Where the command lives:

- **Windows**: the installer puts `raketo` on `PATH`. In the portable zip,
  run `raketo` from the extracted folder.
- **Linux**: the `.deb` and `.rpm` install `/usr/bin/raketo`; with the
  AppImage, run `./Raketo_x.y.z_amd64.AppImage print photo.jpg`.
- **macOS**: link it once with
  `ln -s /Applications/Raketo.app/Contents/MacOS/raketo /usr/local/bin/raketo`.

### HTTP

The command is a thin wrapper around three endpoints, usable from any
language:

```sh
curl --data-binary @photo.jpg "http://127.0.0.1:8631/print?gray=1"
curl --data "Hello" "http://127.0.0.1:8631/print/text?size=40&bold=1&align=center"
curl http://127.0.0.1:8631/status
```

- `POST /print`: the body is an image (PNG, JPEG, GIF, WebP, BMP, PWG raster
  or URF).
- `POST /print/text`: the body is UTF-8 text, laid out by Raketo.
- Query parameters: `mode`, `gray`, `density`, `copies`, `label`,
  `label_height`, `trim`, `wait=0`; for text, `size`, `bold`, `align`.

Replies are JSON: `{"ok":true,"job":3}` once the print is done (or queued with
`wait=0`), otherwise `{"ok":false,"error":"no_paper","message":"…"}` with an
HTTP error status. Error codes: `not_connected`, `no_paper`, `cover_open`,
`overheat`, `label_not_found`, `unsupported_format`, `bad_document`,
`print_failed`.

The port is 8631, or 80 while sharing when 80 is free. The app writes the
current one to `api.json` in its configuration folder
(`%APPDATA%\app.raketo.desktop` on Windows,
`~/Library/Application Support/app.raketo.desktop` on macOS,
`~/.config/app.raketo.desktop` on Linux). The `raketo` command reads that
file. PDF files are not rendered by the API yet; print them to the Raketo
printer instead.

## Building

Requirements: [Rust](https://rustup.rs) (stable) and Node.js 20.

```sh
npm ci
npx tauri build
```

On Linux, install the WebKitGTK and D-Bus development packages first. On
Debian or Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libdbus-1-dev pkg-config
```

On Windows, `build.bat` builds the installer, the portable zip and the
`raketo` command.

For development, `npx tauri dev` runs the app with hot reload. On Windows,
build the command once first (`build.bat` does it), since the app bundles it.
`cargo run -p raketo-cli -- scan` drives a printer directly, without the app,
for protocol tests.

## Layout

```
crates/core      model catalogue, frames, raster encoders, printer replies
crates/bt        Bluetooth LE (btleplug) and RFCOMM (Windows, Linux, macOS), pairing
crates/service   connection, print queue, per-printer settings
crates/server    web page, JSON API, IPP Everywhere endpoint, mDNS
crates/client    the `raketo` command (talks to the running app)
crates/cmd       console build of that command (raketo.com on Windows)
crates/cli       low-level test tool that drives a printer directly
src-tauri        desktop app
src              interface (React), shared by the app and the web page
docs/protocol.md the printer protocol
```

## Known limitations

- Classic Bluetooth on macOS and Linux, and label paper, are implemented but
  not yet tested on hardware.
- The IPP printer has been checked with an IPP client, not yet from each
  operating system's print dialog.
- The AI features of the iPrint app (cartoon, line art, text to image…) are
  not included: they run on the vendor's servers.
- Wi-Fi setup for Wi-Fi capable models is not included.

## License

MIT. Raketo is an independent project. It is not affiliated with the makers
of the iPrint app or of the printers.
