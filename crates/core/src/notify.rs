//! Parsing of printer → host frames.

use crate::frame::cmd;
use crate::Model;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    /// Printer buffer is full: pause writing.
    FlowPause,
    /// Printer buffer drained: resume writing.
    FlowResume,
    DevInfo { dev_type: u8, firmware: String, wifi: u8 },
    Status(Status),
    /// Window credit for models with windowed flow control (YMS-BT01).
    Window { init: bool, credit: u8 },
    Other { cmd: u8, data: Vec<u8> },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub no_paper: bool,
    pub cover_open: bool,
    pub overheat: bool,
    pub low_battery: bool,
    pub charging: bool,
    pub printing: bool,
    /// Battery in percent when the model reports it, else `None`.
    pub battery_percent: Option<u8>,
    /// Battery level on the app's 0..=6 scale when reported that way.
    pub battery_level: Option<u8>,
    /// Paper sensor reading, used to find label gaps.
    pub sensor: Option<u8>,
}

impl Status {
    /// The app shows only the most significant problem (lowest set bit).
    pub fn problem(&self) -> Option<&'static str> {
        if self.no_paper {
            Some("no_paper")
        } else if self.cover_open {
            Some("cover_open")
        } else if self.overheat {
            Some("overheat")
        } else if self.low_battery {
            Some("low_battery")
        } else {
            None
        }
    }
}

/// Splits a byte stream into frames; keeps incomplete tails for the next call.
#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
}

impl Parser {
    pub fn push(&mut self, bytes: &[u8], model: &Model) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        // Two-byte window credits sent alone (YMS-BT01).
        if model.model_no == "YMS-BT01" && self.buf.len() == 2 && self.buf[0] != 0x51 {
            let (a, b) = (self.buf[0], self.buf[1]);
            self.buf.clear();
            if b > 0 && (a == 0 || a == 1) {
                out.push(Event::Window { init: a == 0, credit: b });
            }
            return out;
        }
        loop {
            let Some(start) = self.buf.windows(2).position(|w| w == [0x51, 0x78]) else {
                // Keep a trailing 0x51 that may start the next frame.
                let keep = self.buf.last() == Some(&0x51);
                self.buf.clear();
                if keep {
                    self.buf.push(0x51);
                }
                break;
            };
            self.buf.drain(..start);
            if self.buf.len() < 6 {
                break;
            }
            let len = u16::from_le_bytes([self.buf[4], self.buf[5]]) as usize;
            let total = len + 8;
            if self.buf.len() < total {
                break;
            }
            let f: Vec<u8> = self.buf.drain(..total).collect();
            out.push(decode(&f, model));
        }
        out
    }
}

fn decode(f: &[u8], model: &Model) -> Event {
    let c = f[2];
    let data = &f[6..f.len() - 2];
    match c {
        cmd::FLOW if data.first() == Some(&0x10) => Event::FlowPause,
        cmd::FLOW if data.first() == Some(&0x00) => Event::FlowResume,
        cmd::DEV_INFO if data.len() >= 3 => {
            let ver: Vec<u8> = data.iter().skip(3).take(7).copied().take_while(|&b| b != 0).collect();
            Event::DevInfo {
                dev_type: data[0],
                firmware: String::from_utf8_lossy(&ver).trim().to_string(),
                wifi: data[2],
            }
        }
        cmd::STATUS if !data.is_empty() => Event::Status(status(data, model)),
        _ => Event::Other { cmd: c, data: data.to_vec() },
    }
}

fn status(d: &[u8], model: &Model) -> Status {
    let b = d[0];
    let mut s = Status {
        no_paper: b & 0x01 != 0,
        cover_open: b & 0x02 != 0,
        overheat: b & 0x04 != 0,
        low_battery: b & 0x08 != 0,
        charging: b & 0x10 != 0,
        printing: b & 0x80 != 0,
        sensor: if d.len() >= 3 { d.get(1).copied() } else { None },
        ..Default::default()
    };
    if model.a4xii {
        s.battery_percent = d.get(2).copied();
    } else if model.show_electricity_model == 2 {
        s.battery_percent = if model.model_no.starts_with("PR89") || model.model_no.starts_with("ZPA4Z1") {
            d.get(1).copied()
        } else {
            d.get(2).map(|v| v.saturating_add(1))
        };
    } else if model.show_electricity_model == 1 {
        // The app reads the byte's hex digits as a decimal number.
        if let Some(v) = d.get(2) {
            let n: u32 = format!("{v:x}").parse().unwrap_or(0);
            s.battery_level = Some(match n {
                28.. => 6,
                27 => 5,
                26 => 4,
                25 => 3,
                24 => 2,
                23 => 1,
                _ => 0,
            });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_sc03h_replies() {
        let m = crate::lookup("SC03h-2F95").unwrap();
        let mut p = Parser::default();
        let bytes = [
            0x51, 0x78, 0xa8, 0x01, 0x0a, 0x00, 0x81, 0x00, 0x03, 0x33, 0x2e, 0x30, 0x2e, 0x34, 0x39, 0x00, 0x15,
            0xff, 0x51, 0x78, 0xa3, 0x01, 0x03, 0x00, 0x00, 0x02, 0x26, 0xd8, 0xff,
        ];
        // Split across two pushes to exercise buffering.
        let mut ev = p.push(&bytes[..10], m);
        ev.extend(p.push(&bytes[10..], m));
        assert_eq!(
            ev[0],
            Event::DevInfo { dev_type: 0x81, firmware: "3.0.49".into(), wifi: 3 }
        );
        let Event::Status(s) = &ev[1] else { panic!() };
        assert_eq!(s.problem(), None);
        assert_eq!(s.battery_level, Some(4));
    }

    #[test]
    fn flow_control() {
        let m = crate::lookup("SC03h-2F95").unwrap();
        let mut p = Parser::default();
        let ev = p.push(&[0x51, 0x78, 0xAE, 0x01, 0x01, 0x00, 0x10, 0x70, 0xFF], m);
        assert_eq!(ev, vec![Event::FlowPause]);
        let ev = p.push(&[0x51, 0x78, 0xAE, 0x01, 0x01, 0x00, 0x00, 0x00, 0xFF], m);
        assert_eq!(ev, vec![Event::FlowResume]);
    }
}
