use super_intelligence_bros::{Game, PadInput};

fn main() {
    let mut game = Game::new();
    let right = PadInput {
        right: true,
        ..Default::default()
    };
    let jump_right = PadInput {
        right: true,
        jump: true,
        ..Default::default()
    };

    println!("--- 右に1秒走る（Sキノコを取る）");
    game.step(right, 60);
    println!("{}", game.observe());

    println!("--- 土管の手前までさらに走る");
    game.step(right, 90);
    println!("{}", game.observe());

    println!("--- 右+ジャンプを30フレーム（土管越え）");
    game.step(jump_right, 30);
    println!("{}", game.observe());

    println!("--- 5秒そのまま走る（ボスに会う）");
    game.step(right, 300);
    println!("{}", game.observe());
}
