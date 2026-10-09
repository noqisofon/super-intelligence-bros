//! 見た目の割り当て（描画エンジンに依存しない部分）。
//!
//! 「どのマスにどのタイルを描くか」「キャラにどのスプライトを使うか」をここで決める。
//! Bevy の描画版（`sibros-view`）と、CPU で PNG を出す `sibros-snap` の両方がこれを使うので、
//! 割り当てを PNG で確認すれば、描画版でも同じ絵になる。
//!
//! 画像は 8x8 のタイルが並んだシート。
//! - `assets/tiles_1985.png`: 8列x10行（Adam Saltsman 氏の「1985」）
//! - `assets/sprites.png`: 8列x1行（オリジナル。`tools/gen_sprites.py` で生成）

use bevy::prelude::Vec2;

use crate::{Form, Level, Snapshot};

pub const TILE_PX: u32 = 8;
pub const TILES_COLS: u32 = 8;
pub const TILES_ROWS: u32 = 10;
pub const SPRITES_COLS: u32 = 8;
pub const SPRITES_ROWS: u32 = 1;

/// 空の色（タイルシートの空タイルと同じ）。
pub const SKY_RGB: [u8; 3] = [104, 136, 252];

/// 体の当たり判定の下端に、スプライトの下端をそろえるための y 補正（タイル単位）。
/// 体の高さは 0.9、スプライトは 1.0 なので、中心を 0.05 上にずらす。
pub const BODY_SPRITE_DY: f32 = -0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atlas {
    Tiles,
    Sprites,
}

/// 描く 1 枚。座標はタイル単位の**中心**（y は下向きが正）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub atlas: Atlas,
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub flip_x: bool,
}

/// `sprites.png` の並び。
pub mod sprite {
    pub const ROBOT_IDLE: usize = 0;
    pub const ROBOT_RUN: usize = 1;
    pub const ROBOT_JUMP: usize = 2;
    pub const ROBOT_SI: usize = 3;
    pub const BOSS: usize = 4;
    pub const BOSS_STUNNED: usize = 5;
    pub const MUSHROOM: usize = 6;
    pub const GOAL: usize = 7;
}

/// `tiles_1985.png` の中で使うタイル番号（参考画像で使われ方を確認した）。
pub mod tile {
    /// 地面の表面（x が偶数/奇数で交互に使う）
    pub const GROUND_TOP: [usize; 2] = [0, 1];
    /// 地面の2段目
    pub const GROUND_MID: [usize; 2] = [8, 9];
    /// 地面の3段目より下
    pub const GROUND_DEEP: usize = 12;
    /// 土管の上端（左・右）
    pub const PIPE_HEAD: [usize; 2] = [25, 26];
    /// 土管の胴体（左・右）
    pub const PIPE_BODY: [usize; 2] = [33, 34];
}

fn is(level: &Level, x: i32, y: i32, c: u8) -> bool {
    level.tile(x, y) == c && x >= 0 && x < level.width() && y >= 0 && y < level.height()
}

/// 地形（`#` と `P`）を描くタイルの一覧。空（`.`）は背景色で表すので含まない。
pub fn terrain_draws(level: &Level) -> Vec<Draw> {
    let mut out = Vec::new();
    for y in 0..level.height() {
        for x in 0..level.width() {
            let index = if is(level, x, y, b'#') {
                // 上に何段 `#` が続いているか
                let mut depth = 0;
                while is(level, x, y - 1 - depth, b'#') {
                    depth += 1;
                }
                let parity = (x.rem_euclid(2)) as usize;
                match depth {
                    0 => tile::GROUND_TOP[parity],
                    1 => tile::GROUND_MID[parity],
                    _ => tile::GROUND_DEEP,
                }
            } else if is(level, x, y, b'P') {
                let head = !is(level, x, y - 1, b'P');
                let right = is(level, x - 1, y, b'P');
                let side = right as usize;
                if head { tile::PIPE_HEAD[side] } else { tile::PIPE_BODY[side] }
            } else {
                continue;
            };
            out.push(Draw {
                atlas: Atlas::Tiles,
                index,
                x: x as f32 + 0.5,
                y: y as f32 + 0.5,
                flip_x: false,
            });
        }
    }
    out
}

/// プレイヤーの絵。接地していなければジャンプ、動いていれば走り、止まっていれば立ち。
/// SI 形態のときは、動きにかかわらず専用の絵（胸が黄色い）を使う。
pub fn player_sprite(snap: &Snapshot, form: Form) -> usize {
    if form == Form::Si && snap.player_ground {
        return sprite::ROBOT_SI;
    }
    if !snap.player_ground {
        sprite::ROBOT_JUMP
    } else if snap.player_vel.x.abs() > 0.1 {
        sprite::ROBOT_RUN
    } else {
        sprite::ROBOT_IDLE
    }
}

