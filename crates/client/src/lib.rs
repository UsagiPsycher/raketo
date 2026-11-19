//! The `raketo` command. It prints through the Raketo app running on this
//! computer (or another one with `--url`), so scripts never deal with
//! Bluetooth or the printer protocol.

use std::io::Read;
use std::time::Duration;

pub const USAGE: &str = "\
Print through a running Raketo app.

Usage:
  raketo print <file | ->     Print an image (PNG, JPEG, GIF, WebP, BMP)
  raketo text <text | ->      Print text
  raketo status               Show the connected printer
  raketo help                 Show this help

Print options:
  --mode text|image|photo     Rendering (default: image for files, text for text)
  --gray                      16 gray levels, on models that support it
  --density <1-7>             Darkness (default 4)
  --copies <n>                Number of copies
  --label                     Label paper: one copy per label
  --label-height <mm>         Label height, to bound the gap search
  --trim                      Remove blank margins around the image
  --no-wait                   Return once the job is queued

Text options:
  --size <px>                 Font size in printer dots (default 28)
  --bold                      Bold text
  --center, --right           Alignment

Other:
  --url <url>                 Raketo to use (default: this computer; env RAKETO_URL)
  --json                      Print the server reply as JSON

Exit codes: 0 printed, 1 usage error, 2 Raketo not running, 3 no printer
connected, 4 printer problem (paper, cover, heat), 5 document or other error.
";

/// Commands the app binary hands over to this module instead of opening a window.
pub fn is_command(first: &str) -> bool {
    matches!(first, "print" | "text" | "status" | "help" | "--help" | "-h" | "--version" | "-V")
}

struct Opts {
    url: Option<String>,
    json: bool,
    query: Vec<(String, String)>,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { url: None, json: false, query: Vec::new(), positional: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--url" => o.url = Some(value("--url")?),
            "--json" => o.json = true,
            "--gray" => o.query.push(("gray".into(), "1".into())),
            "--label" => o.query.push(("label".into(), "1".into())),
            "--trim" => o.query.push(("trim".into(), "1".into())),
            "--no-wait" => o.query.push(("wait".into(), "0".into())),
            "--bold" => o.query.push(("bold".into(), "1".into())),
            "--center" => o.query.push(("align".into(), "center".into())),
            "--right" => o.query.push(("align".into(), "right".into())),
            "--mode" => {
                let v = value("--mode")?;
                if !["text", "image", "photo"].contains(&v.as_str()) {
                    return Err("--mode is text, image or photo".into());
                }
                o.query.push(("content".into(), v));
            }
            "--density" => {
                let v = value("--density")?;
                match v.parse::<u8>() {
                    Ok(1..=7) => o.query.push(("density".into(), v)),
                    _ => return Err("--density is between 1 and 7".into()),
                }
            }
            "--copies" | "--size" | "--label-height" => {
                let v = value(a)?;
                if v.parse::<f32>().map(|n| n <= 0.0).unwrap_or(true) {
                    return Err(format!("{a} needs a positive number"));
                }
                let key = match a.as_str() {
                    "--copies" => "copies",
                    "--size" => "size",
                    _ => "label_height",
                };
                o.query.push((key.into(), v));
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => o.positional.push(a.clone()),
        }
    }
    Ok(o)
}

/// API address: `--url`, then RAKETO_URL, then the file the app writes at start.
fn base_url(o: &Opts) -> String {
    if let Some(u) = o.url.clone().or_else(|| std::env::var("RAKETO_URL").ok()) {
        let u = u.trim_end_matches('/').to_string();
        return if u.contains("://") { u } else { format!("http://{u}") };
    }
    let from_file = dirs::config_dir()
        .map(|d| d.join("app.raketo.desktop").join("api.json"))
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("port").and_then(|p| p.as_u64()));
    format!("http://127.0.0.1:{}", from_file.unwrap_or(8631))
}

fn read_input(arg: &str) -> Result<Vec<u8>, String> {
    if arg == "-" {
        let mut b = Vec::new();
        std::io::stdin().read_to_end(&mut b).map_err(|e| format!("cannot read standard input: {e}"))?;
        Ok(b)
    } else {
        std::fs::read(arg).map_err(|e| format!("cannot read {arg}: {e}"))
    }
}

fn agent() -> ureq::Agent {
    // Printing waits for the paper to come out; long jobs take a while.
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(3)).timeout_read(Duration::from_secs(600)).build()
}

fn explain(code: &str) -> &str {
    match code {
        "not_connected" => "no printer connected in Raketo",
        "no_paper" => "the printer is out of paper",
        "cover_open" => "the printer cover is open",
        "overheat" => "the print head is too hot, try again in a moment",
        "no_gray" => "this printer does not print grayscale",
        "label_not_found" => "no label edge found, check that label paper is loaded",
        "no_sensor" => "the printer does not report its paper sensor",
        "unsupported_format" => "unsupported file format",
        "bad_document" => "the document could not be read",
        _ => "printing failed",
    }
}

