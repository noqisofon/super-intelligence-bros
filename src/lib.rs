//! Super Intelligence Bros — AI エージェントが遊べるマリオっぽいゲームのヘッドレスコア。
//!
//! 設計方針:
//! - `app.update()` 1回 = ちょうど 1 tick (1/60 秒)。実時間に依存しないので決定的。
//! - 外からは `step(入力, フレーム数)` で進め、`observe()` でテキスト化した状態を読む。
//! - 座標は「タイル単位」、y は下向きが正（テキストグリッドの行と揃える）。
//!
//! 登場キャラ（すべてオリジナル）:
//! - 主人公: ロボット「エスアイくん」。`@`
//! - ボス: 偉そうなスーツのおっさん「スーパー部長」。`B`
//! - アイテム: 「S」キノコ。取ると名前が AI → SI に変わる（中身は変わらない）。`S`

use bevy::prelude::*;

pub mod solver;

pub const DT: f32 = 1.0 / 60.0;
const GRAVITY: f32 = 40.0; // tiles/s^2
const RUN_SPEED: f32 = 6.0; // tiles/s
const JUMP_SPEED: f32 = 14.0; // tiles/s（上向き）
const STOMP_BOUNCE: f32 = 9.0;
const BOSS_SPEED: f32 = 2.0;
const BOSS_HP: u8 = 3;
const BOSS_STUN_TICKS: u32 = 60;
const BOSS_SHOUT_TICKS: u32 = 180;
const INVULN_TICKS: u32 = 90;
const BODY_HALF: Vec2 = Vec2::new(0.4, 0.45);
const ITEM_HALF: Vec2 = Vec2::new(0.35, 0.35);
const GOAL_HALF: Vec2 = Vec2::new(0.4, 0.5);
const EPS: f32 = 1e-4;

// ---------------------------------------------------------------- components

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct Item;

#[derive(Component)]
pub struct Boss {
    pub hp: u8,
    dir: f32,
    stun: u32,
    shout: u32,
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Pos(pub Vec2);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Vel(pub Vec2);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct OnGround(pub bool);

// ---------------------------------------------------------------- resources

/// エージェントが押しているボタン。`step` の間は押しっぱなし。
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadInput {
    pub left: bool,
    pub right: bool,
    pub jump: bool,
}

#[derive(Resource, Default)]
pub struct Tick(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Playing,
    Dead,
    Cleared,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Playing => "playing",
            Status::Dead => "dead",
            Status::Cleared => "cleared",
        }
    }
}

/// 看板（名前）の状態。見た目だけで、強さは変わらない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Ai,
    Si,
}

impl Form {
    fn as_str(self) -> &'static str {
        match self {
            Form::Ai => "AI",
            Form::Si => "SI",
        }
    }
}

#[derive(Resource)]
pub struct PlayerState {
    pub status: Status,
    pub form: Form,
    invuln: u32,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self { status: Status::Playing, form: Form::Ai, invuln: 0 }
    }
}

/// 前回の `observe` 以降に起きたことの文章ログ。LLM がそのまま読む。
#[derive(Resource, Default)]
pub struct EventLog(Vec<String>);

impl EventLog {
    fn push(&mut self, tick: u64, msg: &str) {
        self.0.push(format!("[tick {tick}] {msg}"));
    }
}

#[derive(Resource)]
pub struct Level {
    rows: Vec<Vec<u8>>,
    player_start: Vec2,
    boss_start: Option<Vec2>,
    items: Vec<Vec2>,
    goals: Vec<Vec2>,
}

