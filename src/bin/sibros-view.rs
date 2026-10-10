//! Super Intelligence Bros の描画版。人間が画面を見て遊ぶためのビューア。
//!
//!   cargo run --features view --bin sibros-view [ステージのテキストファイル]
//!
//! 操作: ← → / A D で移動、Space / Z / ↑ / W でジャンプ、R でやり直し。
//!
//! ゲームのルールはヘッドレス版（MCP サーバー）と同じコアを使う。
//! 「どのタイルを描くか」は `visual` モジュールで決まり、`sibros-snap` で見た絵と同じになる。
//!
//! 注意: 画面の文字は ASCII のみ（Bevy 標準フォントが日本語に対応していないため）。

use bevy::prelude::*;
use bevy::window::WindowResolution;
use super_intelligence_bros::visual::{self, Atlas, Draw};
use super_intelligence_bros::{DEFAULT_LEVEL, Form, Game, Level, PadInput, Status};

/// 1 タイルの表示サイズ（px）。元画像は 8x8 なので 4 倍。
const T: f32 = 32.0;
const WINDOW: (u32, u32) = (960, 540);

/// ゲーム本体。Bevy の `App` を内側に持っていて `Send` ではないので、NonSend として置く。
struct GameRes {
    game: Game,
}

#[derive(Resource)]
struct LevelInfo {
    src: String,
    width: i32,
    height: i32,
}

#[derive(Resource)]
struct Atlases {
    tiles: (Handle<Image>, Handle<TextureAtlasLayout>),
    sprites: (Handle<Image>, Handle<TextureAtlasLayout>),
}

/// 毎フレーム作り直す動くもの（キャラ・アイテム・ゴール）の印。
#[derive(Component)]
struct Dynamic;

#[derive(Component)]
struct HudText;

#[derive(Resource, Default)]
struct CameraTarget(Vec2);

fn main() {
    let src = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }),
        None => DEFAULT_LEVEL.to_string(),
    };
    let game = Game::try_with_level(&src).unwrap_or_else(|e| {
        eprintln!("ステージが不正です: {e}");
        std::process::exit(1);
    });
    let level = Level::parse(&src);
    let info = LevelInfo {
        width: level.width(),
        height: level.height(),
        src,
    };

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest()) // ドット絵をぼかさない
                .set(AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Super Intelligence Bros".into(),
                        resolution: WindowResolution::new(WINDOW.0, WINDOW.1),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb_u8(
            visual::SKY_RGB[0],
            visual::SKY_RGB[1],
            visual::SKY_RGB[2],
        )))
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_non_send(GameRes { game })
        .insert_resource(info)
        .init_resource::<CameraTarget>()
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, step_game)
        .add_systems(
            Update,
            (handle_reset, sync_dynamic, follow_camera, update_hud).chain(),
        )
        .run();
}

fn sprite_for(atlases: &Atlases, d: &Draw) -> Sprite {
    let (image, layout) = match d.atlas {
        Atlas::Tiles => atlases.tiles.clone(),
        Atlas::Sprites => atlases.sprites.clone(),
    };
    let mut sprite = Sprite::from_atlas_image(
        image,
        TextureAtlas {
            layout,
            index: d.index,
        },
    );
    sprite.custom_size = Some(Vec2::splat(T));
    sprite.flip_x = d.flip_x;
    sprite
}

/// タイル単位（y 下向き）→ 画面のワールド座標（y 上向き）。
fn world_pos(d: &Draw, z: f32) -> Transform {
    Transform::from_xyz(d.x * T, -d.y * T, z)
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    info: Res<LevelInfo>,
) {
    let cell = UVec2::splat(visual::TILE_PX);
    let tiles = (
        assets.load("tiles_1985.png"),
        layouts.add(TextureAtlasLayout::from_grid(
            cell,
            visual::TILES_COLS,
            visual::TILES_ROWS,
            None,
            None,
        )),
    );
    let sprites = (
        assets.load("sprites.png"),
        layouts.add(TextureAtlasLayout::from_grid(
            cell,
            visual::SPRITES_COLS,
            visual::SPRITES_ROWS,
            None,
            None,
        )),
    );
    let atlases = Atlases { tiles, sprites };

    commands.spawn(Camera2d);

    // 地形は変わらないので最初に一度だけ並べる
    let level = Level::parse(&info.src);
    for d in visual::terrain_draws(&level) {
        commands.spawn((sprite_for(&atlases, &d), world_pos(&d, 0.0)));
    }
    commands.insert_resource(atlases);

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(8.0),
            ..default()
        },
        HudText,
    ));
}

