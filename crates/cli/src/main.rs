//! Test CLI: `raketo scan`, `raketo status [NAME]`, `raketo print FILE [--gray] [--text] [--density N] [--dry OUT.png]`.

use anyhow::{bail, Context, Result};
use raketo_core::{Content, JobOptions};
use std::time::Duration;

async fn pick(name: Option<&str>) -> Result<raketo_bt::Found> {
    let found = raketo_bt::scan(Duration::from_secs(6)).await?;
    found
        .into_iter()
        .find(|f| name.is_none_or(|n| f.name.starts_with(n)))
        .context("no printer found")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("scan") => {
            for f in raketo_bt::scan(Duration::from_secs(8)).await? {
                let m = f.model;
                println!(
                    "{:<20} {} rssi={:?} model={} {} {}mm {}px",
                    f.name,
                    f.address_string(),
                    f.rssi,
                    m.model_no,
                    if m.use_spp { "spp" } else { "ble" },
                    m.paper_mm(),
                    m.image_width()
                );
            }
        }
        Some("status") => {
            let f = pick(args.get(1).map(String::as_str)).await?;
            let p = raketo_bt::connect(&f).await?;
            println!("{:#?}", p.handshake().await?);
            p.disconnect().await;
        }
        Some("print") => {
            let Some(path) = args.get(1) else { bail!("missing file") };
            let img = image::open(path)?;
            let mut opts = JobOptions {
                gray: flag("--gray"),
                content: if flag("--text") { Content::Text } else if flag("--photo") { Content::Photo } else { Content::Image },
                ..Default::default()
            };
            if let Some(d) = value("--density") {
                opts.density = d.parse()?;
            }
            if let Some(c) = value("--curve") {
                let v: Vec<f32> = c.split(',').filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    opts.gray_curve = Some(raketo_core::encode::GrayCurve { lo: v[0] as u8, hi: v[1] as u8, gamma: v[2] });
                }
            }
            let model_name = value("--model");
            if let Some(out) = value("--dry") {
                let m = raketo_core::lookup(model_name.as_deref().unwrap_or("SC03h")).context("unknown model")?;
                let r = raketo_core::rasterize(m, &[img], &opts);
                r[0].preview().save(&out)?;
                let job = raketo_core::build_job(m, &r, &opts);
                println!("{} bytes for {}", job.len(), m.model_no);
                return Ok(());
            }
            let f = pick(value("--name").as_deref()).await?;
            let p = raketo_bt::connect(&f).await?;
            let info = p.handshake().await?;
            println!("{} fw={:?} status={:?}", info.model_no, info.firmware, info.status);
            opts.firmware = p.firmware();
            let r = raketo_core::rasterize(p.model(), &[img], &opts);
            let job = raketo_core::build_job(p.model(), &r, &opts);
            println!("sending {} bytes", job.len());
            p.send(&job, |s, t| if s == t || s % 4096 < 200 { eprint!("\r{:>3}%", s * 100 / t) }).await?;
            eprintln!();
            tokio::time::sleep(Duration::from_secs(2)).await;
            p.disconnect().await;
        }
        Some("wedge") => {
            // 16 raw gray steps (no tone curve, no dithering) per requested energy.
            let energies: Vec<u32> = args[1..].iter().filter_map(|a| a.parse().ok()).collect();
            let f = pick(value("--name").as_deref()).await?;
            let p = raketo_bt::connect(&f).await?;
            let info = p.handshake().await?;
            println!("{} fw={:?}", info.model_no, info.firmware);
            let w = p.model().image_width();
            let font = |e: u32| -> Vec<u8> { e.to_string().into_bytes() };
            let mut pages = Vec::new();
            for &e in &energies {
                let h = 40u32;
                let mut levels = vec![0u8; (w * h) as usize];
                for y in 0..h {
                    for x in 0..w {
                        levels[(y * w + x) as usize] = (x * 16 / w) as u8;
                    }
                }
                // Tick marks every step on the first rows to count steps.
                for x in (0..w).step_by((w / 16) as usize) {
                    for y in 0..4 {
                        levels[(y * w + x) as usize] = 15;
                    }
                }
                let _ = font(e);
                pages.push((e, raketo_core::raster::GrayBitmap { width: w, height: h, levels }));
            }
            for (e, g) in pages {
                println!("energy {e}");
                let opts = JobOptions { gray: true, energy: Some(e), firmware: p.firmware(), ..Default::default() };
                let job = raketo_core::build_job(p.model(), &[raketo_core::Raster::Gray(g)], &opts);
                p.send(&job, |_, _| {}).await?;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            p.disconnect().await;
        }
        _ => eprintln!("usage: raketo scan | status [NAME] | print FILE [--gray|--text|--photo] [--density 1-7] [--dry OUT.png --model M]"),
    }
    Ok(())
}
