//! Status icons drawn pixel by pixel in pure black, so dithering leaves them sharp.
use crate::render::{Canvas, H, W};
use image::Luma;

pub const WIFI_W: i32 = 15;
pub const WIFI_H: i32 = 11;
pub const BATTERY_W: i32 = 20;
pub const BATTERY_H: i32 = 10;
pub const BOLT_W: i32 = 5;
const BOLT: [&str; 7] = ["...#.", "..##.", ".##..", "#####", "..##.", ".##..", ".#..."];

pub fn dot(c: &mut Canvas, x: i32, y: i32, value: u8) {
    if x >= 0 && y >= 0 && (x as u32) < W && (y as u32) < H {
        c.0.put_pixel(x as u32, y as u32, Luma([value]));
    }
}

pub fn fill_rect(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, value: u8) {
    for yy in y..y + h {
        for xx in x..x + w {
            dot(c, xx, yy, value);
        }
    }
}

/// Filled rectangle with the four corner pixels left untouched.
pub fn fill_rounded(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, value: u8) {
    for yy in y..y + h {
        let edge = yy == y || yy == y + h - 1;
        for xx in x..x + w {
            if edge && (xx == x || xx == x + w - 1) {
                continue;
            }
            dot(c, xx, yy, value);
        }
    }
}

fn outline(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, value: u8) {
    fill_rect(c, x, y, w, 1, value);
    fill_rect(c, x, y + h - 1, w, 1, value);
    fill_rect(c, x, y, 1, h, value);
    fill_rect(c, x + w - 1, y, 1, h, value);
}

/// 0..=3 arcs, from RSSI in dBm.
pub fn wifi_level(rssi_dbm: i8) -> u8 {
    match rssi_dbm {
        r if r >= -60 => 3,
        r if r >= -70 => 2,
        r if r >= -80 => 1,
        _ => 0,
    }
}

/// Wi-Fi fan: a dot plus three arcs. Arcs above `level` are dotted, like a faint grey.
pub fn wifi(c: &mut Canvas, x: i32, y: i32, level: u8) {
    const RINGS: [(f32, f32); 3] = [(3.4, 5.0), (6.4, 8.0), (9.4, 11.0)];
    for py in 0..WIFI_H {
        for px in 0..WIFI_W {
            let dx = px as f32 + 0.5 - WIFI_W as f32 / 2.0;
            let dy = WIFI_H as f32 - (py as f32 + 0.5);
            let r = (dx * dx + dy * dy).sqrt();
            let on = if r < 1.6 {
                true
            } else if dx.abs() > dy {
                false
            } else {
                match RINGS.iter().position(|&(a, b)| r >= a && r < b) {
                    Some(i) if (i as u8) < level => true,
                    Some(_) => (px + py) % 2 == 0,
                    None => false,
                }
            };
            if on {
                dot(c, x + px, y + py, 0);
            }
        }
    }
}

pub fn battery(c: &mut Canvas, x: i32, y: i32, percent: u8) {
    outline(c, x, y, BATTERY_W - 2, BATTERY_H, 0);
    fill_rect(c, x + BATTERY_W - 2, y + 3, 2, BATTERY_H - 6, 0);
    let inner = BATTERY_W - 6;
    let filled = ((inner as f32 * percent.min(100) as f32 / 100.0).ceil() as i32).min(inner);
    fill_rect(c, x + 2, y + 2, filled, BATTERY_H - 4, 0);
}

pub fn bolt(c: &mut Canvas, x: i32, y: i32) {
    for (j, row) in BOLT.iter().enumerate() {
        for (i, ch) in row.chars().enumerate() {
            if ch == '#' {
                dot(c, x + i as i32, y + j as i32, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rssi_maps_to_arcs() {
        assert_eq!(wifi_level(-44), 3);
        assert_eq!(wifi_level(-65), 2);
        assert_eq!(wifi_level(-78), 1);
        assert_eq!(wifi_level(-90), 0);
    }

    #[test]
    fn icons_are_pure_black_and_white() {
        let mut c = Canvas::new(255);
        wifi(&mut c, 2, 2, 2);
        battery(&mut c, 20, 2, 57);
        bolt(&mut c, 44, 2);
        assert!(c.0.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }
}
