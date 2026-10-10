//! Super Intelligence Bros の MCP サーバー（stdio）。
//! stdout はプロトコル専用なので、ここでは println! を使わないこと。
//!
//! Bevy の `App` は `Send` ではないため、ゲームは専用スレッドに閉じ込め、
//! MCP 側（非同期ランタイム）からはチャンネルでコマンドを送って結果を受け取る。

use std::sync::mpsc;
use std::thread;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use super_intelligence_bros::solver::{self, SolveConfig};
use super_intelligence_bros::{DEFAULT_LEVEL, Game, PadInput, Status};

const MAX_FRAMES: u32 = 600;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StepArgs {
    /// 左を押す
    #[serde(default)]
    left: bool,
    /// 右を押す
    #[serde(default)]
    right: bool,
    /// ジャンプを押す（接地中だけ跳べる）
    #[serde(default)]
    jump: bool,
    /// 何フレーム（1フレーム=1/60秒）押しっぱなしで進めるか。1〜600。省略すると 10。
    frames: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LoadLevelArgs {
    /// ステージのテキスト。1行=1段、全行同じ幅。
    /// `.`空気 `#`地面 `P`土管 `S`Sキノコ `B`ボス（最大1） `G`ゴール（最大1） `@`プレイヤー開始（ちょうど1）
    level: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolveArgs {
    /// 探索にかけてよい秒数。1〜60。省略すると 10。
    max_seconds: Option<f64>,
}

// ------------------------------------------------------------ ゲームスレッド

enum Cmd {
    Step(PadInput, u32),
    Observe,
    Reset,
    Load(String),
    Solve(f64),
}

struct Req {
    cmd: Cmd,
    reply: mpsc::Sender<String>,
}

fn game_thread(rx: mpsc::Receiver<Req>) {
    let mut level_src = DEFAULT_LEVEL.to_string();
    let mut game = Game::new();

    for req in rx {
        let out = match req.cmd {
            Cmd::Step(input, frames) => {
                game.step(input, frames);
                let mut out = game.observe();
                if game.status() != Status::Playing {
                    out.push_str("(ゲーム終了。reset で最初からやり直せます)\n");
                }
                out
            }
            Cmd::Observe => game.observe(),
            Cmd::Reset => {
                game = Game::with_level(&level_src);
                game.observe()
            }
            Cmd::Load(src) => match Game::try_with_level(&src) {
                Ok(g) => {
                    game = g;
                    level_src = src;
                    game.observe()
                }
                Err(e) => format!("error: ステージを読み込めませんでした: {e}"),
            },
            // 今のゲーム状態は触らず、ステージを最初から解けるか探す
            Cmd::Solve(max_seconds) => {
                let cfg = SolveConfig {
                    max_seconds,
                    ..Default::default()
                };
                match solver::solve(&level_src, &cfg) {
                    Ok(outcome) => solver::format_outcome(&outcome),
                    Err(e) => format!("error: {e}"),
                }
            }
        };
        let _ = req.reply.send(out);
    }
}

// ------------------------------------------------------------ MCP サーバー

#[derive(Clone)]
struct SiBros {
    tx: mpsc::Sender<Req>,
    tool_router: ToolRouter<Self>,
}

impl SiBros {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || game_thread(rx));
        Self {
            tx,
            tool_router: Self::tool_router(),
        }
    }

    fn ask(&self, cmd: Cmd) -> String {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(Req { cmd, reply }).is_err() {
            return "error: ゲームスレッドが止まっています".into();
        }
        rx.recv()
            .unwrap_or_else(|_| "error: ゲームスレッドから応答がありません".into())
    }
}

#[tool_router]
impl SiBros {
    #[tool(
        description = "入力を押したまま指定フレーム進め、その後の状態を返す。left/right/jump を指定。\
                       すでに dead / cleared のときは進まず、reset を促すメッセージを返す。"
    )]
    fn step(&self, Parameters(a): Parameters<StepArgs>) -> String {
        let frames = a.frames.unwrap_or(10).clamp(1, MAX_FRAMES);
        self.ask(Cmd::Step(
            PadInput {
                left: a.left,
                right: a.right,
                jump: a.jump,
            },
            frames,
        ))
    }

    #[tool(
        description = "時間を進めずに現在の状態（ステータス・座標・ボス・グリッド・前回からのイベント）を返す。"
    )]
    fn observe(&self) -> String {
        self.ask(Cmd::Observe)
    }

    #[tool(description = "現在のステージを最初からやり直す。")]
    fn reset(&self) -> String {
        self.ask(Cmd::Reset)
    }

    #[tool(
        description = "ステージをテキストで差し替える（ステージエディット用）。\
                       検証に通らなければ理由を返し、今のステージはそのまま。"
    )]
    fn load_level(&self, Parameters(a): Parameters<LoadLevelArgs>) -> String {
        self.ask(Cmd::Load(a.level))
    }

    #[tool(
        description = "今のステージが最初からクリアできるかを探索する（ゲームの進行状態は変えない）。\
                       solved なら入力列を返す。unreachable は「最も細かい粒度(1フレーム刻み)・3600tick以内で探索しきっても\
                       見つからなかった」という意味で、絶対に不可能という証明ではない。\
                       gave_up は時間切れで、クリア可否は不明。"
    )]
    fn solve(&self, Parameters(a): Parameters<SolveArgs>) -> String {
        let secs = a.max_seconds.unwrap_or(10.0).clamp(1.0, 60.0);
        self.ask(Cmd::Solve(secs))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SiBros {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Super Intelligence Bros は横スクロール風アクション。あなたはロボット「エスアイくん」(@)。\
             step で入力を押したまま進め、返ってくるテキストを読んで次の手を決める。\
             グリッド: `#`地面 `P`土管 `S`Sキノコ `B`スーパー部長 `G`ゴール `@`自分。座標はタイル単位で y は下向きが正。\
             ボスは上から踏むとHPが減る（3回で倒せる）。横から当たると、SIならAIに戻り、AIならやられる。\
             クリア条件: ステージに G があれば G に触れる。G がなければボスを倒す。\
             Sキノコは看板が SI になるだけで強さは変わらない。\
             ステージは load_level で書き換えられ、solve でクリア可能かを確かめられる。",
        )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SiBros::new()
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
