# Printer protocol

Notes on the protocol spoken by the printers supported by the iPrint Android
app (`com.frogtosea.iprint`), as implemented in `crates/core` and `crates/bt`.
It belongs to the family often called "cat printers".

## Identification

Every printer advertises over Bluetooth LE. Its advertised name selects the
model: the catalogue entry whose `modelNo` is the longest prefix of the name
(`SC03h-2F95` → `SC03h`). The catalogue (`crates/core/models.json`, 141
models) holds everything that differs between models: transport, head width,
resolution, compression, energies, speeds, packet interval and label support.

## Transport

- **BLE GATT** (`useSPP: false`): write without response to the first
  characteristic found among `AE01`, `FF02`, `AB01`, `49535343-8841-…`,
  `AE81`; notifications on `AE02`, `AE04`, `AB03`, `FF03`, `AE82`. Packets are
  `imgMTU - 3` bytes when the model allows a larger MTU, 20 bytes otherwise.
- **Classic Bluetooth RFCOMM** (`useSPP: true`): serial port profile
  (`00001101-…`), channel 1, at the same address as the BLE advertisement.
  Packets are `imgMTU` bytes, 2–10 ms apart.

Both directions use the same framing.

## Frame

```
51 78 CMD 00 LEN_lo LEN_hi DATA… CRC8(DATA) FF
```

CRC-8 uses polynomial 0x07, initial value 0, over `DATA` only. "newFormat"
models prefix some frames with `12`.

## Host → printer

| Cmd | Data | Meaning |
|-----|------|---------|
| `A1` | u16 lines | Feed paper |
| `A1`/`A0` | u16 lines, `11` | Feed forward/back until a gap or black mark (max lines) |
| `A2` | width/8 bytes | Raw row, bit 0 = leftmost pixel (bit 7 on A4XII heads) |
| `A3` | `00` | Request status |
| `A4` | `32`…`35` | Print quality |
| `A6` | `05` | Stop |
| `A8` | `00` | Request device info |
| `AF` | u16 | Heating energy |
| `BB` | id | Device id / activation (app side only) |
| `BD` | u8 | Speed |
| `BE` | `00` image, `01` text, `02` tattoo, `03` photo; `00 01` gray | Print mode |
| `BF` | runs | Run-length row: each byte is `color << 7 \| length` (length ≤ 127) |
| `CE` | raw len u16, comp len u16, payload | Block of rows compressed with LZO1X (zlib on "CorePrint" models) |
| `CF` | same | Block of 4-bit gray rows, LZO1X |
| `D1` | 8 random bytes | Anti-clone challenge, verified by the app only |

Energy is `base × (1 + 0.15 × (density − 4))` for a density from 1 to 7,
where `base` is the model's text, image or gray energy.

## Printer → host

- `AE 01` with `10`: buffer full, pause writing; with `00`: resume.
- `A8`: device type, Wi‑Fi state and firmware version (ASCII, 7 bytes).
- `A3`: status bits (bit 0 no paper, 1 cover open, 2 overheating, 3 low
  battery, 4 charging, 7 printing), paper sensor reading, battery.

## Print job

Standard models:

```
A4 quality
for each page: AF energy, BE mode, BD speed, rows, BD speed every 200 rows
BD 25, A1 feed × paperNum, A8, A3
```

Rows are `BF` when the run-length form is shorter than the raw row and the
model allows it, `A2` otherwise. Models with an `lzoVersion` the firmware
meets send `CE` blocks of 18 rows (10 at 300 dpi) instead, each followed by
`BD`.

Gray mode: 16 blank rows, then 4-bit pixels (low nibble = left pixel,
15 = black) in `CF` blocks of 20 rows.

A4 heads (`size: 8`) pad rows on the left and finish each page with a
gap-detecting feed. "newFormat" models use ESC/POS column graphics
(`1B 2A 21`, 24 dots per band) instead of the frames above.

## Label paper

Models with `canPrintLabel` find the next label from the paper sensor: the
host sends 8 × (`A1` 1 line + `A3`), reads the sensor value of each status
reply, and looks for the edge between gap and label relative to a baseline
(`labelValue`, then learnt from stable readings). Once found it feeds
`backPaperNum` lines and prints. Models with `autoLabelCheck` do this in
firmware after `A1 … 11`.