/// 動くもの（ゴール・アイテム・ボス・プレイヤー）の描画一覧。奥から手前の順。
pub fn dynamic_draws(snap: &Snapshot, form: Form, goal: Option<Vec2>) -> Vec<Draw> {
    let mut out = Vec::new();
    let mut push = |index: usize, p: Vec2, dy: f32, flip_x: bool| {
        out.push(Draw { atlas: Atlas::Sprites, index, x: p.x, y: p.y + dy, flip_x });
    };
    if let Some(g) = goal {
        push(sprite::GOAL, g, 0.0, false);
    }
    for i in &snap.items {
        push(sprite::MUSHROOM, *i, 0.0, false);
    }
    if let Some(b) = &snap.boss {
        let index = if b.stun > 0 { sprite::BOSS_STUNNED } else { sprite::BOSS };
        push(index, b.pos, BODY_SPRITE_DY, b.vel.x > 0.0);
    }
    push(
        player_sprite(snap, form),
        snap.player_pos,
        BODY_SPRITE_DY,
        snap.player_vel.x < -0.1,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEFAULT_LEVEL, Game};

    fn draw_at(draws: &[Draw], x: i32, y: i32) -> Option<usize> {
        draws
            .iter()
            .find(|d| d.atlas == Atlas::Tiles && d.x == x as f32 + 0.5 && d.y == y as f32 + 0.5)
            .map(|d| d.index)
    }

    #[test]
    fn ground_gets_top_mid_and_deep_tiles() {
        let level = Level::parse("....\n####\n####\n####\n####");
        let d = terrain_draws(&level);
        // 表面は x の偶奇で 0/1 を交互に
        assert_eq!(draw_at(&d, 0, 1), Some(0));
        assert_eq!(draw_at(&d, 1, 1), Some(1));
        // 2段目は 8/9、それより下は 12
        assert_eq!(draw_at(&d, 0, 2), Some(8));
        assert_eq!(draw_at(&d, 1, 2), Some(9));
        assert_eq!(draw_at(&d, 0, 3), Some(12));
        assert_eq!(draw_at(&d, 3, 4), Some(12));
        // 空は描かない
        assert_eq!(draw_at(&d, 0, 0), None);
    }

    #[test]
    fn pipes_have_head_and_body_with_left_and_right() {
        let level = Level::parse(DEFAULT_LEVEL);
        let d = terrain_draws(&level);
        // 初期ステージの土管は x=16,17。上端は row 6、その下は胴体
        assert_eq!(draw_at(&d, 16, 6), Some(25));
        assert_eq!(draw_at(&d, 17, 6), Some(26));
        assert_eq!(draw_at(&d, 16, 7), Some(33));
        assert_eq!(draw_at(&d, 17, 7), Some(34));
        assert_eq!(draw_at(&d, 16, 8), Some(33));
    }

    #[test]
    fn ground_next_to_a_pipe_keeps_its_own_depth() {
        let level = Level::parse(DEFAULT_LEVEL);
        let d = terrain_draws(&level);
        // 土管の隣の地面は、土管に影響されず表面(row 7)・2段目(row 8)になる
        assert_eq!(draw_at(&d, 15, 7), Some(1));
        assert_eq!(draw_at(&d, 15, 8), Some(9));
        // 土管の右隣(x=18)の地面も同様
        assert_eq!(draw_at(&d, 18, 7), Some(0));
    }

    #[test]
    fn player_sprite_follows_state() {
        let mut g = Game::new();
        g.step(crate::PadInput::default(), 20);
        let s = g.snapshot();
        assert_eq!(player_sprite(&s, Form::Ai), sprite::ROBOT_IDLE);
        assert_eq!(player_sprite(&s, Form::Si), sprite::ROBOT_SI);

        g.step(crate::PadInput { right: true, ..Default::default() }, 5);
        assert_eq!(player_sprite(&g.snapshot(), Form::Ai), sprite::ROBOT_RUN);

        g.step(crate::PadInput { jump: true, ..Default::default() }, 3);
        assert_eq!(player_sprite(&g.snapshot(), Form::Ai), sprite::ROBOT_JUMP);
    }

    #[test]
    fn dynamic_draws_include_boss_item_goal_and_player_in_order() {
        let mut g = Game::with_level("@.S.B.G\n#######");
        let goal = g.goal_pos();
        let snap = g.snapshot();
        let d = dynamic_draws(&snap, Form::Ai, goal);
        let idx: Vec<usize> = d.iter().map(|x| x.index).collect();
        // 奥から手前: ゴール → アイテム → ボス → プレイヤー
        assert_eq!(idx, vec![sprite::GOAL, sprite::MUSHROOM, sprite::BOSS, sprite::ROBOT_IDLE]);
    }
}
