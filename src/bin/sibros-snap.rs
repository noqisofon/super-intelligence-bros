//! ステージの絵を CPU で PNG に書き出す（GPU もウィンドウも不要）。
//!
//! 使い方:
//!   sibros-snap OUT.png [--level FILE] [--run SPEC] [--scale N]
//!
//! - `--level FILE`: ステージのテキストファイル。省略すると初期ステージ。
//! - `--run SPEC`  : 描く前にゲームを進める。例 `right:60,right+jump:20,wait:10`
//!   （入力は left / right / jump を `+` でつなぐ。何も押さないなら wait）
//! - `--scale N`   : 拡大率（既定 4）
//!
//! 描画版 `sibros-view` と同じ割り当て（`visual` モジュール）を使うので、
//! この PNG の見た目が、描画版の見た目の確認になる。

use std::io::Cursor;

use super_intelligence_bros::visual::{self, Atlas, Draw};
use super_intelligence_bros::{DEFAULT_LEVEL, Game, Level, PadInput};

const TILES_PNG: &[u8] = include_bytes!("../../assets/tiles_1985.png");
const SPRITES_PNG: &[u8] = include_bytes!("../../assets/sprites.png");

struct Image {
    w: usize,
    #[allow(dead_code)]
    h: usize,
    rgba: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Result<Image, String> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("画像が大きすぎます")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..info.buffer_size()];
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => return Err(format!("未対応の色形式です: {other:?}")),
    };
    Ok(Image { w, h, rgba })
}

fn parse_run(spec: &str) -> Result<Vec<(PadInput, u32)>, String> {
    let mut out = Vec::new();
    for part in spec.split(',').filter(|s| !s.is_empty()) {
        let (keys, frames) = part
            .split_once(':')
            .ok_or(format!("`{part}` は 入力:フレーム の形で書いてください"))?;
        let frames: u32 = frames
            .parse()
            .map_err(|_| format!("フレーム数が数字ではありません: {frames}"))?;
        let mut input = PadInput::default();
        for k in keys.split('+') {
            match k {
                "left" => input.left = true,
                "right" => input.right = true,
                "jump" => input.jump = true,
                "wait" => {}
                other => {
                    return Err(format!(
                        "知らない入力です: {other}（left / right / jump / wait）"
                    ));
                }
            }
        }
        out.push((input, frames));
    }
    Ok(out)
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut out_path: Option<String> = None;
    let mut level_path: Option<String> = None;
    let mut run = String::new();
    let mut scale: usize = 4;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--level" => level_path = Some(args.next().ok_or("--level にファイル名が必要です")?),
            "--run" => run = args.next().ok_or("--run に指定が必要です")?,
            "--scale" => {
                scale = args
                    .next()
                    .ok_or("--scale に数字が必要です")?
                    .parse()
                    .map_err(|_| "--scale は数字です")?;
            }
            _ if out_path.is_none() => out_path = Some(a),
            _ => return Err(format!("余計な引数です: {a}")),
        }
    }
    let out_path =
        out_path.ok_or("使い方: sibros-snap OUT.png [--level FILE] [--run SPEC] [--scale N]")?;
    let src = match level_path {
        Some(p) => std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?,
        None => DEFAULT_LEVEL.to_string(),
    };

    let mut game = Game::try_with_level(&src)?;
    for (input, frames) in parse_run(&run)? {
        game.step(input, frames);
    }
    let snap = game.snapshot();
    let level = Level::parse(&src);

    let tiles = decode(TILES_PNG)?;
    let sprites = decode(SPRITES_PNG)?;
    let (cell, tw) = (visual::TILE_PX as usize, level.width() as usize);
    let (w, h) = (tw * cell * scale, level.height() as usize * cell * scale);
    let mut canvas = vec![0u8; w * h * 4];
    for px in canvas.chunks_mut(4) {
        px.copy_from_slice(&[
            visual::SKY_RGB[0],
            visual::SKY_RGB[1],
            visual::SKY_RGB[2],
            255,
        ]);
    }

    let mut draws: Vec<Draw> = visual::terrain_draws(&level);
    draws.extend(visual::dynamic_draws(&snap, game.form(), game.goal_pos()));
    for d in &draws {
        let (img, cols) = match d.atlas {
            Atlas::Tiles => (&tiles, visual::TILES_COLS as usize),
            Atlas::Sprites => (&sprites, visual::SPRITES_COLS as usize),
        };
        let (sx, sy) = ((d.index % cols) * cell, (d.index / cols) * cell);
        let left = ((d.x - 0.5) * cell as f32 * scale as f32).round() as i64;
        let top = ((d.y - 0.5) * cell as f32 * scale as f32).round() as i64;
        for y in 0..cell * scale {
            for x in 0..cell * scale {
                let (u, v) = (x / scale, y / scale);
                let u = if d.flip_x { cell - 1 - u } else { u };
                let s = ((sy + v) * img.w + sx + u) * 4;
                if img.rgba[s + 3] == 0 {
                    continue;
                }
                let (dx, dy) = (left + x as i64, top + y as i64);
                if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
                    continue;
                }
                let t = (dy as usize * w + dx as usize) * 4;
                canvas[t..t + 4].copy_from_slice(&img.rgba[s..s + 4]);
            }
        }
    }

    let file = std::fs::File::create(&out_path).map_err(|e| format!("{out_path}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&canvas)
        .map_err(|e| e.to_string())?;
    println!(
        "wrote {out_path} ({w}x{h}) tick={} status={:?}",
        snap.tick(),
        snap.status()
    );
    Ok(())
}