impl Level {
    /// `#` 地面/ブロック, `P` 土管, `@` プレイヤー開始, `B` ボス, `S` Sキノコ,
    /// `G` ゴール（あればここに触れてクリア。なければボスを倒してクリア）, `.` 空気
    pub fn parse(src: &str) -> Self {
        let mut player_start = Vec2::new(1.5, 1.5);
        let mut boss_start = None;
        let mut items = Vec::new();
        let mut goals = Vec::new();
        let mut rows: Vec<Vec<u8>> = Vec::new();
        for (y, line) in src.lines().enumerate() {
            let mut row = line.as_bytes().to_vec();
            for (x, c) in row.iter_mut().enumerate() {
                // 立っている体は、マーカーのタイルの下端に足が付く位置から始める
                let stand = Vec2::new(x as f32 + 0.5, y as f32 + 1.0 - BODY_HALF.y - EPS);
                match *c {
                    b'@' => {
                        player_start = stand;
                        *c = b'.';
                    }
                    b'B' => {
                        boss_start = Some(stand);
                        *c = b'.';
                    }
                    b'S' => {
                        items.push(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                        *c = b'.';
                    }
                    b'G' => {
                        goals.push(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                        *c = b'.';
                    }
                    _ => {}
                }
            }
            rows.push(row);
        }
        Self { rows, player_start, boss_start, items, goals }
    }

    pub fn width(&self) -> i32 {
        self.rows.first().map_or(0, |r| r.len() as i32)
    }

    pub fn height(&self) -> i32 {
        self.rows.len() as i32
    }

    fn tile(&self, x: i32, y: i32) -> u8 {
        if y < 0 || y >= self.height() {
            return b'.'; // 上空と奈落は空気（落ちたら死亡判定は別）
        }
        if x < 0 || x >= self.width() {
            return b'#'; // 左右の端は壁
        }
        // 行の長さが不揃いなレベルでも落ちないよう、足りない所は空気扱い
        self.rows[y as usize].get(x as usize).copied().unwrap_or(b'.')
    }

    fn solid(&self, x: i32, y: i32) -> bool {
        matches!(self.tile(x, y), b'#' | b'P')
    }

    fn collides(&self, center: Vec2, half: Vec2) -> bool {
        let (x0, x1) = ((center.x - half.x).floor() as i32, (center.x + half.x).floor() as i32);
        let (y0, y1) = ((center.y - half.y).floor() as i32, (center.y + half.y).floor() as i32);
        (y0..=y1).any(|y| (x0..=x1).any(|x| self.solid(x, y)))
    }
}

pub const DEFAULT_LEVEL: &str = "\
........................
........................
........................
........................
........................
........................
.@....S.........PP...B..
################PP######
################PP######";

/// ステージ文字列の検証。エディタ（人間 or LLM）が書いたものを受け入れる前に通す。
pub fn validate_level(src: &str) -> Result<(), String> {
    const MAX_W: usize = 200;
    const MAX_H: usize = 50;
    let rows: Vec<&str> = src.lines().collect();
    if rows.is_empty() {
        return Err("レベルが空です".into());
    }
    if rows.len() > MAX_H {
        return Err(format!("行数が多すぎます（最大 {MAX_H} 行）"));
    }
    let w = rows[0].len();
    if w == 0 || w > MAX_W {
        return Err(format!("幅は 1〜{MAX_W} 列にしてください"));
    }
    let (mut players, mut bosses, mut goals) = (0, 0, 0);
    for (y, row) in rows.iter().enumerate() {
        if row.len() != w {
            return Err(format!(
                "{}行目の幅が {} 列です（1行目は {w} 列。全行そろえてください）",
                y + 1,
                row.len()
            ));
        }
        for (x, c) in row.chars().enumerate() {
            match c {
                '.' | '#' | 'P' | 'S' => {}
                '@' => players += 1,
                'B' => bosses += 1,
                'G' => goals += 1,
                _ => {
                    return Err(format!(
                        "{}行{}列目に使えない文字 {c:?} があります（使える文字: . # P S B G @）",
                        y + 1,
                        x + 1
                    ));
                }
            }
        }
    }
    if players != 1 {
        return Err(format!("'@'（プレイヤー開始位置）はちょうど1つ必要です（今は {players} 個）"));
    }
    if bosses > 1 {
        return Err(format!("'B'（ボス）は最大1つです（今は {bosses} 個）"));
    }
    if goals > 1 {
        return Err(format!("'G'（ゴール）は最大1つです（今は {goals} 個）"));
    }
    Ok(())
}

// ---------------------------------------------------------------- physics

/// 軸分離の移動＋タイル衝突解決。戻り値は (接地したか, 横方向にぶつかったか)。
fn integrate(level: &Level, pos: &mut Vec2, vel: &mut Vec2, half: Vec2) -> (bool, bool) {
    let mut ground = false;
    let mut hit_x = false;

    pos.x += vel.x * DT;
    if level.collides(*pos, half) {
        pos.x = if vel.x > 0.0 {
            (pos.x + half.x).floor() - half.x - EPS
        } else {
            (pos.x - half.x).floor() + 1.0 + half.x + EPS
        };
        vel.x = 0.0;
        hit_x = true;
    }

    pos.y += vel.y * DT;
    if level.collides(*pos, half) {
        if vel.y > 0.0 {
            pos.y = (pos.y + half.y).floor() - half.y - EPS;
            ground = true;
        } else {
            pos.y = (pos.y - half.y).floor() + 1.0 + half.y + EPS;
        }
        vel.y = 0.0;
    }
    (ground, hit_x)
}

fn overlap(a: Vec2, a_half: Vec2, b: Vec2, b_half: Vec2) -> bool {
    (a.x - b.x).abs() < a_half.x + b_half.x && (a.y - b.y).abs() < a_half.y + b_half.y
}

// ---------------------------------------------------------------- systems

fn player_physics(
    input: Res<PadInput>,
    level: Res<Level>,
    state: Res<PlayerState>,
    mut q: Query<(&mut Pos, &mut Vel, &mut OnGround), With<Player>>,
) {
    if state.status != Status::Playing {
        return;
    }
    for (mut pos, mut vel, mut ground) in &mut q {
        let dir = input.right as i32 - input.left as i32;
        vel.0.x = dir as f32 * RUN_SPEED;
        if input.jump && ground.0 {
            vel.0.y = -JUMP_SPEED;
        }
        vel.0.y += GRAVITY * DT;
        let (g, _) = integrate(&level, &mut pos.0, &mut vel.0, BODY_HALF);
        ground.0 = g;
    }
}

fn boss_ai(
    level: Res<Level>,
    state: Res<PlayerState>,
    tick: Res<Tick>,
    mut log: ResMut<EventLog>,
    mut q: Query<(&mut Pos, &mut Vel, &mut Boss)>,
) {
    if state.status != Status::Playing {
        return;
    }
    for (mut pos, mut vel, mut boss) in &mut q {
        if boss.stun > 0 {
            boss.stun -= 1;
            vel.0.x = 0.0;
        } else {
            vel.0.x = boss.dir * BOSS_SPEED;
            boss.shout += 1;
            if boss.shout >= BOSS_SHOUT_TICKS {
                boss.shout = 0;
                log.push(tick.0, "スーパー部長「それはAIじゃない、SIだ！」");
            }
        }
        vel.0.y += GRAVITY * DT;
        let (_, hit_x) = integrate(&level, &mut pos.0, &mut vel.0, BODY_HALF);
        if hit_x {
            boss.dir = -boss.dir;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn interact(
    mut commands: Commands,
    level: Res<Level>,
    tick: Res<Tick>,
    mut state: ResMut<PlayerState>,
    mut log: ResMut<EventLog>,
    mut players: Query<(&Pos, &mut Vel), (With<Player>, Without<Boss>)>,
    mut bosses: Query<(Entity, &Pos, &mut Boss)>,
    items: Query<(Entity, &Pos), (With<Item>, Without<Boss>, Without<Player>)>,
) {
    if state.status != Status::Playing {
        return;
    }
    state.invuln = state.invuln.saturating_sub(1);
    let Ok((ppos, mut pvel)) = players.single_mut() else {
        return;
    };
    let p = ppos.0;
    let t = tick.0;

    if p.y > level.height() as f32 + 2.0 {
        state.status = Status::Dead;
        log.push(t, "奈落に落ちた。ゲームオーバー。");
        return;
    }

    for (e, ipos) in &items {
        if overlap(p, BODY_HALF, ipos.0, ITEM_HALF) {
            commands.entity(e).despawn();
            state.form = Form::Si;
            log.push(t, "Sキノコを取った。看板が AI から SI に変わった。中身は何も変わっていない。");
        }
    }

    for g in &level.goals {
        if overlap(p, BODY_HALF, *g, GOAL_HALF) {
            state.status = Status::Cleared;
            log.push(t, "ゴールに着いた。クリア！");
            return;
        }
    }

    for (e, bpos, mut boss) in &mut bosses {
        if boss.stun > 0 || !overlap(p, BODY_HALF, bpos.0, BODY_HALF) {
            continue;
        }
        if pvel.0.y > 0.0 && p.y < bpos.0.y - 0.1 {
            // 踏みつけ
            boss.hp = boss.hp.saturating_sub(1);
            pvel.0.y = -STOMP_BOUNCE;
            if boss.hp == 0 {
                commands.entity(e).despawn();
                if level.goals.is_empty() {
                    state.status = Status::Cleared;
                    log.push(t, "スーパー部長を踏み倒した。「SI」の看板が落ちて、静かに「AI」と読めた。クリア！");
                } else {
                    log.push(t, "スーパー部長を踏み倒した。「SI」の看板が落ちて、静かに「AI」と読めた。あとはゴールだ。");
                }
            } else {
                boss.stun = BOSS_STUN_TICKS;
                log.push(
                    t,
                    &format!("スーパー部長を踏んだ。看板が「AI」に戻った。(残りHP {})", boss.hp),
                );
            }
        } else if state.invuln == 0 {
            if state.form == Form::Si {
                state.form = Form::Ai;
                state.invuln = INVULN_TICKS;
                log.push(t, "スーパー部長にぶつかった。SIの看板を奪われて AI に戻った。");
            } else {
                state.status = Status::Dead;
                log.push(t, "スーパー部長にぶつかってやられた。ゲームオーバー。");
            }
        }
    }
}

fn count_tick(mut tick: ResMut<Tick>) {
    tick.0 += 1;
}

// ---------------------------------------------------------------- Snapshot

#[derive(Clone, Debug)]
struct BossSnap {
    pos: Vec2,
    vel: Vec2,
    hp: u8,
    dir: f32,
    stun: u32,
    shout: u32,
}

/// ゲーム状態の完全なコピー。ソルバーが「この状態に戻ってやり直す」ために使う。
#[derive(Clone, Debug)]
pub struct Snapshot {
    tick: u64,
    status: Status,
    form: Form,
    invuln: u32,
    player_pos: Vec2,
    player_vel: Vec2,
    player_ground: bool,
    boss: Option<BossSnap>,
    items: Vec<Vec2>,
}

impl Snapshot {
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn player_pos(&self) -> Vec2 {
        self.player_pos
    }

    pub fn boss_pos(&self) -> Option<Vec2> {
        self.boss.as_ref().map(|b| b.pos)
    }

    pub fn boss_hp(&self) -> Option<u8> {
        self.boss.as_ref().map(|b| b.hp)
    }

    /// 「同じ状態」とみなすための量子化キー。時刻(tick)は含めない
    /// （ボスの位相・無敵時間などが状態に入っているので、時刻自体は結果に影響しない）。
    pub fn key(&self) -> Vec<i32> {
        let q = |v: f32, scale: f32| (v * scale).round() as i32;
        let mut k = vec![
            q(self.player_pos.x, 32.0),
            q(self.player_pos.y, 32.0),
            q(self.player_vel.x, 8.0),
            q(self.player_vel.y, 8.0),
            self.player_ground as i32,
            self.status as i32,
            self.form as i32,
            self.invuln as i32,
        ];
        match &self.boss {
            Some(b) => k.extend([
                1,
                q(b.pos.x, 32.0),
                q(b.pos.y, 32.0),
                q(b.vel.x, 4.0),
                b.hp as i32,
                b.dir as i32,
                b.stun as i32,
            ]),
            None => k.push(0),
        }
        for i in &self.items {
            k.extend([q(i.x, 2.0), q(i.y, 2.0)]);
        }
        k
    }
}

// ---------------------------------------------------------------- Game

/// Bevy の `App` をヘッドレスで包んだもの。MCP サーバーはこれを叩く。
pub struct Game {
    app: App,
}

impl Game {
    pub fn new() -> Self {
        Self::with_level(DEFAULT_LEVEL)
    }

    /// 検証してからステージを読み込む。エディタ/MCP からはこちらを使う。
    pub fn try_with_level(src: &str) -> Result<Self, String> {
        validate_level(src)?;
        Ok(Self::with_level(src))
    }

    pub fn with_level(src: &str) -> Self {
        let level = Level::parse(src);
        let player_start = level.player_start;
        let boss_start = level.boss_start;
        let items = level.items.clone();

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(level)
            .init_resource::<PadInput>()
            .init_resource::<Tick>()
            .init_resource::<PlayerState>()
            .init_resource::<EventLog>()
            .add_systems(Update, (player_physics, boss_ai, interact, count_tick).chain());
        app.finish();
        app.cleanup();

        let world = app.world_mut();
        world.spawn((Player, Pos(player_start), Vel::default(), OnGround::default()));
        if let Some(b) = boss_start {
            world.spawn((
                Boss { hp: BOSS_HP, dir: -1.0, stun: 0, shout: 0 },
                Pos(b),
                Vel::default(),
            ));
        }
        for i in items {
            world.spawn((Item, Pos(i)));
        }

        Self { app }
    }

    /// 入力を押したまま `frames` tick 進める。終了状態（dead / cleared）では何も進まない。
    pub fn step(&mut self, input: PadInput, frames: u32) {
        *self.app.world_mut().resource_mut::<PadInput>() = input;
        for _ in 0..frames {
            self.app.update();
        }
    }

    /// 現在の状態をテキストで返す。前回の observe 以降のイベントも含み、ログは空になる。
    pub fn observe(&mut self) -> String {
        let world = self.app.world_mut();
        let tick = world.resource::<Tick>().0;
        let (pos, vel, ground) = {
            let mut q = world.query_filtered::<(&Pos, &Vel, &OnGround), With<Player>>();
            let (p, v, g) = q.single(world).expect("player must exist");
            (p.0, v.0, g.0)
        };
        let boss = {
            let mut q = world.query::<(&Pos, &Boss)>();
            q.iter(world).next().map(|(p, b)| (p.0, b.hp, b.stun > 0))
        };
        let item_tiles: Vec<Vec2> = {
            let mut q = world.query_filtered::<&Pos, With<Item>>();
            q.iter(world).map(|p| p.0).collect()
        };
        let (status, form) = {
            let s = world.resource::<PlayerState>();
            (s.status, s.form)
        };
        let events = std::mem::take(&mut world.resource_mut::<EventLog>().0);
        let level = world.resource::<Level>();

        // グリッド: 地形 → アイテム → ボス → プレイヤーの順に重ねる
        let mut grid: Vec<Vec<char>> = (0..level.height())
            .map(|y| (0..level.width()).map(|x| level.tile(x, y) as char).collect())
            .collect();
        let mut put = |p: Vec2, c: char| {
            let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
            if (0..level.width()).contains(&x) && (0..level.height()).contains(&y) {
                grid[y as usize][x as usize] = c;
            }
        };
        for g in &level.goals {
            put(*g, 'G');
        }
        for p in &item_tiles {
            put(*p, 'S');
        }
        if let Some((bp, _, _)) = boss {
            put(bp, 'B');
        }
        put(pos, '@');

        let mut out = format!(
            "tick={tick} status={} form={}\nplayer: pos=({:.2},{:.2}) vel=({:.2},{:.2}) on_ground={ground}\n",
            status.as_str(),
            form.as_str(),
            pos.x, pos.y, vel.x, vel.y
        );
        match boss {
            Some((bp, hp, stunned)) => out.push_str(&format!(
                "boss: hp={hp} sign={} pos=({:.2},{:.2})\n",
                if stunned { "AI" } else { "SI" },
                bp.x, bp.y
            )),
            None => out.push_str("boss: defeated\n"),
        }
        for row in grid {
            out.extend(row);
            out.push('\n');
        }
        if !events.is_empty() {
            out.push_str("events:\n");
            for e in events {
                out.push_str(&format!("- {e}\n"));
            }
        }
        out
    }

    /// いまの状態を丸ごと保存する。
    pub fn snapshot(&mut self) -> Snapshot {
        let world = self.app.world_mut();
        let tick = world.resource::<Tick>().0;
        let (status, form, invuln) = {
            let s = world.resource::<PlayerState>();
            (s.status, s.form, s.invuln)
        };
        let (player_pos, player_vel, player_ground) = {
            let mut q = world.query_filtered::<(&Pos, &Vel, &OnGround), With<Player>>();
            let (p, v, g) = q.single(world).expect("player must exist");
            (p.0, v.0, g.0)
        };
        let boss = {
            let mut q = world.query::<(&Pos, &Vel, &Boss)>();
            q.iter(world).next().map(|(p, v, b)| BossSnap {
                pos: p.0,
                vel: v.0,
                hp: b.hp,
                dir: b.dir,
                stun: b.stun,
                shout: b.shout,
            })
        };
        let mut items: Vec<Vec2> = {
            let mut q = world.query_filtered::<&Pos, With<Item>>();
            q.iter(world).map(|p| p.0).collect()
        };
        items.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        Snapshot { tick, status, form, invuln, player_pos, player_vel, player_ground, boss, items }
    }

    /// 保存した状態に巻き戻す（イベントログは空になる）。
    pub fn restore(&mut self, s: &Snapshot) {
        let world = self.app.world_mut();
        world.resource_mut::<Tick>().0 = s.tick;
        {
            let mut st = world.resource_mut::<PlayerState>();
            st.status = s.status;
            st.form = s.form;
            st.invuln = s.invuln;
        }
        world.resource_mut::<EventLog>().0.clear();
        {
            let mut q = world.query_filtered::<(&mut Pos, &mut Vel, &mut OnGround), With<Player>>();
            let (mut p, mut v, mut g) = q.single_mut(world).expect("player must exist");
            p.0 = s.player_pos;
            v.0 = s.player_vel;
            g.0 = s.player_ground;
        }
        let old: Vec<Entity> = {
            let mut q = world.query_filtered::<Entity, Or<(With<Boss>, With<Item>)>>();
            q.iter(world).collect()
        };
        for e in old {
            world.despawn(e);
        }
        if let Some(b) = &s.boss {
            world.spawn((
                Boss { hp: b.hp, dir: b.dir, stun: b.stun, shout: b.shout },
                Pos(b.pos),
                Vel(b.vel),
            ));
        }
        for i in &s.items {
            world.spawn((Item, Pos(*i)));
        }
    }

    /// ゴールタイルの位置（なければ None）。
    pub fn goal_pos(&self) -> Option<Vec2> {
        self.app.world().resource::<Level>().goals.first().copied()
    }

    // ---- テスト・MCP 用のアクセサ

    pub fn player_pos(&mut self) -> Vec2 {
        let world = self.app.world_mut();
        let mut q = world.query_filtered::<&Pos, With<Player>>();
        q.single(world).expect("player must exist").0
    }

    pub fn status(&self) -> Status {
        self.app.world().resource::<PlayerState>().status
    }

    pub fn form(&self) -> Form {
        self.app.world().resource::<PlayerState>().form
    }

    pub fn boss_hp(&mut self) -> Option<u8> {
        let world = self.app.world_mut();
        let mut q = world.query::<&Boss>();
        q.iter(world).next().map(|b| b.hp)
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    const RIGHT: PadInput = PadInput { left: false, right: true, jump: false };
    const IDLE: PadInput = PadInput { left: false, right: false, jump: false };
    const JUMP_RIGHT: PadInput = PadInput { left: false, right: true, jump: true };

    /// ボスがうろうろするだけの平らな舞台。
    const ARENA: &str = "\
........................
........................
........................
........................
.@...................B..
########################";

    #[test]
    fn one_frame_returns_position() {
        let mut g = Game::new();
        g.step(IDLE, 1);
        let out = g.observe();
        assert!(out.starts_with("tick=1 "), "{out}");
    }

    #[test]
    fn stands_on_ground() {
        let mut g = Game::new();
        g.step(IDLE, 30);
        assert!(g.observe().contains("on_ground=true"));
    }

    #[test]
    fn runs_right_and_stops_at_pipe() {
        let mut g = Game::new();
        g.step(RIGHT, 600); // 十分長く走る
        let p = g.player_pos();
        // 土管は x=16..18。左面(x=16)の手前で止まるはず
        assert!(p.x < 16.0 && p.x > 15.0, "x={}", p.x);
    }

    #[test]
    fn can_jump_over_pipe() {
        let mut g = Game::new();
        g.step(RIGHT, 60); // 助走
        let mut best_x = 0.0f32;
        for _ in 0..20 {
            g.step(JUMP_RIGHT, 20);
            best_x = best_x.max(g.player_pos().x);
        }
        assert!(best_x > 18.0, "pipe was not cleared: x={best_x}");
    }

    #[test]
    fn s_mushroom_changes_the_sign_only() {
        let mut g = Game::new();
        assert_eq!(g.form(), Form::Ai);
        g.step(RIGHT, 60); // S は x=6.5 付近
        assert_eq!(g.form(), Form::Si);
        let out = g.observe();
        assert!(out.contains("Sキノコ"), "{out}");
        // 取ったアイテムはグリッドから消える
        assert!(!out.contains('S') || out.contains("form=SI"), "{out}");
    }

    #[test]
    fn touching_boss_from_the_side_kills_or_shrinks() {
        let mut g = Game::with_level(ARENA);
        g.step(RIGHT, 600);
        assert_ne!(g.status(), Status::Playing);
    }

    #[test]
    fn boss_can_be_stomped() {
        // 走りながら、ジャンプ開始のタイミングを総当たりして踏める条件があることを確かめる
        let mut stomped = false;
        for delay in 0..120 {
            let mut g = Game::with_level(ARENA);
            g.step(RIGHT, delay);
            for _ in 0..30 {
                g.step(JUMP_RIGHT, 10);
                if g.status() != Status::Playing {
                    break;
                }
            }
            if g.boss_hp().is_none_or(|hp| hp < BOSS_HP) && g.status() != Status::Dead {
                stomped = true;
                break;
            }
        }
        assert!(stomped, "no jump timing could stomp the boss");
    }

    #[test]
    fn default_level_is_valid() {
        assert_eq!(validate_level(DEFAULT_LEVEL), Ok(()));
        assert_eq!(validate_level(ARENA), Ok(()));
    }

    #[test]
    fn invalid_levels_are_rejected_with_reasons() {
        let err = |s: &str| validate_level(s).unwrap_err();
        assert!(err("").contains("空"));
        assert!(err("..\n...\n").contains("2行目"));
        assert!(err("@x\n##").contains("使えない文字"));
        assert!(err("..\n##").contains("'@'"));
        assert!(err("@@\n##").contains("'@'"));
        assert!(err("@B\nB#").contains("'B'"));
        assert!(Game::try_with_level("@.\n##").is_ok());
    }

    #[test]
    fn touching_the_goal_clears_the_stage() {
        let mut g = Game::with_level("@....G\n######");
        g.step(RIGHT, 120);
        assert_eq!(g.status(), Status::Cleared);
        let out = g.observe();
        assert!(out.contains("ゴールに着いた"), "{out}");
    }

    /// ボスの真上から落下する状況を作る（ボスのHPは1）。踏めば撃破になる。
    fn game_about_to_stomp_boss(level: &str) -> Game {
        let mut g = Game::with_level(level);
        let mut snap = g.snapshot();
        let boss = snap.boss.as_mut().expect("level must have a boss");
        boss.hp = 1;
        let above = Vec2::new(boss.pos.x, boss.pos.y - 1.0);
        snap.player_pos = above;
        snap.player_vel = Vec2::new(0.0, 5.0);
        snap.player_ground = false;
        g.restore(&snap);
        g
    }

    const BOSS_AND_GOAL: &str = "\
...........
.@......B.G
###########";
    const BOSS_ONLY: &str = "\
...........
.@......B..
###########";

    #[test]
    fn defeating_the_boss_clears_only_when_there_is_no_goal() {
        // ゴールがないステージ: ボスを踏み倒したらクリア
        let mut g = game_about_to_stomp_boss(BOSS_ONLY);
        g.step(IDLE, 10);
        assert_eq!(g.boss_hp(), None, "ボスは倒れているはず");
        assert_eq!(g.status(), Status::Cleared);

        // ゴールがあるステージ: ボスを倒してもまだ playing。G に触れるまでクリアにならない
        let mut g = game_about_to_stomp_boss(BOSS_AND_GOAL);
        g.step(IDLE, 10);
        assert_eq!(g.boss_hp(), None, "ボスは倒れているはず");
        assert_eq!(g.status(), Status::Playing);
        assert!(g.observe().contains("あとはゴールだ"));
    }

    #[test]
    fn snapshot_restore_roundtrip_is_exact() {
        let mut g = Game::new();
        g.step(RIGHT, 70); // Sキノコを取り、ボスも動いている
        let snap = g.snapshot();
        let key = snap.key();

        g.step(JUMP_RIGHT, 50);
        g.step(IDLE, 30);
        assert_ne!(g.snapshot().key(), key, "状態が動いているはず");

        g.restore(&snap);
        assert_eq!(g.snapshot().key(), key, "復元後は保存時と同じ状態");

        // 復元した状態から先は、最初から同じ入力を流した場合と完全に一致する
        g.step(JUMP_RIGHT, 40);
        let after_restore = g.observe();

        let mut fresh = Game::new();
        fresh.step(RIGHT, 70);
        fresh.step(JUMP_RIGHT, 40);
        let after_fresh = fresh.observe();

        let strip = |s: &str| s.lines().filter(|l| !l.starts_with("events") && !l.starts_with("- ")).collect::<Vec<_>>().join("\n");
        assert_eq!(strip(&after_restore), strip(&after_fresh));
    }

    #[test]
    fn restoring_brings_back_a_picked_up_item() {
        let mut g = Game::new();
        let before = g.snapshot();
        g.step(RIGHT, 60);
        assert_eq!(g.form(), Form::Si);
        g.restore(&before);
        assert_eq!(g.form(), Form::Ai);
        assert!(g.observe().contains('S'), "Sキノコがグリッドに戻っているはず");
    }

    #[test]
    fn goal_level_validation() {
        assert!(validate_level("@.G\n###").is_ok());
        assert!(validate_level("@GG\n###").unwrap_err().contains("'G'"));
    }

    #[test]
    fn deterministic() {
        let run = || {
            let mut g = Game::new();
            g.step(RIGHT, 40);
            g.step(JUMP_RIGHT, 25);
            g.step(IDLE, 10);
            g.observe()
        };
        assert_eq!(run(), run());
    }
}
