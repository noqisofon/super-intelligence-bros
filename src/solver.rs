//! ステージの解法探索。
//!
//! ゲームは 1 tick ごとに結果が決まる（決定的）ので、入力列を探索してクリアできるか調べられる。
//!
//! - 入力は「左/なし/右 × ジャンプの有無」の6通りを、一定フレームずつ押しっぱなしにする。
//! - 最良優先探索。状態は `Snapshot::key` で量子化して、同じ状態は二度調べない。
//! - 粗い粒度（6フレーム）で見つからなくても、すぐには諦めない。2フレーム、1フレームと
//!   細かくして探し直し、**最も細かい粒度でも探索しきったときだけ**「到達不能」と返す。
//!   （粗い粒度でジャンプのタイミングを拾いきれず見逃す場合への保険。粗い粒度が実際に
//!   見逃した例は、今のところ見つかっていない）
//! - 状態が多すぎて探索が終わらないステージ（広い穴など）は、`GaveUp`（結論なし）を返す。
//! - 「到達不能」は**入力の粒度（1フレーム）と時間の上限（`max_ticks`）の範囲での結論**で、
//!   「どんな操作でも絶対に不可能」という数学的証明ではない。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::time::Instant;

use crate::{Game, PadInput, Snapshot, Status};

#[derive(Clone, Debug)]
pub struct SolveConfig {
    /// 展開するノード数の上限（全粒度の合計）。
    pub max_expansions: usize,
    /// 見つかった解の最大 tick（これを超える状態は捨てる）。
    pub max_ticks: u64,
    /// 1 回の行動で押しっぱなしにするフレーム数。粗い方から順に試す。
    pub granularities: Vec<u32>,
    /// 探索にかけてよい実時間（秒、全粒度の合計）。
    pub max_seconds: f64,
}

