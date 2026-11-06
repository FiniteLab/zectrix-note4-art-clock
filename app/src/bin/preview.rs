//! Hold one device session open and flip through PNG mockups with the device keys.
//!
//! The device leaves remote mode as soon as the session closes, so the image only
//! stays on screen while this process is running.
//!
//! Usage: preview <device IP> <png>... [--minutes N]
//! Keys: Down = next, Up = previous, hold Up+Down on the device to exit.
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::signal::unix::{signal, SignalKind};
use zectrix_sdk::{Device, Error, Event, ImageFormat, Key, KeyAction, OpenOptions};

struct Slide {
    name: String,
    png: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut minutes = 30u64;
    if let Some(i) = args.iter().position(|a| a == "--minutes") {
        minutes = args.get(i + 1).ok_or("--minutes 后面需要分钟数")?.parse()?;
        args.drain(i..=i + 1);
    }
    if args.len() < 2 {
        return Err("用法：preview <设备 IP> <PNG>... [--minutes N]".into());
    }
    let target = args.remove(0);
    let slides = args
        .iter()
        .map(|p| {
            let name = Path::new(p)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone());
            std::fs::read(p).map(|png| Slide { name, png })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let device = open_with_retry(&target).await?;
    println!("已连接：{} ({})", device.summary.name, device.summary.address);
    let mut idx = 0usize;
    show(&device, &slides, idx).await;

    let mut term = signal(SignalKind::terminate())?;
    let limit = tokio::time::sleep(Duration::from_secs(minutes * 60));
    tokio::pin!(limit);
    loop {
        tokio::select! {
            event = device.next_event() => match event {
                Ok(Some(Event::Key { key, action })) => {
                    println!("按键：{key:?} {action:?}");
                    let next = match (key, action) {
                        (Key::Down, KeyAction::Short) => Some((idx + 1) % slides.len()),
                        (Key::Up, KeyAction::Short) => Some((idx + slides.len() - 1) % slides.len()),
                        _ => None,
                    };
                    if let Some(n) = next {
                        idx = n;
                        show(&device, &slides, idx).await;
                    }
                }
                Ok(Some(Event::Disconnected { reason })) => {
                    println!("连接结束：{reason:?}");
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    eprintln!("事件错误：{e}");
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => { println!("收到 Ctrl-C，退出"); break; }
            _ = term.recv() => { println!("收到退出信号，退出"); break; }
            _ = &mut limit => { println!("已运行 {minutes} 分钟，自动退出"); break; }
        }
    }
    device.close().await?;
    println!("已断开");
    Ok(())
}

async fn open_with_retry(target: &str) -> Result<Device, Error> {
    let mut attempt = 1;
    loop {
        let opened = Device::open(OpenOptions {
            target: Some(target.into()),
            client_name: Some("Art Clock Preview".into()),
        })
        .await;
        match opened {
            Ok(device) => return Ok(device),
            // Wi-Fi power save can swallow the first INFO probe, and a session that
            // just closed keeps the device busy for a moment.
            Err(e @ (Error::NoDevice | Error::Busy)) if attempt < 6 => {
                eprintln!("第 {attempt} 次连接失败：{e}，3 秒后重试");
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn show(device: &Device, slides: &[Slide], idx: usize) {
    let slide = &slides[idx];
    let started = Instant::now();
    match device.show_with_format(&slide.png, ImageFormat::Gray16).await {
        Ok(()) => println!(
            "[{}/{}] {} 显示完成，用时 {:.2}s",
            idx + 1,
            slides.len(),
            slide.name,
            started.elapsed().as_secs_f64()
        ),
        Err(e) => println!("[{}/{}] {} 显示失败：{e}", idx + 1, slides.len(), slide.name),
    }
}