fn read_pad(keys: &ButtonInput<KeyCode>) -> PadInput {
    PadInput {
        left: keys.any_pressed([KeyCode::ArrowLeft, KeyCode::KeyA]),
        right: keys.any_pressed([KeyCode::ArrowRight, KeyCode::KeyD]),
        jump: keys.any_pressed([
            KeyCode::Space,
            KeyCode::KeyZ,
            KeyCode::ArrowUp,
            KeyCode::KeyW,
        ]),
    }
}

/// 固定 60Hz で、キー入力を押したまま 1 tick 進める。
fn step_game(mut g: NonSendMut<GameRes>, keys: Res<ButtonInput<KeyCode>>) {
    g.game.step(read_pad(&keys), 1);
}

fn handle_reset(mut g: NonSendMut<GameRes>, keys: Res<ButtonInput<KeyCode>>, info: Res<LevelInfo>) {
    if keys.just_pressed(KeyCode::KeyR) {
        g.game = Game::with_level(&info.src);
    }
}

/// 動くもの（キャラ・アイテム・ゴール）の描画をゲーム状態と同期する。
/// 既存の Entity を再利用し、過不足分のみ spawn / despawn する。
fn sync_dynamic(
    mut commands: Commands,
    mut g: NonSendMut<GameRes>,
    atlases: Res<Atlases>,
    mut dynamic_query: Query<(Entity, &mut Sprite, &mut Transform), With<Dynamic>>,
    mut target: ResMut<CameraTarget>,
) {
    let snap = g.game.snapshot();
    let draws = visual::dynamic_draws(&snap, g.game.form(), g.game.goal_pos());

    let mut existing: Vec<_> = dynamic_query.iter_mut().collect();
    for (i, d) in draws.iter().enumerate() {
        let pos = world_pos(d, 1.0 + i as f32 * 0.1);
        let sprite = sprite_for(&atlases, d);
        if let Some((_, existing_sprite, existing_pos)) = existing.get_mut(i) {
            **existing_sprite = sprite;
            **existing_pos = pos;
        } else {
            commands.spawn((sprite, pos, Dynamic));
        }
    }

    // draws より多い既存エンティティ（消えたアイテムやボスなど）は削除
    for (e, _, _) in existing.into_iter().skip(draws.len()) {
        commands.entity(e).despawn();
    }

    let p = snap.player_pos();
    target.0 = Vec2::new(p.x * T, -p.y * T);
}

fn follow_camera(
    target: Res<CameraTarget>,
    info: Res<LevelInfo>,
    mut cam: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    let (w, h) = (info.width as f32 * T, info.height as f32 * T);
    let half_view = Vec2::new(WINDOW.0 as f32, WINDOW.1 as f32) / 2.0;
    // ステージが画面より狭い方向は中央に置き、広い方向はプレイヤーを追う（端では止める）
    let clamp_axis = |center: f32, level_len: f32, half: f32, level_min: f32| {
        if level_len <= half * 2.0 {
            level_min + level_len / 2.0
        } else {
            center.clamp(level_min + half, level_min + level_len - half)
        }
    };
    t.translation.x = clamp_axis(target.0.x, w, half_view.x, 0.0);
    t.translation.y = clamp_axis(target.0.y, h, half_view.y, -h);
}

fn update_hud(mut g: NonSendMut<GameRes>, mut hud: Query<&mut Text, With<HudText>>) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };
    let form = match g.game.form() {
        Form::Ai => "AI",
        Form::Si => "SI",
    };
    let boss = match g.game.boss_hp() {
        Some(hp) => format!("BOSS HP {hp}"),
        None => "BOSS -".to_string(),
    };
    let status = match g.game.status() {
        Status::Playing => String::new(),
        Status::Cleared => "\nCLEAR!  (R: restart)".to_string(),
        Status::Dead => "\nGAME OVER  (R: restart)".to_string(),
    };
    text.0 = format!("FORM {form}   {boss}{status}");
}