impl Default for SolveConfig {
    fn default() -> Self {
        Self {
            max_expansions: 400_000,
            max_ticks: 3600,
            granularities: vec![6, 2, 1],
            max_seconds: 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub input: PadInput,
    pub frames: u32,
}

#[derive(Clone, Debug)]
pub enum SolveOutcome {
    /// クリアできる入力列が見つかった。
    Solved {
        plan: Vec<PlanStep>,
        ticks: u64,
        expansions: usize,
    },
    /// 最も細かい粒度でも探索しきった（`max_ticks` 以内では到達不能）。
    Unreachable {
        explored: usize,
        max_ticks: u64,
        frames: u32,
    },
    /// 上限に達して打ち切った。結論は出ていない。
    GaveUp {
        expansions: usize,
        reason: &'static str,
    },
}

struct Node {
    snap: Snapshot,
    parent: Option<usize>,
    action: PadInput,
}

fn actions() -> Vec<PadInput> {
    let mut v = Vec::new();
    for (left, right) in [(false, false), (true, false), (false, true)] {
        for jump in [false, true] {
            v.push(PadInput { left, right, jump });
        }
    }
    v
}

/// 目標までの残り時間の見積もり（tick）。最良優先探索の優先度にだけ使う。
fn heuristic(snap: &Snapshot, target_x: Option<f32>) -> u64 {
    let Some(tx) = target_x else { return 0 };
    let dx = (snap.player_pos().x - tx).abs();
    let run = (dx / 6.0 * 60.0) as u64;
    // ボスを倒すのが目的のとき、HPが残っているほど遠いとみなす（1回踏むごとに大きく前進）
    let boss_left = snap.boss_hp().map_or(0, |hp| hp as u64 * 120);
    run + boss_left
}

/// ステージ文字列を探索する。ステージが不正なら Err。
pub fn solve(level_src: &str, cfg: &SolveConfig) -> Result<SolveOutcome, String> {
    let mut game = Game::try_with_level(level_src)?;
    let root = game.snapshot();
    let started = Instant::now();
    let mut used = 0usize;

    let last = cfg.granularities.len().saturating_sub(1);
    for (i, &frames) in cfg.granularities.iter().enumerate() {
        let out = search(&mut game, &root, cfg, frames.max(1), started, &mut used);
        match out {
            // 粗い粒度で探索しきっただけなら、細かくして探し直す
            SolveOutcome::Unreachable { .. } if i < last => continue,
            other => return Ok(other),
        }
    }
    Ok(SolveOutcome::GaveUp {
        expansions: used,
        reason: "粒度の指定が空",
    })
}

fn search(
    game: &mut Game,
    root: &Snapshot,
    cfg: &SolveConfig,
    frames: u32,
    started: Instant,
    used: &mut usize,
) -> SolveOutcome {
    let goal_x = game.goal_pos().map(|g| g.x);
    // 目標: ゴールがあればその x、なければボスの x（ボスは動くので展開ごとに更新する）
    let target = |s: &Snapshot| goal_x.or_else(|| s.boss_pos().map(|b| b.x));
    let acts = actions();

    let mut nodes = vec![Node {
        snap: root.clone(),
        parent: None,
        action: PadInput::default(),
    }];
    let mut seen: HashSet<Vec<i32>> = HashSet::new();
    seen.insert(root.key());
    let mut heap = BinaryHeap::new();
    heap.push(Reverse((heuristic(root, target(root)), 0usize)));
    let mut explored = 0usize;

    while let Some(Reverse((_, idx))) = heap.pop() {
        if *used >= cfg.max_expansions {
            return SolveOutcome::GaveUp {
                expansions: *used,
                reason: "展開数の上限",
            };
        }
        if explored % 256 == 0 && started.elapsed().as_secs_f64() > cfg.max_seconds {
            return SolveOutcome::GaveUp {
                expansions: *used,
                reason: "時間の上限",
            };
        }
        explored += 1;
        *used += 1;

        let base = nodes[idx].snap.clone();
        for &a in &acts {
            game.restore(&base);
            game.step(a, frames);
            let snap = game.snapshot();
            match snap.status() {
                Status::Dead => continue,
                Status::Cleared => {
                    nodes.push(Node {
                        snap: snap.clone(),
                        parent: Some(idx),
                        action: a,
                    });
                    let plan = build_plan(&nodes, nodes.len() - 1, frames);
                    return SolveOutcome::Solved {
                        plan,
                        ticks: snap.tick(),
                        expansions: *used,
                    };
                }
                Status::Playing => {}
            }
            if snap.tick() > cfg.max_ticks || !seen.insert(snap.key()) {
                continue;
            }
            let cost = snap.tick() + 2 * heuristic(&snap, target(&snap));
            nodes.push(Node {
                snap,
                parent: Some(idx),
                action: a,
            });
            heap.push(Reverse((cost, nodes.len() - 1)));
        }
    }
    SolveOutcome::Unreachable {
        explored: *used,
        max_ticks: cfg.max_ticks,
        frames,
    }
}

/// 親をたどって入力列を作り、同じ入力が続く所はまとめる。
fn build_plan(nodes: &[Node], last: usize, frames: u32) -> Vec<PlanStep> {
    let mut rev = Vec::new();
    let mut cur = last;
    while let Some(p) = nodes[cur].parent {
        rev.push(nodes[cur].action);
        cur = p;
    }
    rev.reverse();
    let mut plan: Vec<PlanStep> = Vec::new();
    for a in rev {
        match plan.last_mut() {
            Some(s) if s.input == a => s.frames += frames,
            _ => plan.push(PlanStep { input: a, frames }),
        }
    }
    plan
}

/// 入力列を最初から再生して、最終状態を返す（解の検証用）。
pub fn replay(level_src: &str, plan: &[PlanStep]) -> Result<Status, String> {
    let mut game = Game::try_with_level(level_src)?;
    for s in plan {
        game.step(s.input, s.frames);
    }
    Ok(game.status())
}

fn input_label(i: &PadInput) -> String {
    let mut parts = Vec::new();
    if i.left {
        parts.push("left");
    }
    if i.right {
        parts.push("right");
    }
    if i.jump {
        parts.push("jump");
    }
    if parts.is_empty() {
        "wait".to_string()
    } else {
        parts.join("+")
    }
}

/// 結果を人間（と LLM）が読めるテキストにする。
pub fn format_outcome(o: &SolveOutcome) -> String {
    match o {
        SolveOutcome::Solved {
            plan,
            ticks,
            expansions,
        } => {
            let mut out = format!(
                "solved: {}手・{ticks} tick（約{:.1}秒）でクリアできます（探索 {expansions} ノード）\n",
                plan.len(),
                *ticks as f32 / 60.0
            );
            for (n, s) in plan.iter().take(80).enumerate() {
                out.push_str(&format!(
                    "{}. {} {}f\n",
                    n + 1,
                    input_label(&s.input),
                    s.frames
                ));
            }
            if plan.len() > 80 {
                out.push_str(&format!("...（残り {} 手は省略）\n", plan.len() - 80));
            }
            out
        }
        SolveOutcome::Unreachable {
            explored,
            max_ticks,
            frames,
        } => format!(
            "unreachable: {max_ticks} tick 以内で、{frames}フレーム刻みの操作では、クリアできる入力が見つかりませんでした\
             （合計 {explored} ノードを探索しきりました）。\
             ※この条件の範囲での結論で、絶対に不可能という証明ではありません\n"
        ),
        SolveOutcome::GaveUp { expansions, reason } => format!(
            "gave_up: {reason}に達したので打ち切りました（{expansions} ノード展開）。\
             クリア可能かどうかは分かっていません\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SolveConfig {
        SolveConfig {
            max_seconds: 30.0,
            ..Default::default()
        }
    }

    fn solved_plan(src: &str) -> Vec<PlanStep> {
        match solve(src, &cfg()).unwrap() {
            SolveOutcome::Solved { plan, .. } => plan,
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn solves_a_flat_corridor_and_plan_replays() {
        let src = "@.........G\n###########";
        let plan = solved_plan(src);
        assert_eq!(replay(src, &plan).unwrap(), Status::Cleared);
    }

    #[test]
    fn solves_a_level_that_needs_a_jump() {
        let src = "\
...........
...........
@..P....G..
###########";
        // 土管(P)は高さ1。ジャンプしないと越えられない
        let plan = solved_plan(src);
        assert!(
            plan.iter().any(|s| s.input.jump),
            "plan should jump: {plan:?}"
        );
        assert_eq!(replay(src, &plan).unwrap(), Status::Cleared);
    }

    #[test]
    fn walled_off_goal_is_unreachable() {
        let src = "\
...#...
...#...
...#...
@..#..G
#######";
        match solve(src, &cfg()).unwrap() {
            SolveOutcome::Unreachable {
                explored, frames, ..
            } => {
                assert!(explored > 0);
                assert_eq!(frames, 1, "最も細かい粒度まで試してから結論を出すこと");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    /// 高さ `h` の壁の手前にプレイヤー、奥にゴールを置いたステージ。
    fn wall_level(h: usize) -> String {
        let mut rows = Vec::new();
        for r in 0..5 {
            let wall = 5 - r <= h; // 下から h 段が壁
            let mut line = String::from(if r == 4 { "@..." } else { "...." });
            line.push(if wall { '#' } else { '.' });
            line.push_str(if r == 4 { "..G" } else { "..." });
            rows.push(line);
        }
        rows.push("########".to_string());
        rows.join("\n")
    }

    #[test]
    fn wall_height_boundary() {
        // 高さ1・2は越えられる（最大ジャンプ高さは約2.45タイル）
        for h in [1, 2] {
            let src = wall_level(h);
            let plan = solved_plan(&src);
            assert_eq!(replay(&src, &plan).unwrap(), Status::Cleared, "h={h}");
        }
        // 高さ3は最大ジャンプ高さ（約2.45）を超える
        let src = wall_level(3);
        assert!(matches!(
            solve(&src, &cfg()).unwrap(),
            SolveOutcome::Unreachable { .. }
        ));
    }

    #[test]
    fn solves_the_default_level_by_stomping_the_boss() {
        // ボスは動き回るので、状態が多く探索が重い。ボスのHPを優先度に入れていないと
        // 数十万ノードを使っても解けなかった（回帰テスト）。
        match solve(crate::DEFAULT_LEVEL, &cfg()).unwrap() {
            SolveOutcome::Solved {
                plan, expansions, ..
            } => {
                assert!(expansions < 20_000, "探索が重すぎる: {expansions}");
                assert_eq!(
                    replay(crate::DEFAULT_LEVEL, &plan).unwrap(),
                    Status::Cleared
                );
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn invalid_level_is_an_error() {
        assert!(solve("....", &cfg()).is_err());
    }

    #[test]
    fn tiny_budget_gives_up_instead_of_claiming_unreachable() {
        let src = "@.........G\n###########";
        let cfg = SolveConfig {
            max_expansions: 1,
            ..Default::default()
        };
        assert!(matches!(
            solve(src, &cfg).unwrap(),
            SolveOutcome::GaveUp { .. }
        ));
    }
}