fn exit_code(code: &str) -> i32 {
    match code {
        "not_connected" => 3,
        "no_paper" | "cover_open" | "overheat" | "label_not_found" | "no_sensor" => 4,
        _ => 5,
    }
}

/// Sends a request and turns the reply into an exit code and a message.
fn send(req: ureq::Request, body: &[u8], json: bool) -> i32 {
    let reply = match req.send_bytes(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            eprintln!("raketo: Raketo is not running (start the app, or use --url)");
            return 2;
        }
    };
    let text = reply.into_string().unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    if json {
        println!("{text}");
    }
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        return 0;
    }
    let code = v.get("error").and_then(|c| c.as_str()).unwrap_or("print_failed");
    if !json {
        let detail = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
        let msg = explain(code);
        if msg == "printing failed" && !detail.is_empty() {
            eprintln!("raketo: {detail}");
        } else {
            eprintln!("raketo: {msg}");
        }
    }
    exit_code(code)
}

fn with_query(url: String, q: &[(String, String)]) -> String {
    if q.is_empty() {
        return url;
    }
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    let qs: Vec<String> = q.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect();
    format!("{url}?{}", qs.join("&"))
}

fn status(o: &Opts) -> i32 {
    let url = format!("{}/status", base_url(o));
    let reply = match agent().get(&url).call() {
        Ok(r) => r,
        Err(_) => {
            eprintln!("raketo: Raketo is not running (start the app, or use --url)");
            return 2;
        }
    };
    let text = reply.into_string().unwrap_or_default();
    if o.json {
        println!("{text}");
    }
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if v.get("connected").and_then(|b| b.as_bool()) != Some(true) {
        if !o.json {
            println!("Raketo is running, no printer connected.");
        }
        return 3;
    }
    if !o.json {
        let p = &v["printer"]["printer"];
        let st = &v["printer"]["status"];
        let battery = st["batteryPercent"]
            .as_u64()
            .or_else(|| st["batteryLevel"].as_u64().map(|l| l * 100 / 6))
            .map(|b| format!(", battery {b} %"))
            .unwrap_or_default();
        let state = match v["problem"].as_str() {
            Some(c) => explain(c).to_string(),
            None if v["busy"].as_bool() == Some(true) => "printing".into(),
            None => "ready".into(),
        };
        println!(
            "{} ({}, {} mm){battery} – {state}",
            p["name"].as_str().unwrap_or("?"),
            p["modelNo"].as_str().unwrap_or("?"),
            p["paperMm"].as_u64().unwrap_or(0)
        );
    }
    match v["problem"].as_str() {
        Some("low_battery") | None => 0,
        Some(_) => 4,
    }
}

/// Runs the command line and returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let Some(cmd) = args.first() else {
        print!("{USAGE}");
        return 1;
    };
    if matches!(cmd.as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return 0;
    }
    if matches!(cmd.as_str(), "--version" | "-V") {
        println!("raketo {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let o = match parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("raketo: {e}\nRun `raketo help` for usage.");
            return 1;
        }
    };
    match cmd.as_str() {
        "status" => status(&o),
        "print" | "text" => {
            let Some(src) = o.positional.first() else {
                eprintln!("raketo: {cmd} needs {}", if cmd == "print" { "a file, or - for standard input" } else { "some text, or - for standard input" });
                return 1;
            };
            let body = if cmd == "text" && src != "-" {
                o.positional.join(" ").into_bytes()
            } else {
                match read_input(src) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("raketo: {e}");
                        return 5;
                    }
                }
            };
            let path = if cmd == "print" { "/print" } else { "/print/text" };
            let url = with_query(format!("{}{path}", base_url(&o)), &o.query);
            let ctype = if cmd == "text" { "text/plain; charset=utf-8" } else { "application/octet-stream" };
            send(agent().post(&url).set("content-type", ctype), &body, o.json)
        }
        other => {
            eprintln!("raketo: unknown command {other}\nRun `raketo help` for usage.");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_options() {
        let o = parse(&s(&["photo.png", "--gray", "--density", "6", "--copies", "2"])).unwrap();
        assert_eq!(o.positional, vec!["photo.png"]);
        assert!(o.query.contains(&("density".into(), "6".into())));
        assert!(parse(&s(&["--density", "9"])).is_err());
        assert!(parse(&s(&["--nope"])).is_err());
    }

    #[test]
    fn query_is_encoded() {
        let u = with_query("http://h/print/text".into(), &[("align".into(), "center".into()), ("x".into(), "a b".into())]);
        assert_eq!(u, "http://h/print/text?align=center&x=a%20b");
    }

    #[test]
    fn not_running_is_exit_2() {
        let code = run(&s(&["status", "--url", "127.0.0.1:1"]));
        assert_eq!(code, 2);
    }
}
